use super::*;
use dunlin_domain::jobs::{Enqueue, Job, Queue, Status};
use dunlin_mcp_client::linear::Issue;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Task {
    pub request: Enqueue,
    pub issue: Issue,
    pub root: String,
    pub head: String,
    pub settings_digest: String,
    pub device: String,
    pub linear_revision: String,
    #[serde(default)]
    pub native_grant: Option<NativeGrantStamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_checkout_stamp: Option<super::native_checkout::NativeCheckoutStamp>,
}
#[derive(Clone)]
pub(crate) struct Record {
    pub job: Job,
    pub task: Task,
    pub checkout_grant: Option<NativeGrantStamp>,
    pub checkout_stamp: Option<super::native_checkout::NativeCheckoutStamp>,
}

fn status(value: &str) -> rusqlite::Result<Status> {
    serde_json::from_value(serde_json::Value::String(value.into()))
        .map_err(|_| rusqlite::Error::InvalidQuery)
}
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    let payload: String = row.get(4)?;
    if payload.len() > 96 * 1024 {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let task: Task = serde_json::from_str(&payload).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(Record {
        job: Job {
            id: row.get(0)?,
            owner: row.get(1)?,
            owner_name: row.get(2)?,
            project_id: row.get(3)?,
            issue_id: task.issue.id.clone(),
            identifier: task.issue.identifier.clone(),
            title: task.issue.title.clone(),
            profile: task.request.profile.clone(),
            status: status(&row.get::<_, String>(5)?)?,
            cancel_requested: row.get(6)?,
            report: row.get(7)?,
            checkout_project: row.get(8)?,
            branch: row.get(9)?,
            sync_state: row.get(10)?,
            sync_error: row.get(11)?,
        },
        task,
        checkout_stamp: row
            .get::<_, Option<String>>(13)?
            .map(|raw| {
                if raw.len() > 16 * 1024 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .transpose()?,
        checkout_grant: row
            .get::<_, Option<String>>(12)?
            .map(|raw| serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery))
            .transpose()?,
    })
}
const SELECT: &str = "SELECT j.id,j.owner,u.name,j.project,j.task,j.status,j.cancel,j.report,j.checkout_project,j.branch,j.sync_state,j.sync_error,j.checkout_grant,j.checkout_stamp FROM relay_jobs j JOIN users u ON u.id=j.owner";

impl Store {
    pub(crate) fn job_workload(&self, user: &str) -> Result<dunlin_domain::presence::Workload> {
        let (queued,active,interrupted)=self.connection.query_row("SELECT coalesce(sum(j.status='queued'),0),coalesce(sum(j.status IN ('preparing','running')),0),coalesce(sum(j.status='unknown'),0) FROM relay_jobs j WHERE EXISTS(SELECT 1 FROM project_grants g WHERE g.project_id=j.project AND g.user_id=?1 AND g.active=1)",[user],|row|Ok((row.get::<_,u32>(0)? as usize,row.get::<_,u32>(1)? as usize,row.get::<_,u32>(2)? as usize)))?;
        let paused = self.connection.query_row(
            "SELECT paused FROM relay_queue_state WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        Ok(dunlin_domain::presence::Workload {
            queued,
            active,
            interrupted,
            paused,
        })
    }
    pub(crate) fn job(&self, id: &str) -> Result<Option<Record>> {
        Ok(self
            .connection
            .query_row(&format!("{SELECT} WHERE j.id=?1"), [id], row)
            .optional()?)
    }
    pub(crate) fn jobs(&self) -> Result<Queue> {
        let paused = self.connection.query_row(
            "SELECT paused FROM relay_queue_state WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let mut statement = self
            .connection
            .prepare(&format!("{SELECT} ORDER BY j.position,j.seq"))?;
        let jobs = statement
            .query_map([], row)?
            .map(|row| row.map(|record| record.job))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Queue {
            paused,
            jobs,
            self_user_id: None,
        })
    }
    #[cfg(test)]
    pub(crate) fn enqueue(&self, user: &str, task: &Task) -> Result<Job> {
        self.native_checkout_transaction(|store| store.enqueue_in_transaction(user, task))
    }
    pub(crate) fn enqueue_in_transaction(&self, user: &str, task: &Task) -> Result<Job> {
        let payload =
            serde_json::to_string(task).map_err(|_| StoreError::Limit("Invalid task".into()))?;
        if payload.len() > 96 * 1024 {
            return Err(StoreError::Limit("Task snapshot is too large".into()));
        }
        let fingerprint = crate::auth::hash_token(&payload);
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "Job approval transaction required".into(),
            ));
        }
        let transaction = &self.connection;
        if let Some((id, previous)) = transaction
            .query_row(
                "SELECT job,fingerprint FROM relay_job_approvals WHERE owner=?1 AND idem=?2",
                params![user, task.request.idempotency_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != fingerprint {
                return Err(StoreError::Limit(
                    "Idempotency key was already used for a different task approval".into(),
                ));
            }
            return Ok(self
                .job(&id)?
                .ok_or_else(|| {
                    StoreError::Limit(
                        "This approval was already archived; it cannot be replayed".into(),
                    )
                })?
                .job);
        }
        let count: i64 =
            transaction.query_row("SELECT count(*) FROM relay_jobs", [], |row| row.get(0))?;
        if count >= 512 {
            return Err(StoreError::Limit(
                "Queue history is full; archive finished jobs first".into(),
            ));
        }
        let exists:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM relay_jobs WHERE issue=?1 AND status IN ('queued','preparing','running','unknown'))",[&task.issue.id],|row|row.get(0))?;
        if exists {
            return Err(StoreError::Limit(
                "This issue already has an active or interrupted job on this host".into(),
            ));
        }
        let position: i64 = transaction.query_row(
            "SELECT coalesce(max(position),0)+1 FROM relay_jobs",
            [],
            |row| row.get(0),
        )?;
        let id = uuid::Uuid::now_v7().to_string();
        let approvals: i64 =
            transaction.query_row("SELECT count(*) FROM relay_job_approvals", [], |row| {
                row.get(0)
            })?;
        if approvals >= 100_000 {
            return Err(StoreError::Limit(
                "Approval ledger is full; administrator maintenance is required".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO relay_job_approvals(owner,idem,fingerprint,job) VALUES (?1,?2,?3,?4)",
            params![user, task.request.idempotency_key, fingerprint, id],
        )?;
        transaction.execute("INSERT INTO relay_jobs(id,owner,project,issue,idem,fingerprint,task,position) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![id,user,task.request.project_id,task.issue.id,task.request.idempotency_key,fingerprint,payload,position])?;
        Ok(self.job(&id)?.unwrap().job)
    }
    pub(crate) fn claim_job(&self) -> Result<Option<Record>> {
        let transaction = self.connection.unchecked_transaction()?;
        let blocked:bool=transaction.query_row("SELECT (SELECT paused FROM relay_queue_state WHERE id=1) OR EXISTS(SELECT 1 FROM relay_jobs WHERE status IN ('preparing','running','unknown'))",[],|row|row.get(0))?;
        if blocked {
            return Ok(None);
        }
        let id: Option<String> = transaction
            .query_row(
                "SELECT id FROM relay_jobs WHERE status='queued' ORDER BY position,seq LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = &id {
            transaction.execute(
                "UPDATE relay_jobs SET status='preparing' WHERE id=?1 AND status='queued'",
                [id],
            )?;
        }
        transaction.commit()?;
        id.map(|id| self.job(&id)).transpose().map(Option::flatten)
    }
    #[cfg(test)]
    pub(crate) fn job_running(&self, id: &str, project: &str, branch: &str) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        let record = self
            .job(id)?
            .ok_or_else(|| StoreError::Conflict("Job was removed".into()))?;
        if record.job.status != Status::Preparing
            || record.job.cancel_requested
            || record.job.checkout_project.is_some()
            || record.checkout_grant.is_some()
        {
            return Err(StoreError::Conflict(
                "Job checkout reservation is no longer new".into(),
            ));
        }
        let source = record
            .task
            .native_grant
            .as_ref()
            .ok_or_else(|| StoreError::Conflict("Legacy task lacks native authority".into()))?;
        if source.project_id != record.job.project_id || source.user_id != record.job.owner {
            return Err(StoreError::Conflict(
                "Job source grant scope changed".into(),
            ));
        }
        self.revalidate_native_grant(source)?;
        let grant = self.capture_native_grant(project, &record.job.owner)?;
        if project == record.job.project_id {
            return Err(StoreError::Conflict(
                "Job checkout must be independent".into(),
            ));
        }
        let changed=self.connection.execute("UPDATE relay_jobs SET status='running',checkout_project=?2,branch=?3,checkout_grant=?4 WHERE id=?1 AND status='preparing' AND cancel=0 AND checkout_project IS NULL AND checkout_grant IS NULL",params![id,project,branch,serde_json::to_string(&grant)?])?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "Job reservation is no longer active".into(),
            ));
        }
        tx.commit()?;
        Ok(())
    }
    /// Publish the independent clone, grant and execution stamp atomically.
    /// All filesystem inspection has completed before this fresh SQL cut.
    pub(crate) fn publish_job_checkout(
        &self,
        id: &str,
        source: &super::native_checkout::NativeCheckoutAuthority,
        prepared: super::native_checkout::PreparedOrdinaryCheckout,
        branch: &str,
        authorize: impl FnOnce(&Store, &Record) -> Result<()>,
    ) -> Result<Record> {
        let result=self.native_checkout_transaction(|store| {
            let record=store.job(id)?.ok_or_else(||StoreError::Conflict("Job removed".into()))?;
            if record.job.status!=Status::Preparing || record.job.cancel_requested || record.job.checkout_project.is_some() || record.checkout_grant.is_some() || record.checkout_stamp.is_some(){return Err(StoreError::Conflict("Job checkout reservation is no longer new".into()));}
            if record.task.source_checkout_stamp.as_ref()!=Some(&source.stamp()) || source.stamp().native_grant.project_id!=record.job.project_id || source.stamp().native_grant.user_id!=record.job.owner {return Err(StoreError::Conflict("Job source checkout approval changed".into()));}
            store.revalidate_native_checkout_in_transaction(source)?;authorize(store,&record)?;
            let project=Project{id:format!("prj-{}",uuid::Uuid::now_v7()),name:format!("job-{id}"),path:prepared.root().into()};
            store.connection.execute("INSERT INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",params![project.id,project.name,project.path,super::now_secs()])?;
            store.connection.execute("INSERT INTO project_grants(project_id,user_id,revision,active) VALUES(?1,?2,1,1)",params![project.id,record.job.owner])?;
            let checkout=store.authorize_new_native_checkout_in_transaction(&project.id,&record.job.owner,super::local_metadata::MetadataPermission::Write,prepared)?;
            let stamp=checkout.stamp();let grant=stamp.native_grant.clone();
            let changed=store.connection.execute("UPDATE relay_jobs SET status='running',checkout_project=?2,branch=?3,checkout_grant=?4,checkout_stamp=?5 WHERE id=?1 AND status='preparing' AND cancel=0 AND checkout_project IS NULL AND checkout_grant IS NULL AND checkout_stamp IS NULL",params![id,project.id,branch,serde_json::to_string(&grant)?,serde_json::to_string(&stamp)?])?;
            if changed!=1{return Err(StoreError::Conflict("Job reservation changed".into()));}store.job(id)?.ok_or_else(||StoreError::Conflict("Published job missing".into()))
        })?;
        self.catalog_changed();
        Ok(result)
    }
    pub(crate) fn finish_job(&self, id: &str, state: Status, report: &str) -> Result<()> {
        let state = serde_json::to_value(state)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        if !matches!(
            state.as_str(),
            "review" | "failed" | "blocked" | "cancelled"
        ) {
            return Err(StoreError::Limit("Invalid terminal job state".into()));
        }
        if report.len() > 16 * 1024 {
            return Err(StoreError::Limit("Job report is too large".into()));
        }
        self.connection.execute("UPDATE relay_jobs SET status=CASE WHEN cancel=1 THEN 'cancelled' ELSE ?2 END,report=?3,sync_state=CASE WHEN ?2='review' AND cancel=0 THEN 'pending' ELSE 'none' END WHERE id=?1 AND status IN ('preparing','running')",params![id,state,report])?;
        Ok(())
    }
    pub(crate) fn cancel_job(&self, id: &str) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET cancel=1,status=CASE WHEN status='queued' THEN 'cancelled' ELSE status END WHERE id=?1 AND status IN ('queued','preparing','running')",[id])?;
        Ok(())
    }
    pub(crate) fn pause_jobs(&self, paused: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE relay_queue_state SET paused=?1 WHERE id=1",
            [paused],
        )?;
        Ok(())
    }
    pub(crate) fn prioritize_job(&self, id: &str) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET position=(SELECT coalesce(min(position),0)-1 FROM relay_jobs) WHERE id=?1 AND status='queued'",[id])?;
        Ok(())
    }
    pub(crate) fn recover_jobs(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let changed=transaction.execute("UPDATE relay_jobs SET status='unknown',report='Relay restarted during execution. Inspect and stop orphaned processes before reconciliation.' WHERE status IN ('preparing','running')",[])?;
        transaction.execute("UPDATE relay_jobs SET sync_state='unknown',sync_error='Status delivery was interrupted; inspect Linear before retrying' WHERE sync_state='sending'",[])?;
        if changed > 0 {
            transaction.execute("UPDATE relay_queue_state SET paused=1 WHERE id=1", [])?;
        }
        transaction.commit()?;
        Ok(())
    }
    pub(crate) fn reconcile_jobs(&self) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET status='cancelled',report='Administrator explicitly acknowledged that interrupted processes have stopped' WHERE status='unknown'",[])?;
        Ok(())
    }
    pub(crate) fn claim_status_delivery(&self) -> Result<Option<Record>> {
        let transaction = self.connection.unchecked_transaction()?;
        let id:Option<String>=transaction.query_row("SELECT id FROM relay_jobs WHERE sync_state='pending' AND sync_after<=?1 ORDER BY seq LIMIT 1",[now_secs()],|row|row.get(0)).optional()?;
        if let Some(id) = &id {
            transaction.execute("UPDATE relay_jobs SET sync_state='sending',sync_error=NULL WHERE id=?1 AND sync_state='pending'",[id])?;
        }
        transaction.commit()?;
        id.map(|id| self.job(&id)).transpose().map(Option::flatten)
    }
    pub(crate) fn finish_status_delivery(&self, id: &str, error: Option<&str>) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET sync_state=?2,sync_error=?3 WHERE id=?1 AND sync_state='sending'",params![id,if error.is_some(){"unknown"}else{"sent"},error])?;
        Ok(())
    }
    pub(crate) fn defer_status_delivery(&self, id: &str, error: &str) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET sync_state='pending',sync_error=?2,sync_after=?3 WHERE id=?1 AND sync_state='sending'",params![id,error,now_secs()+60])?;
        Ok(())
    }
    pub(crate) fn retry_status_delivery(&self, id: &str) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET sync_state='pending',sync_error=NULL,sync_after=0 WHERE id=?1 AND status='review' AND sync_state='unknown'",[id])?;
        Ok(())
    }
    pub(crate) fn archive_job(&self, id: &str) -> Result<()> {
        self.connection.execute("DELETE FROM relay_jobs WHERE id=?1 AND status IN ('review','failed','blocked','cancelled') AND sync_state IN ('none','sent','resolved')",[id])?;
        Ok(())
    }
    pub(crate) fn resolve_status_delivery(&self, id: &str) -> Result<()> {
        self.connection.execute("UPDATE relay_jobs SET sync_state='resolved',sync_error='Owner/admin acknowledged manual resolution; no Linear mutation was sent' WHERE id=?1 AND status='review' AND sync_state IN ('pending','unknown')",[id])?;
        Ok(())
    }
}
