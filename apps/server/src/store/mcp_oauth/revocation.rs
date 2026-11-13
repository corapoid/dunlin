//! Durable negative operation, separate from the positive credential journal.
//! A crash leaves the operation blocked/unknown; no startup path replays HTTP.
use super::*;
pub(super) mod cleanup;
mod lease;
mod legacy;
mod reconcile;
use dunlin_mcp_client::oauth::persistence::{
    RevocationIntent, RevocationJournal, RevocationSummary, RevocationTarget,
};
pub(crate) use lease::RevocationLease;
pub(crate) use lease::WriterLease;
pub(crate) use legacy::LegacyReview;
use serde::{Deserialize, Serialize};

pub(in crate::store) const MIGRATION: &str = "ALTER TABLE mcp_oauth_connections ADD COLUMN revocation TEXT CHECK(revocation IS NULL OR (length(revocation)<=262144 AND json_valid(revocation)));";
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobState {
    Running,
    Confirmed,
    Unconfirmed,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TargetState {
    Pending,
    Unknown,
    Confirmed,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    reference: String,
    revision: Option<u64>,
    binding: Option<String>,
    generation: Option<u64>,
    sha256: Option<String>,
    state: TargetState,
    #[serde(default)]
    prior_confirmed: bool,
    #[serde(default)]
    attempted_revision: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    id: String,
    source_revision: u64,
    disabled_revision: u64,
    binding: String,
    state: JobState,
    // Older binaries did not hold the kernel lease. Its absence cannot prove
    // that a pre-upgrade worker has stopped, so recovery must fail closed.
    #[serde(default)]
    lease_protected: bool,
    targets: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operator_recovery: Option<legacy::Recovery>,
}
fn decode(value: &str) -> Result<Ledger, PersistenceError> {
    if value.len() > 262144 {
        return Err(PersistenceError::Storage);
    }
    let ledger: Ledger = serde_json::from_str(value).map_err(|_| PersistenceError::Storage)?;
    if ledger.targets.len() > 256 || !hex_digest(&ledger.binding) {
        return Err(PersistenceError::Storage);
    }
    Ok(ledger)
}
// Import only exact acknowledged envelopes from pre47 journals. A missing row
// may have been safely pruned; it is never recreated from a receipt.
fn remember_confirmations(
    db: &Connection,
    id: &str,
    ledger: &Ledger,
) -> Result<(), PersistenceError> {
    for e in &ledger.targets {
        if e.state != TargetState::Confirmed {
            continue;
        }
        let (Some(revision), Some(binding), Some(generation), Some(hash)) = (
            e.revision,
            e.binding.as_ref(),
            e.generation,
            e.sha256.as_ref(),
        ) else {
            continue;
        };
        if revision == 0
            || generation == 0
            || revision > 9_007_199_254_740_991
            || generation > 9_007_199_254_740_991
            || !hex_digest(binding)
            || !hex_digest(hash)
        {
            continue;
        }
        db.execute("UPDATE mcp_oauth_secret_references SET as_confirmed=1 WHERE connection_id=?1 AND reference=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6 AND as_confirmed=0",
            params![id,e.reference,revision as i64,binding,generation as i64,hash]).map_err(storage)?;
    }
    Ok(())
}
pub(super) fn positive_allowed(db: &Connection, connection: &str) -> Result<(), PersistenceError> {
    let value: Option<Option<String>> = db
        .query_row(
            "SELECT revocation FROM mcp_oauth_connections WHERE connection_id=?1",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    if let Some(value) = value.flatten()
        && decode(&value)?.state == JobState::Running
    {
        return Err(PersistenceError::Conflict);
    }
    Ok(())
}
impl Ledger {
    fn targets(&self, context: &AuthContext) -> Vec<Option<RevocationTarget>> {
        self.targets
            .iter()
            .map(|entry| {
                let revision = entry.revision?;
                let generation = entry.generation?;
                let hash = entry.sha256.as_ref()?;
                if entry.binding.as_deref() != Some(self.binding.as_str())
                    || !hex_digest(hash)
                    || revision == 0
                    || revision > 9_007_199_254_740_991
                    || generation == 0
                    || generation > 9_007_199_254_740_991
                {
                    return None;
                }
                Some(RevocationTarget {
                    context: AuthContext {
                        connection_revision: revision,
                        ..context.clone()
                    },
                    record: CredentialRecord {
                        generation,
                        secret_reference: entry.reference.clone(),
                        sha256: hash.clone(),
                    },
                })
            })
            .collect()
    }
    fn summary(&self) -> RevocationSummary {
        let confirmed = self
            .targets
            .iter()
            .filter(|e| e.state == TargetState::Confirmed)
            .count();
        RevocationSummary {
            connection_revision: self.disabled_revision,
            confirmed,
            unconfirmed: self.targets.len() - confirmed,
            confirmed_known_tokens: !self.targets.is_empty() && self.targets.len() == confirmed,
        }
    }
}
fn load(
    db: &Connection,
    context: &AuthContext,
    intent: &RevocationIntent,
) -> Result<Ledger, PersistenceError> {
    let value:String=db.query_row("SELECT revocation FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND state='disabled'",
        params![context.connection_id,context.workspace_id,context.user_id,intent.connection_revision as i64],|r|r.get(0)).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
    let ledger = decode(&value)?;
    if ledger.state != JobState::Running
        || ledger.id != intent.operation_id
        || ledger.binding != intent.binding
        || ledger.disabled_revision != intent.connection_revision
        || ledger.source_revision != context.connection_revision
        || ledger.targets(context) != intent.targets
        || ledger
            .targets
            .iter()
            .map(|entry| entry.prior_confirmed)
            .collect::<Vec<_>>()
            != intent.already_confirmed
    {
        return Err(PersistenceError::Conflict);
    }
    Ok(ledger)
}
fn save(db: &Connection, context: &AuthContext, ledger: &Ledger) -> Result<(), PersistenceError> {
    let value = serde_json::to_string(ledger).map_err(|_| PersistenceError::Storage)?;
    if value.len() > 262144 {
        return Err(PersistenceError::Limit);
    }
    db.execute(
        "UPDATE mcp_oauth_connections SET revocation=?2 WHERE connection_id=?1",
        params![context.connection_id, value],
    )
    .map_err(storage)?;
    Ok(())
}
impl RevocationJournal for CheckedJournal<'_> {
    fn begin_revocation(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<RevocationIntent, PersistenceError> {
        if !hex_digest(binding) {
            return Err(PersistenceError::Unauthorized);
        }
        if context.connection_revision >= 9_007_199_254_740_991 {
            return Err(PersistenceError::Limit);
        }
        let tx = self.transaction(context)?;
        positive_allowed(&tx, &context.connection_id)?;
        let (state,previous):(String,Option<String>)=tx.query_row("SELECT state,revocation FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND binding=?5",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64,binding],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
        let previous = previous.as_deref().map(decode).transpose()?;
        if previous.as_ref().is_some_and(|old| {
            old.state == JobState::Confirmed && old.disabled_revision == context.connection_revision
        }) && state == "disabled"
        {
            return Err(PersistenceError::Conflict);
        }
        if let Some(old) = previous.as_ref() {
            remember_confirmations(&tx, &context.connection_id, old)?;
        }
        let mut targets = {
            let mut query=tx.prepare("SELECT reference,credential_revision,binding,generation,sha256,as_confirmed FROM mcp_oauth_secret_references WHERE connection_id=?1 AND retired=0 ORDER BY reference LIMIT 257").map_err(storage)?;
            query
                .query_map([&context.connection_id], |r| {
                    Ok(Entry {
                        reference: r.get(0)?,
                        revision: r
                            .get::<_, Option<i64>>(1)?
                            .and_then(|n| u64::try_from(n).ok()),
                        binding: r.get(2)?,
                        generation: r
                            .get::<_, Option<i64>>(3)?
                            .and_then(|n| u64::try_from(n).ok()),
                        sha256: r.get(4)?,
                        state: if r.get::<_, bool>(5)? {
                            TargetState::Confirmed
                        } else {
                            TargetState::Pending
                        },
                        prior_confirmed: r.get(5)?,
                        attempted_revision: 0,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?
        };
        if targets.len() > 256 {
            return Err(PersistenceError::Limit);
        }
        let operator_recovery = previous.as_ref().and_then(|p| p.operator_recovery.clone());
        if let Some(previous) = previous {
            for entry in &mut targets {
                if let Some(old) = previous.targets.iter().find(|old| {
                    old.reference == entry.reference
                        && old.revision == entry.revision
                        && old.binding == entry.binding
                        && old.generation == entry.generation
                        && old.sha256 == entry.sha256
                }) {
                    entry.attempted_revision = old.attempted_revision;
                    if old.state == TargetState::Confirmed {
                        entry.prior_confirmed = true;
                        entry.state = TargetState::Confirmed;
                    }
                }
            }
        }
        // Persist least-recently-attempted ordering across explicit retries. A
        // slow/broken old token cannot starve later, still-unattempted records.
        targets.sort_by(|a, b| {
            a.prior_confirmed
                .cmp(&b.prior_confirmed)
                .then_with(|| a.attempted_revision.cmp(&b.attempted_revision))
                .then_with(|| a.reference.cmp(&b.reference))
        });
        let ledger = Ledger {
            id: uuid::Uuid::now_v7().to_string(),
            source_revision: context.connection_revision,
            disabled_revision: context.connection_revision + 1,
            binding: binding.into(),
            state: JobState::Running,
            lease_protected: self
                .store
                .oauth_revocation_owner
                .load(std::sync::atomic::Ordering::Acquire),
            targets,
            operator_recovery,
        };
        tx.execute("UPDATE mcp_oauth_connections SET state='disabled',revision=?2,operation_id=NULL,staged_reference=NULL WHERE connection_id=?1",
            params![context.connection_id,ledger.disabled_revision as i64]).map_err(storage)?;
        save(&tx, context, &ledger)?;
        // A full shared audit must neither grow without bound on retries nor
        // prevent this negative action. Its bounded private receipt is durable.
        match self.team_audit_capacity() {
            Ok(()) => {
                tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,'mcp_oauth_revocation_started',?3,?4,?5)",
                params![context.workspace_id,context.user_id,ledger.disabled_revision as i64,super::super::now_secs(),context.connection_id]).map_err(storage)?;
            }
            Err(super::super::StoreError::Limit(_)) => {}
            Err(_) => return Err(PersistenceError::Storage),
        }
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(RevocationIntent {
            operation_id: ledger.id.clone(),
            connection_revision: ledger.disabled_revision,
            binding: ledger.binding.clone(),
            targets: ledger.targets(context),
            already_confirmed: ledger
                .targets
                .iter()
                .map(|entry| entry.prior_confirmed)
                .collect(),
        })
    }
    fn claim_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(context)?;
        let mut ledger = load(&tx, context, intent)?;
        let attempted_revision = ledger.disabled_revision;
        let entry = ledger
            .targets
            .get_mut(index)
            .ok_or(PersistenceError::Conflict)?;
        if entry.state != TargetState::Pending {
            return Err(PersistenceError::Conflict);
        }
        entry.state = TargetState::Unknown;
        entry.attempted_revision = attempted_revision;
        save(&tx, context, &ledger)?;
        tx.commit().map_err(storage)
    }
    fn check_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(context)?;
        let ledger = load(&tx, context, intent)?;
        if ledger
            .targets
            .get(index)
            .is_none_or(|e| e.state != TargetState::Unknown)
        {
            return Err(PersistenceError::Conflict);
        }
        tx.commit().map_err(storage)
    }
    fn confirm_revocation_target(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
        index: usize,
    ) -> Result<(), PersistenceError> {
        let tx = self.transaction(context)?;
        let mut ledger = load(&tx, context, intent)?;
        let entry = ledger
            .targets
            .get_mut(index)
            .ok_or(PersistenceError::Conflict)?;
        if entry.state != TargetState::Unknown
            || intent.targets.get(index).is_none_or(Option::is_none)
        {
            return Err(PersistenceError::Conflict);
        }
        entry.state = TargetState::Confirmed;
        let target = intent.targets[index]
            .as_ref()
            .ok_or(PersistenceError::Conflict)?;
        let changed=tx.execute("UPDATE mcp_oauth_secret_references SET as_confirmed=1 WHERE connection_id=?1 AND reference=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6",
            params![context.connection_id,target.record.secret_reference,target.context.connection_revision as i64,entry.binding,target.record.generation as i64,target.record.sha256]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        save(&tx, context, &ledger)?;
        tx.commit().map_err(storage)
    }
    fn finish_revocation(
        &self,
        context: &AuthContext,
        intent: &RevocationIntent,
    ) -> Result<RevocationSummary, PersistenceError> {
        let tx = self.transaction(context)?;
        let mut ledger = load(&tx, context, intent)?;
        let summary = ledger.summary();
        ledger.state = if summary.confirmed_known_tokens {
            JobState::Confirmed
        } else {
            JobState::Unconfirmed
        };
        save(&tx, context, &ledger)?;
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(summary)
    }
}
impl RevocationJournal for Store {
    fn begin_revocation(
        &self,
        c: &AuthContext,
        b: &str,
    ) -> Result<RevocationIntent, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .begin_revocation(c, b)
    }
    fn claim_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .claim_revocation_target(c, i, n)
    }
    fn check_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .check_revocation_target(c, i, n)
    }
    fn confirm_revocation_target(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
        n: usize,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .confirm_revocation_target(c, i, n)
    }
    fn finish_revocation(
        &self,
        c: &AuthContext,
        i: &RevocationIntent,
    ) -> Result<RevocationSummary, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .finish_revocation(c, i)
    }
}

#[derive(Serialize)]
pub(crate) struct RevocationReceipt {
    pub state: &'static str,
    pub revision: u64,
    pub confirmed: usize,
    pub unconfirmed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconciliation_supported: Option<bool>,
}
pub(super) fn receipt(value: &str) -> Result<RevocationReceipt, PersistenceError> {
    let ledger = decode(value)?;
    let summary = ledger.summary();
    Ok(RevocationReceipt {
        reconciliation_supported: if ledger.state == JobState::Running {
            Some(ledger.lease_protected)
        } else {
            None
        },
        state: match ledger.state {
            JobState::Running => "unfinished",
            JobState::Confirmed => "confirmed_known_tokens",
            JobState::Unconfirmed => "unconfirmed",
        },
        revision: summary.connection_revision,
        confirmed: summary.confirmed,
        unconfirmed: summary.unconfirmed,
    })
}
impl Store {
    pub(crate) fn mcp_oauth_credential_binding(
        &self,
        context: &AuthContext,
    ) -> Result<String, PersistenceError> {
        authorize(&self.connection, context)?;
        self.connection.query_row("SELECT binding FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64],|r|r.get(0)).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)
    }
}
