//! AS-confirmed immutable references have separate quota and bounded history.
//! Uncertain writers retain tombstones; only independently verified history expires.
use super::*;
use dunlin_mcp_client::oauth::persistence::{CleanupIntent, CleanupJournal, CleanupSummary};

pub(in crate::store) const MIGRATION: &str = "ALTER TABLE mcp_oauth_connections ADD COLUMN cleanup TEXT CHECK(cleanup IS NULL OR (length(cleanup)<=262144 AND json_valid(cleanup)));";
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DeleteState {
    Pending,
    Unknown,
    Acknowledged,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    reference: String,
    credential_revision: u64,
    binding: String,
    generation: u64,
    sha256: String,
    state: DeleteState,
    attempted_revision: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sweep {
    id: String,
    revision: u64,
    connection_revision: u64,
    finished: bool,
    targets: Vec<Target>,
    #[serde(default)]
    reclaimed: usize,
    #[serde(default)]
    pruned: usize,
}
fn sweep(value: &str) -> Result<Sweep, PersistenceError> {
    if value.len() > 262144 {
        return Err(PersistenceError::Storage);
    }
    let sweep: Sweep = serde_json::from_str(value).map_err(|_| PersistenceError::Storage)?;
    if sweep.revision == 0
        || sweep.revision > 9_007_199_254_740_991
        || sweep.targets.is_empty()
        || sweep.targets.len() > 256
        || sweep.reclaimed > 256
        || sweep.pruned > retention::MAX_REFERENCES
    {
        return Err(PersistenceError::Storage);
    }
    Ok(sweep)
}
impl Sweep {
    fn references(&self) -> Vec<String> {
        self.targets.iter().map(|t| t.reference.clone()).collect()
    }
    fn summary(&self) -> CleanupSummary {
        let acknowledged = self
            .targets
            .iter()
            .filter(|t| t.state == DeleteState::Acknowledged)
            .count();
        CleanupSummary {
            revision: self.revision,
            connection_revision: self.connection_revision,
            delete_acknowledged: acknowledged,
            unconfirmed: self.targets.len() - acknowledged,
            reclaimed: self.reclaimed,
            pruned: self.pruned,
        }
    }
}
#[derive(Serialize)]
pub(crate) struct CleanupReceipt {
    pub revision: u64,
    pub connection_revision: u64,
    pub status: &'static str,
    pub delete_acknowledged: usize,
    pub unconfirmed: usize,
    pub reclaimed: usize,
    pub pruned: usize,
}
pub(in crate::store::mcp_oauth) fn receipt(
    value: &str,
) -> Result<CleanupReceipt, PersistenceError> {
    let sweep = sweep(value)?;
    let summary = sweep.summary();
    Ok(CleanupReceipt {
        revision: summary.revision,
        connection_revision: summary.connection_revision,
        status: if !sweep.finished {
            "unfinished"
        } else if summary.unconfirmed == 0 {
            "sweep_complete"
        } else {
            "unconfirmed"
        },
        delete_acknowledged: summary.delete_acknowledged,
        unconfirmed: summary.unconfirmed,
        reclaimed: summary.reclaimed,
        pruned: summary.pruned,
    })
}
fn save_sweep(db: &Connection, c: &AuthContext, s: &Sweep) -> Result<(), PersistenceError> {
    let value = serde_json::to_string(s).map_err(|_| PersistenceError::Storage)?;
    if value.len() > 262144 {
        return Err(PersistenceError::Limit);
    }
    db.execute(
        "UPDATE mcp_oauth_connections SET cleanup=?2 WHERE connection_id=?1",
        params![c.connection_id, value],
    )
    .map_err(storage)?;
    Ok(())
}
pub(in crate::store::mcp_oauth) fn revision(
    db: &Connection,
    id: &str,
) -> Result<u64, PersistenceError> {
    let value: Option<String> = db
        .query_row(
            "SELECT cleanup FROM mcp_oauth_connections WHERE connection_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?
        .flatten();
    value
        .as_deref()
        .map(sweep)
        .transpose()
        .map(|s| s.map_or(0, |s| s.revision))
}
fn pointers(
    db: &Connection,
    c: &AuthContext,
) -> Result<(String, Option<String>, Option<String>), PersistenceError> {
    db.query_row("SELECT state,secret_reference,staged_reference FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4",
        params![c.connection_id,c.workspace_id,c.user_id,c.connection_revision as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)
}
fn protected(p: &(String, Option<String>, Option<String>), reference: &str) -> bool {
    p.0 != "disabled" && (p.1.as_deref() == Some(reference) || p.2.as_deref() == Some(reference))
}
fn eligible(db: &Connection, c: &AuthContext) -> Result<Vec<Target>, PersistenceError> {
    let p = pointers(db, c)?;
    let mut query=db.prepare("SELECT reference,credential_revision,binding,generation,sha256,cleanup_attempted_revision FROM mcp_oauth_secret_references WHERE connection_id=?1 AND as_confirmed=1 AND credential_revision IS NOT NULL AND binding IS NOT NULL AND generation IS NOT NULL AND sha256 IS NOT NULL ORDER BY cleanup_attempted_revision,reference LIMIT 1281").map_err(storage)?;
    let rows = query
        .query_map([&c.connection_id], |r| {
            Ok(Target {
                reference: r.get(0)?,
                credential_revision: super::super::super::positive_revision(r, 1)?,
                binding: r.get(2)?,
                generation: super::super::super::positive_revision(r, 3)?,
                sha256: r.get(4)?,
                state: DeleteState::Pending,
                attempted_revision: r.get::<_, i64>(5)? as u64,
            })
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    if rows.len() > retention::MAX_REFERENCES {
        return Err(PersistenceError::Limit);
    }
    Ok(rows
        .into_iter()
        .filter(|t| !protected(&p, &t.reference) && hex_digest(&t.binding) && hex_digest(&t.sha256))
        .collect())
}
fn load(db: &Connection, c: &AuthContext, i: &CleanupIntent) -> Result<Sweep, PersistenceError> {
    let value:String=db.query_row("SELECT cleanup FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4",
        params![c.connection_id,c.workspace_id,c.user_id,c.connection_revision as i64],|r|r.get(0)).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
    let sweep = sweep(&value)?;
    if sweep.finished
        || sweep.id != i.operation_id
        || sweep.revision != i.revision
        || sweep.connection_revision != i.connection_revision
        || c.connection_revision != i.connection_revision
        || sweep.references() != i.references
    {
        return Err(PersistenceError::Conflict);
    }
    Ok(sweep)
}
fn target_valid(
    db: &Connection,
    c: &AuthContext,
    s: &Sweep,
    n: usize,
) -> Result<(), PersistenceError> {
    let t = s.targets.get(n).ok_or(PersistenceError::Conflict)?;
    if t.state != DeleteState::Unknown || protected(&pointers(db, c)?, &t.reference) {
        return Err(PersistenceError::Conflict);
    }
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM mcp_oauth_secret_references WHERE connection_id=?1 AND reference=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6 AND as_confirmed=1)",
        params![c.connection_id,t.reference,t.credential_revision as i64,t.binding,t.generation as i64,t.sha256],|r|r.get(0)).map_err(storage)?;
    if !valid {
        return Err(PersistenceError::Conflict);
    }
    Ok(())
}
impl CleanupJournal for CheckedJournal<'_> {
    fn begin_cleanup(
        &self,
        c: &AuthContext,
        expected: u64,
    ) -> Result<CleanupIntent, PersistenceError> {
        if expected >= 9_007_199_254_740_991 {
            return Err(PersistenceError::Limit);
        }
        let tx = self.transaction(c)?;
        let old:Option<String>=tx.query_row("SELECT cleanup FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4",
            params![c.connection_id,c.workspace_id,c.user_id,c.connection_revision as i64],|r|r.get(0)).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
        let previous = old.as_deref().map(sweep).transpose()?;
        if previous.as_ref().map_or(0, |s| s.revision) != expected {
            return Err(PersistenceError::Conflict);
        }
        let legacy: Option<String> = tx
            .query_row(
                "SELECT revocation FROM mcp_oauth_connections WHERE connection_id=?1",
                [&c.connection_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if let Some(value) = legacy {
            remember_confirmations(&tx, &c.connection_id, &decode(&value)?)?;
        }
        if let Some(previous) = previous {
            for t in previous.targets {
                tx.execute("UPDATE mcp_oauth_secret_references SET cleanup_attempted_revision=max(cleanup_attempted_revision,?7) WHERE connection_id=?1 AND reference=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6",
                    params![c.connection_id,t.reference,t.credential_revision as i64,t.binding,t.generation as i64,t.sha256,t.attempted_revision as i64]).map_err(storage)?;
            }
        }
        let pruned = retention::prune(
            &tx,
            c,
            self.oauth_writer_owner
                .load(std::sync::atomic::Ordering::Acquire),
        )?;
        let mut targets = eligible(&tx, c)?;
        if targets.is_empty() {
            return Err(PersistenceError::Conflict);
        }
        targets.sort_by(|a, b| {
            a.attempted_revision
                .cmp(&b.attempted_revision)
                .then_with(|| a.reference.cmp(&b.reference))
        });
        targets.truncate(256);
        // Superseding an unfinished delete is safe: the old nonce is fenced,
        // and any already-started delete can touch only its retired unique UUID.
        let s = Sweep {
            id: uuid::Uuid::now_v7().to_string(),
            revision: expected + 1,
            connection_revision: c.connection_revision,
            finished: false,
            targets,
            reclaimed: 0,
            pruned,
        };
        save_sweep(&tx, c, &s)?;
        match self.team_audit_capacity() {
            Ok(()) => {
                tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,'mcp_oauth_cleanup_started',?3,?4,?5)",params![c.workspace_id,c.user_id,s.revision as i64,crate::store::now_secs(),c.connection_id]).map_err(storage)?;
            }
            Err(crate::store::StoreError::Limit(_)) => {}
            Err(_) => return Err(PersistenceError::Storage),
        }
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(CleanupIntent {
            operation_id: s.id.clone(),
            revision: s.revision,
            connection_revision: s.connection_revision,
            references: s.references(),
        })
    }
    fn claim_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(c)?;
        let mut s = load(&tx, c, i)?;
        let revision = s.revision;
        let target = s.targets.get_mut(n).ok_or(PersistenceError::Conflict)?;
        if target.state != DeleteState::Pending {
            return Err(PersistenceError::Conflict);
        }
        target.state = DeleteState::Unknown;
        target.attempted_revision = revision;
        target_valid(&tx, c, &s, n)?;
        tx.execute("UPDATE mcp_oauth_secret_references SET cleanup_attempted_revision=?2 WHERE reference=?1",params![s.targets[n].reference,s.revision as i64]).map_err(storage)?;
        save_sweep(&tx, c, &s)?;
        tx.commit().map_err(storage)
    }
    fn check_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(c)?;
        let s = load(&tx, c, i)?;
        target_valid(&tx, c, &s, n)?;
        tx.commit().map_err(storage)
    }
    fn acknowledge_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(c)?;
        let mut s = load(&tx, c, i)?;
        target_valid(&tx, c, &s, n)?;
        let reference = &s.targets[n].reference;
        let (retired,previous_verified,proof):(bool,bool,bool)=tx.query_row("SELECT retired,delete_verified,(writer_protected=1 AND write_claimed=1 AND write_acknowledged=1 AND write_operation_id IS NOT NULL) FROM mcp_oauth_secret_references WHERE reference=?1",[reference],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage)?;
        let verified = proof
            && self
                .oauth_writer_owner
                .load(std::sync::atomic::Ordering::Acquire);
        let uncertain:u32=tx.query_row("SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE connection_id=?1 AND retired=1 AND delete_verified=0",[&c.connection_id],|r|r.get(0)).map_err(storage)?;
        if retired || verified || uncertain < retention::UNCERTAIN_HISTORY as u32 {
            tx.execute("UPDATE mcp_oauth_secret_references SET retired=1,delete_verified=?2 WHERE reference=?1",params![reference,previous_verified||verified]).map_err(storage)?;
            if !retired {
                s.reclaimed += 1;
            }
        }
        s.targets[n].state = DeleteState::Acknowledged;
        save_sweep(&tx, c, &s)?;
        tx.commit().map_err(storage)
    }
    fn finish_cleanup(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
    ) -> Result<CleanupSummary, PersistenceError> {
        let tx = self.transaction(c)?;
        let mut s = load(&tx, c, i)?;
        s.pruned += retention::prune(
            &tx,
            c,
            self.oauth_writer_owner
                .load(std::sync::atomic::Ordering::Acquire),
        )?;
        s.finished = true;
        let summary = s.summary();
        save_sweep(&tx, c, &s)?;
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(summary)
    }
}
impl CleanupJournal for Store {
    fn begin_cleanup(&self, c: &AuthContext, r: u64) -> Result<CleanupIntent, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .begin_cleanup(c, r)
    }
    fn claim_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .claim_cleanup_target(c, i, n)
    }
    fn check_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .check_cleanup_target(c, i, n)
    }
    fn acknowledge_cleanup_target(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .acknowledge_cleanup_target(c, i, n)
    }
    fn finish_cleanup(
        &self,
        c: &AuthContext,
        i: &CleanupIntent,
    ) -> Result<CleanupSummary, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .finish_cleanup(c, i)
    }
}
