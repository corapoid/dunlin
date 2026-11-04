//! Explicit two-agent runs: immutable shared prompt/commit, separate trees,
//! shared durable budget, one dispatch claim per side, no paid restart/retry.
use crate::{
    app::DunlinApp,
    harness::{HarnessState, ParkedRuntime},
};
use dunlin_domain::{
    PermissionMode, Project, ProjectSettings, ProjectTone,
    task_budget::{Budget, Member},
};
use dunlin_ui::*;
use dunlin_workspace::{comparison as api, task_review::Checkpoint};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
const KEY: &str = "comparison-runs-v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Choice {
    pub harness: String,
    pub model: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Spec {
    pub label: String,
    pub prompt: String,
    pub choices: [Choice; 2],
    pub seconds: u64,
    pub requests: u32,
}
impl Spec {
    pub fn validate(&self) -> Result<(), String> {
        if self.label.trim().is_empty()
            || self.label.len() > 128
            || self.prompt.trim().is_empty()
            || self.prompt.len() > 32_000
            || !(1..=86400).contains(&self.seconds)
            || !(2..=1000).contains(&self.requests)
        {
            return Err("Choose a label, prompt (32 KiB), 2–1,000 shared client requests and 1–86,400 seconds".into());
        }
        for choice in &self.choices {
            let known = choice.harness == "Custom ACP"
                || crate::harness_discovery::catalog_names().any(|name| name == choice.harness);
            if !known || choice.model.len() > 512 || choice.model.chars().any(char::is_control) {
                return Err("Choose a known ACP harness and an explicit model wire ID (or blank = advertised default)".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Side {
    #[serde(default)]
    pub settings: ProjectSettings,
    #[serde(default)]
    pub actual_model: Option<String>,
    pub project: String,
    pub session: String,
    pub path: String,
    pub choice: Choice,
    pub baseline: Checkpoint,
    pub claimed: bool,
    pub status: String,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Run {
    pub id: String,
    pub owner: String,
    pub source: String,
    pub head: String,
    pub spec: Spec,
    pub paths: Vec<String>,
    pub sides: Vec<Side>,
    pub error: Option<String>,
    pub started_ms: Option<u64>,
    pub budget: Option<String>,
    pub preferred: Option<usize>,
    #[serde(default)]
    pub reports: Vec<(usize, dunlin_workspace::checks::Report)>,
}
#[derive(Default)]
pub(crate) struct State {
    pub runs: Vec<Run>,
    pub error: Option<String>,
    pub active: HashSet<String>,
}
impl State {
    pub fn load(store: Option<&dunlin_storage::Store>) -> Self {
        match store.map(|s| s.load_setting::<Vec<Run>>(KEY)) {
            Some(Ok(Some(mut runs))) if runs.len() <= 16 => {
                for run in &mut runs {
                    if run.started_ms.is_some() {
                        for side in &mut run.sides {
                            side.status =
                                "not resumed after app restart; inspect transcript".into();
                        }
                    }
                }
                Self {
                    runs,
                    error: None,
                    active: HashSet::new(),
                }
            }
            Some(Ok(None)) => Self::default(),
            _ => Self {
                error: Some("Comparison registry could not be loaded; no work was resumed".into()),
                ..Default::default()
            },
        }
    }
}
impl DunlinApp {
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_comparison_send(
        &mut self,
        id: &str,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        self.with_comparison_runtime(id, false, cx, |this, cx| {
            Ok(this.submit_automation_prompt(text, cx))
        })
        .unwrap()
    }
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_comparison_allow(&mut self, id: &str, cx: &mut Context<Self>) {
        self.with_comparison_runtime(id, false, cx, |this, cx| {
            let request = this
                .harness
                .transcript
                .iter()
                .find_map(|item| match item {
                    crate::harness::ChatItem::Permission {
                        request_id,
                        options,
                        resolved: None,
                        ..
                    } => Some((
                        request_id.clone(),
                        options
                            .iter()
                            .find(|o| o.kind == dunlin_agent::PermissionKind::AllowOnce)
                            .unwrap()
                            .id
                            .clone(),
                    )),
                    _ => None,
                })
                .expect("Pending fixture permission");
            this.resolve_permission(&request.0, Some(request.1), "Smoke allow once".into(), cx);
            Ok(())
        })
        .unwrap();
    }
    pub(crate) fn save_comparisons(&mut self) -> Result<(), String> {
        let result = self
            .store
            .as_ref()
            .ok_or("Storage unavailable".to_string())
            .and_then(|s| {
                s.save_setting(KEY, &self.comparisons.runs)
                    .map_err(|_| "Could not persist comparison state".to_string())
            });
        if let Err(e) = &result {
            self.comparisons.error = Some(e.clone());
            self.comparisons.active.clear();
        }
        result
    }
    pub(crate) fn register_comparison(
        &mut self,
        owner: &Project,
        spec: Spec,
        trees: api::Trees,
        environment: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let mut run = Run {
            id: trees.id,
            owner: owner.id.clone(),
            source: owner.path.clone(),
            head: trees.head,
            spec,
            paths: trees.paths.clone(),
            sides: Vec::new(),
            error: trees.error,
            started_ms: None,
            budget: None,
            preferred: None,
            reports: Vec::new(),
        };
        if self.comparisons.runs.len() >= 16 {
            return Err("Comparison retention limit reached; created trees were retained and must be inspected manually".into());
        }
        // Persist the filesystem result BEFORE importing anything. Partial runs
        // and directories are visible even when a later DB/import step fails.
        self.comparisons.runs.push(run.clone());
        self.save_comparisons()?;
        if run.error.is_none() && trees.paths.len() == 2 && trees.baselines.len() == 2 {
            for i in 0..2 {
                let choice = run.spec.choices[i].clone();
                let mut settings = ProjectSettings {
                    harness: Some(choice.harness.clone()),
                    permission_mode: Some(PermissionMode::Ask),
                    environment: environment.clone(),
                    ..Default::default()
                };
                settings.set_default_model(&choice.harness, &choice.model)?;
                let project = Project {
                    id: format!("compare-project-{}", uuid::Uuid::now_v7()),
                    name: format!("{} · {}", run.spec.label, if i == 0 { "A" } else { "B" }),
                    path: trees.paths[i].clone(),
                    branch: None,
                    tone: if i == 0 {
                        ProjectTone::Blue
                    } else {
                        ProjectTone::Violet
                    },
                    pinned: false,
                    trusted: true,
                    running_sessions: 0,
                    kind: Default::default(),
                    remote: None,
                    group_id: None,
                    settings,
                };
                let mut session = crate::app::blank_session(&project.id, &choice.harness, None);
                session.title = format!("Comparison {}", if i == 0 { "A" } else { "B" });
                let store = self.store.as_ref().ok_or("Storage unavailable")?;
                store
                    .upsert_project(&project)
                    .map_err(|_| "Could not import comparison project")?;
                self.projects.push(project.clone());
                if store.upsert_session(&session).is_err() {
                    run.error=Some("Project imported but session creation failed; inspect retained comparison project".into());
                    break;
                }
                self.all_sessions.push(session.clone());
                run.sides.push(Side {
                    settings: project.settings.clone(),
                    actual_model: None,
                    project: project.id,
                    session: session.id,
                    path: project.path,
                    choice,
                    baseline: trees.baselines[i].clone(),
                    claimed: false,
                    status: "prepared; no agent started".into(),
                    error: None,
                });
                *self.comparisons.runs.last_mut().unwrap() = run.clone();
                self.save_comparisons()?;
            }
        }
        *self.comparisons.runs.last_mut().unwrap() = run;
        self.save_comparisons()?;
        Ok(())
    }
    pub(crate) fn register_remote_comparison(
        &mut self,
        owner: &Project,
        spec: Spec,
        trees: api::Trees,
        host_id: String,
        remote_projects: Vec<crate::remote::RemoteProjectInfo>,
    ) -> Result<(), String> {
        let mut run = Run {
            id: trees.id,
            owner: owner.id.clone(),
            source: owner.path.clone(),
            head: trees.head,
            spec,
            paths: trees.paths.clone(),
            sides: Vec::new(),
            error: trees.error,
            started_ms: None,
            budget: None,
            preferred: None,
            reports: Vec::new(),
        };
        if self.comparisons.runs.len() >= 16 {
            return Err("Comparison retention limit reached; created trees were retained and must be inspected manually".into());
        }
        self.comparisons.runs.push(run.clone());
        self.save_comparisons()?;
        let complete = run.error.is_none()
            && trees.paths.len() == 2
            && trees.baselines.len() == 2
            && remote_projects.len() == 2;
        if !complete && run.error.is_none() {
            run.error = Some(
                "Relay comparison is incomplete; trees/projects were retained for manual inspection"
                    .into(),
            );
            *self.comparisons.runs.last_mut().unwrap() = run.clone();
            self.save_comparisons()?;
            return Ok(());
        }
        if complete {
            for (i, info) in remote_projects.iter().enumerate() {
                let choice = run.spec.choices[i].clone();
                if trees.paths[i] != info.path {
                    run.error = Some(
                        "Relay comparison project path does not match its worktree; no agents started"
                            .into(),
                    );
                    break;
                }
                let mut settings = ProjectSettings {
                    harness: Some(choice.harness.clone()),
                    permission_mode: Some(PermissionMode::Ask),
                    ..Default::default()
                };
                settings.set_default_model(&choice.harness, &choice.model)?;
                let project = Project {
                    id: format!("compare-project-{}", uuid::Uuid::now_v7()),
                    name: format!("{} · {}", run.spec.label, if i == 0 { "A" } else { "B" }),
                    path: info.path.clone(),
                    branch: None,
                    tone: if i == 0 {
                        ProjectTone::Blue
                    } else {
                        ProjectTone::Violet
                    },
                    pinned: false,
                    trusted: true,
                    running_sessions: 0,
                    kind: Default::default(),
                    remote: Some(dunlin_domain::RemoteProject {
                        host_id: host_id.clone(),
                        project_id: info.id.clone(),
                        path: info.path.clone(),
                    }),
                    group_id: None,
                    settings,
                };
                let mut session = crate::app::blank_session(&project.id, &choice.harness, None);
                session.title = format!("Comparison {}", if i == 0 { "A" } else { "B" });
                let store = self.store.as_ref().ok_or("Storage unavailable")?;
                store
                    .upsert_project(&project)
                    .map_err(|_| "Could not import remote comparison project")?;
                self.projects.push(project.clone());
                if store.upsert_session(&session).is_err() {
                    run.error = Some(
                        "Remote project imported but session creation failed; inspect the retained project"
                            .into(),
                    );
                    break;
                }
                self.all_sessions.push(session.clone());
                run.sides.push(Side {
                    settings: project.settings.clone(),
                    actual_model: None,
                    project: project.id,
                    session: session.id,
                    path: project.path,
                    choice,
                    baseline: trees.baselines[i].clone(),
                    claimed: false,
                    status: "prepared; no agent started".into(),
                    error: None,
                });
                *self.comparisons.runs.last_mut().unwrap() = run.clone();
                self.save_comparisons()?;
            }
        }
        *self.comparisons.runs.last_mut().unwrap() = run;
        self.save_comparisons()?;
        Ok(())
    }

    pub(crate) fn launch_comparison(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let index = self
            .comparisons
            .runs
            .iter()
            .position(|r| r.id == id)
            .ok_or("Comparison unavailable")?;
        let run = self.comparisons.runs[index].clone();
        if self.comparisons.error.is_some() {
            return Err("Comparison storage is unverifiable; no adapter start authorized".into());
        }
        if self
            .parked_runtimes
            .values()
            .filter(|r| r.state.harness.is_some())
            .count()
            + usize::from(self.harness.harness.is_some())
            > 6
        {
            return Err("Two free adapter slots are required".into());
        }
        if run.started_ms.is_some() || run.error.is_some() || run.sides.len() != 2 {
            return Err(
                "This comparison was already started or is incomplete; no automatic resume/retry"
                    .into(),
            );
        }
        for side in &run.sides {
            let project = self
                .projects
                .iter()
                .find(|p| p.id == side.project)
                .ok_or("Comparison project disappeared")?;
            if project.group_id.is_some()
                || serde_json::to_value(&project.settings).ok()
                    != serde_json::to_value(&side.settings).ok()
            {
                return Err("Comparison runtime settings/group context changed; prepare a new comparison rather than silently broadening consent".into());
            }
            if !self
                .projects
                .iter()
                .any(|p| p.id == side.project && p.path == side.path && p.trusted)
                || !self.all_sessions.iter().any(|s| {
                    s.id == side.session
                        && s.project_id == side.project
                        && s.harness == side.choice.harness
                        && s.acp_session_id.is_none()
                })
            {
                return Err("Comparison project/session identity changed".into());
            }
        }
        let now = crate::harness::now_ms();
        let budget = Budget {
            id: uuid::Uuid::now_v7().to_string(),
            label: run.spec.label.clone(),
            started_ms: now,
            deadline_ms: now + run.spec.seconds * 1000,
            max_attempts: run.spec.requests,
            reported_usd_limit: None,
            stopped: None,
            members: run
                .sides
                .iter()
                .map(|s| Member {
                    session: s.session.clone(),
                    project: s.project.clone(),
                    root: s.path.clone(),
                    attempts: 0,
                    baseline: None,
                    latest: None,
                    opaque_children: false,
                })
                .collect(),
        };
        let budget_id = budget.id.clone();
        self.approve_task_budget(budget)?;
        self.comparisons.runs[index].started_ms = Some(now);
        self.comparisons.runs[index].budget = Some(budget_id);
        self.save_comparisons()?;
        self.comparisons.active.insert(id.into());
        for side in &run.sides {
            let result = self.with_comparison_runtime(&side.session, true, cx, |this, cx| {
                this.connect_harness(cx);
                Ok(())
            });
            if let Err(e) = result
                && let Some(record) = self.comparisons.runs[index]
                    .sides
                    .iter_mut()
                    .find(|s| s.session == side.session)
            {
                record.error = Some(e);
                record.status = "failed before dispatch".into();
                record.claimed = true;
            }
        }
        self.save_comparisons()?;
        self.persist_settings(cx);
        cx.notify();
        Ok(())
    }
    fn with_comparison_runtime<T>(
        &mut self,
        id: &str,
        create: bool,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &mut Context<Self>) -> Result<T, String>,
    ) -> Result<T, String> {
        let session = self
            .all_sessions
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .ok_or("Comparison session disappeared")?;
        let index = self
            .projects
            .iter()
            .position(|p| p.id == session.project_id && p.trusted)
            .ok_or("Comparison project is no longer trusted")?;
        if self.session().id == id {
            return action(self, cx);
        }
        let mut runtime = match self.parked_runtimes.remove(id) {
            Some(r) => r,
            None if create => ParkedRuntime {
                project: session.project_id.clone(),
                root: self.projects[index].path.clone(),
                state: HarnessState::default(),
            },
            _ => {
                return Err(
                    "Comparison runtime is not connected; explicit reconnect is required".into(),
                );
            }
        };
        if runtime.project != session.project_id || runtime.root != self.projects[index].path {
            self.parked_runtimes.insert(id.into(), runtime);
            return Err("Comparison runtime root changed".into());
        }
        let old_project = std::mem::replace(&mut self.selected_project, index);
        let old_sessions = std::mem::replace(&mut self.sessions, vec![session]);
        let old_selected = std::mem::replace(&mut self.selected_session, 0);
        let notice = self.notice.clone();
        std::mem::swap(&mut self.harness, &mut runtime.state);
        let result = action(self, cx);
        std::mem::swap(&mut self.harness, &mut runtime.state);
        self.selected_project = old_project;
        self.sessions = old_sessions;
        self.selected_session = old_selected;
        self.notice = notice;
        for session in &mut self.sessions {
            if let Some(updated) = self.all_sessions.iter().find(|s| s.id == session.id) {
                *session = updated.clone();
            }
        }
        self.parked_runtimes.insert(id.into(), runtime);
        self.persist_settings(cx);
        result
    }
    pub(crate) fn comparison_runtime(&self, id: &str) -> Option<&HarnessState> {
        if self.session().id == id {
            Some(&self.harness)
        } else {
            self.parked_runtimes.get(id).map(|r| &r.state)
        }
    }
    pub(crate) fn advance_comparisons(&mut self, cx: &mut Context<Self>) {
        if self.comparisons.error.is_some() {
            return;
        }
        let ids = self.comparisons.active.iter().cloned().collect::<Vec<_>>();
        let mut changed = false;
        for id in ids {
            let Some(index) = self.comparisons.runs.iter().position(|r| r.id == id) else {
                continue;
            };
            let run = self.comparisons.runs[index].clone();
            for (i, side) in run.sides.iter().enumerate() {
                if self.comparisons.error.is_some() {
                    break;
                }
                if side.error.is_some() {
                    continue;
                }
                if side.claimed {
                    let status = self
                        .comparison_runtime(&side.session)
                        .map(|runtime| {
                            if runtime.running {
                                "running"
                            } else if runtime.status.is_busy() {
                                "connecting"
                            } else {
                                "stopped/finished; inspect transcript and evidence"
                            }
                        })
                        .unwrap_or("runtime unavailable; inspect saved transcript");
                    if self.comparisons.runs[index].sides[i].status != status {
                        self.comparisons.runs[index].sides[i].status = status.into();
                        changed = true;
                    }
                    continue;
                }
                if !self.projects.iter().any(|p| {
                    p.id == side.project
                        && p.path == side.path
                        && p.trusted
                        && p.group_id.is_none()
                        && serde_json::to_value(&p.settings).ok()
                            == serde_json::to_value(&side.settings).ok()
                }) {
                    self.comparisons.runs[index].sides[i].error=Some("Comparison project/settings changed before dispatch; consent no longer matches".into());
                    changed = true;
                    continue;
                }
                let elapsed = crate::harness::now_ms().saturating_sub(run.started_ms.unwrap_or(0));
                let stop=self.task_budgets.error.clone().or_else(||match self.task_budgets.registry.task(&side.session){
                    Some(t) if Some(&t.id)==run.budget.as_ref()=>t.active_stop(crate::harness::now_ms()),
                    _=>Some("Approved shared comparison budget was released or replaced; dispatch refused".into()),
                });
                let state = self.comparison_runtime(&side.session);
                if stop.is_some()
                    || elapsed > 60_000
                    || state.is_none()
                    || state.is_some_and(|s| {
                        s.missing_command.is_some()
                            || matches!(
                                s.status,
                                crate::harness::HarnessStatus::Failed(_)
                                    | crate::harness::HarnessStatus::Exited(_)
                            )
                    })
                {
                    self.comparisons.runs[index].sides[i].error = Some(stop.unwrap_or(
                        "Adapter/model did not become ready within 60 seconds; no retry".into(),
                    ));
                    changed = true;
                    continue;
                }
                let runtime = state.unwrap();
                if !runtime.status.is_ready()
                    || runtime.local_session_loading
                    || runtime.session_id.is_none()
                    || runtime.model_changing
                    || runtime.mode_changing
                {
                    continue;
                }
                if !side.choice.model.is_empty()
                    && runtime
                        .models
                        .as_ref()
                        .is_none_or(|m| m.current != side.choice.model)
                {
                    continue;
                }
                if runtime.running
                    || self
                        .task_budgets
                        .registry
                        .task(&side.session)
                        .is_some_and(|t| {
                            t.members
                                .iter()
                                .any(|m| m.session == side.session && m.attempts > 0)
                        })
                {
                    self.comparisons.runs[index].sides[i].error=Some("Session was used outside its pending comparison; duplicate dispatch refused".into());
                    changed = true;
                    continue;
                }
                // Durable one-shot claim precedes wire reservation. A lost app
                // response/restart never automatically resubmits paid work.
                let actual_model = runtime.models.as_ref().map(|m| m.current.clone());
                self.comparisons.runs[index].sides[i].actual_model = actual_model;
                self.comparisons.runs[index].sides[i].claimed = true;
                self.comparisons.runs[index].sides[i].status = "dispatch claimed".into();
                if let Err(e) = self.save_comparisons() {
                    self.comparisons.runs[index].sides[i].error = Some(e);
                    changed = true;
                    continue;
                }
                let prompt = run.spec.prompt.clone();
                let sent = self.with_comparison_runtime(&side.session, false, cx, |this, cx| {
                    Ok(this.submit_automation_prompt(prompt, cx))
                });
                match sent{Ok(true)=>self.comparisons.runs[index].sides[i].status="running".into(),_=>self.comparisons.runs[index].sides[i].error=Some("Dispatch guard refused this request; no automatic retry. Open the session to inspect its policy/model/budget.".into())}
                changed = true;
            }
            if self.comparisons.runs[index].sides.iter().all(|s| {
                s.error.is_some()
                    || (s.claimed
                        && self
                            .comparison_runtime(&s.session)
                            .is_none_or(|r| !r.running && !r.status.is_busy()))
            }) {
                self.comparisons.active.remove(&id);
            }
        }
        if changed {
            if let Err(e) = self.save_comparisons() {
                self.comparisons.error = Some(e);
            }
            cx.notify();
        }
    }
}
