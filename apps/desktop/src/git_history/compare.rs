use super::*;
use crate::comparisons::{Choice, Spec};
use dunlin_ui::component::input::{Textarea, TextareaState};
use dunlin_workspace::{checks, comparison as api, task_review};
use std::sync::{Arc, atomic::AtomicBool};
enum CompareCheckPlan {
    Local(checks::Prepared),
    Remote {
        workspace: crate::remote::RemoteWorkspace,
        project_id: String,
    },
}

#[allow(clippy::large_enum_variant)]
enum PreparedComparison {
    Local(api::Prepared),
    Remote {
        workspace: crate::remote::RemoteWorkspace,
        prepared: crate::remote::RemotePreparedComparison,
    },
}

impl PreparedComparison {
    fn head(&self) -> &str {
        match self {
            Self::Local(plan) => &plan.head,
            Self::Remote { prepared, .. } => &prepared.head,
        }
    }
    fn paths(&self) -> &[String] {
        match self {
            Self::Local(plan) => &plan.paths,
            Self::Remote { prepared, .. } => &prepared.paths,
        }
    }
    fn confirmation_note(&self) -> &'static str {
        match self {
            Self::Local(_) => {
                "Creates two detached local worktrees and imports two local projects/sessions. Source must be clean; unsaved/dirty source edits are never copied. Git may run configured hooks/filters/signing tools (not sandboxed). New projects copy the effective local environment and use Ask permissions; no group context. Partial trees are retained on failure, never automatically deleted. This action starts NO agents and incurs no provider request. Agent starts need a separate confirmation."
            }
            Self::Remote { .. } => {
                "Creates two detached worktrees and relay projects/sessions on the relay host. Source must be clean; Git may run server-side hooks/filters (not sandboxed). The relay registers explicit grants for both worktree paths with Ask permissions; no group context. Partial trees are retained on failure, never automatically deleted. This action starts NO agents and incurs no provider request. Agent starts need a separate confirmation."
            }
        }
    }
}

pub(super) struct State {
    pub visible: bool,
    label: Entity<InputState>,
    prompt: Entity<TextareaState>,
    harness: [Entity<InputState>; 2],
    model: [Entity<InputState>; 2],
    seconds: Entity<InputState>,
    requests: Entity<InputState>,
    check: Entity<InputState>,
    pending: Option<(uuid::Uuid, PreparedComparison, Spec)>,
    preview: Option<(String, Vec<String>)>,
    error: Option<String>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(w: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            label: cx.new(|cx| InputState::new(w, cx).placeholder("Comparison label")),
            prompt: cx
                .new(|cx| TextareaState::new(w, cx).placeholder("Identical prompt for A and B")),
            harness: [
                cx.new(|cx| InputState::new(w, cx).default_value("OpenCode")),
                cx.new(|cx| InputState::new(w, cx).default_value("OpenAI Codex")),
            ],
            model: [
                cx.new(|cx| {
                    InputState::new(w, cx)
                        .placeholder("A model wire ID; blank = advertised default")
                }),
                cx.new(|cx| {
                    InputState::new(w, cx)
                        .placeholder("B model wire ID; blank = advertised default")
                }),
            ],
            seconds: cx.new(|cx| InputState::new(w, cx).default_value("600")),
            requests: cx.new(|cx| InputState::new(w, cx).default_value("2")),
            check: cx.new(|cx| {
                InputState::new(w, cx).placeholder("Optional same checks: JSON argv for both trees")
            }),
            pending: None,
            preview: None,
            error: None,
            scroll: ScrollHandle::new(),
        }
    }
}
impl DunlinApp {
    fn forget_comparison(&mut self, id: String, w: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.comparisons.runs.iter().find(|r| r.id == id) else {
            return;
        };
        if self.git_history.running
            || self.comparisons.active.contains(&id)
            || run.sides.iter().any(|s| {
                self.comparison_runtime(&s.session).is_some_and(|r| {
                    r.running || r.status.is_busy() || !r.queued_prompts().is_empty()
                })
            })
        {
            self.file_notice("Finish/stop comparison work and clear retained queues before forgetting its record",cx);
            return;
        }
        let receiver=w.prompt(PromptLevel::Warning,"Forget comparison record and saved logs?",Some("Deletes this comparison's local metadata, baseline snapshots, preference and check reports. Worktrees, files, imported projects/sessions, transcripts and shared budget ledgers are RETAINED; remove/release those separately after review. No Git operation, merge or agent request occurs."),&["Cancel","Forget record and logs"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let Some(run) = this.comparisons.runs.iter().find(|r| r.id == id) else {
                    return;
                };
                if this.git_history.running
                    || this.comparisons.active.contains(&id)
                    || run.sides.iter().any(|s| {
                        this.comparison_runtime(&s.session).is_some_and(|r| {
                            r.running || r.status.is_busy() || !r.queued_prompts().is_empty()
                        })
                    })
                {
                    return;
                }
                let previous = this.comparisons.runs.clone();
                this.comparisons.runs.retain(|r| r.id != id);
                if let Err(e) = this.save_comparisons() {
                    this.comparisons.runs = previous;
                    this.file_notice(e, cx);
                } else if let Some(v) = &mut this.git_history.view
                    && v.compare
                        .preview
                        .as_ref()
                        .is_some_and(|(target, _)| target == &id)
                {
                    v.compare.preview = None;
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn open_comparisons(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(w, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let v = self.git_history.view.as_mut().unwrap();
        v.hide_tools();
        v.compare.visible = true;
        cx.notify();
    }
    fn comparison_guard(&self, close_files: bool) -> Result<(), String> {
        let v = self
            .git_history
            .view
            .as_ref()
            .ok_or("Open comparisons first")?;
        if !owns(&v.project, self.project()) {
            return Err(
                "Comparisons require the same trusted source project; reload the tab".into(),
            );
        }
        if self.git_history.running
            || self.branch_ui.running()
            || self.explorer.busy
            || !self.live_agents().is_empty()
            || self.projects.iter().any(|p| {
                self.git_mutation_in_flight(&p.id) || self.ai_commit_locks_project_id(&p.id)
            })
        {
            return Err("Finish connected agents and repository/file operations first".into());
        }
        if self.editor_dirty
            || self
                .workbench
                .tabs
                .iter()
                .any(|t| t.dirty || t.loading || t.git_changed || t.conflict.is_some())
            || (close_files && !self.workbench.tabs.is_empty())
        {
            return Err("Save buffers/resolve file reviews first; close all file tabs before running arbitrary checks".into());
        }
        if self.comparisons.error.is_some() {
            return Err("Repair comparison storage before preparing new work".into());
        }
        Ok(())
    }
    fn comparison_lock(&mut self, owner: &Project, lock: bool) {
        self.git_history.running = lock;
        self.git_history.mutation_global = lock;
        self.git_history.mutation_project = lock.then(|| owner.id.clone());
        if lock {
            self.begin_git_mutation(&owner.id);
        } else {
            self.finish_git_mutation(&owner.id);
        }
    }
    fn prepare_comparison(&mut self, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        if let Err(e) = self.comparison_guard(false) {
            self.file_notice(e, cx);
            return;
        }
        if self.comparisons.runs.len() >= 16 {
            self.file_notice(
                "Retain at most 16 comparison records; explicitly forget an obsolete record first",
                cx,
            );
            return;
        }
        let v = self.git_history.view.as_ref().unwrap();
        let s = &v.compare;
        let (Ok(seconds), Ok(requests)) = (
            s.seconds.read(cx).value().parse(),
            s.requests.read(cx).value().parse(),
        ) else {
            self.file_notice("Use integer time/request limits", cx);
            return;
        };
        let spec = Spec {
            label: s.label.read(cx).value().to_string(),
            prompt: s.prompt.read(cx).value().to_string(),
            choices: [
                Choice {
                    harness: s.harness[0].read(cx).value().to_string(),
                    model: s.model[0].read(cx).value().to_string(),
                },
                Choice {
                    harness: s.harness[1].read(cx).value().to_string(),
                    model: s.model[1].read(cx).value().to_string(),
                },
            ],
            seconds,
            requests,
        };
        if let Err(e) = spec.validate() {
            self.file_notice(e, cx);
            return;
        }
        let id = v.id;
        let owner = v.project.clone();
        let root = owner.path.clone();
        let remote = if owner.is_remote() {
            match self.remote_workspace_for_project(&owner) {
                Ok(workspace) => Some(workspace),
                Err(error) => {
                    self.file_notice(error, cx);
                    return;
                }
            }
        } else {
            None
        };
        self.comparison_lock(&owner, true);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(workspace) = remote {
                        workspace
                            .compare_prepare()
                            .map(|prepared| PreparedComparison::Remote {
                                workspace,
                                prepared,
                            })
                    } else {
                        api::prepare(&root).map(PreparedComparison::Local)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.comparison_lock(&owner, false);
                if let Some(v) = &mut this.git_history.view
                    && v.id == id
                    && this.projects.iter().any(|p| owns(&owner, p))
                {
                    match result {
                        Ok(plan) => v.compare.pending = Some((uuid::Uuid::now_v7(), plan, spec)),
                        Err(e) => v.compare.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn confirm_comparison_trees(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.comparison_guard(false) {
            self.file_notice(e, cx);
            return;
        }
        let v = self.git_history.view.as_ref().unwrap();
        let Some((token, plan, spec)) = &v.compare.pending else {
            return;
        };
        let id = v.id;
        let token = *token;
        let owner = v.project.clone();
        let environment = match self
            .resolved_project_settings(&owner)
            .and_then(|s| s.process_environment())
        {
            Ok(e) => e,
            Err(e) => {
                self.file_notice(e, cx);
                return;
            }
        };
        let paths = plan.paths();
        let summary = format!(
            "{}\nPinned commit: {}\nA: {}\nB: {}\n\n{}",
            spec.label,
            plan.head(),
            paths.first().map(String::as_str).unwrap_or("unknown"),
            paths.get(1).map(String::as_str).unwrap_or("unknown"),
            plan.confirmation_note()
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Create comparison worktrees?",
            Some(&summary),
            &["Cancel", "Create both trees"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.comparison_guard(false).is_err() {
                    return;
                }
                let Some(v) = this.git_history.view.as_mut().filter(|v| {
                    v.id == id
                        && v.compare
                            .pending
                            .as_ref()
                            .is_some_and(|(t, _, _)| *t == token)
                }) else {
                    return;
                };
                let (_, plan, spec) = v.compare.pending.take().unwrap();
                this.comparison_lock(&owner, true);
                cx.spawn(async move |this, cx| {
                    let outcome = cx
                        .background_executor()
                        .spawn(async move {
                            match plan {
                                PreparedComparison::Local(plan) => {
                                    Ok::<_, String>((api::create(plan), Vec::new(), None))
                                }
                                PreparedComparison::Remote {
                                    workspace,
                                    prepared,
                                } => {
                                    let response = workspace.compare_create(&prepared.id)?;
                                    Ok((
                                        response.trees,
                                        response.projects,
                                        Some(workspace.host.id),
                                    ))
                                }
                            }
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.comparison_lock(&owner, false);
                        let (mut trees, remote_projects, host) = match outcome {
                            Ok(value) => value,
                            Err(error) => {
                                this.file_notice(error, cx);
                                return;
                            }
                        };
                        if !this.projects.iter().any(|p| owns(&owner, p)) {
                            trees.error = Some(
                                "Source trust/identity changed; trees retained and no projects/agents started"
                                    .into(),
                            );
                        }
                        let result = if let Some(host) = host {
                            this.register_remote_comparison(
                                &owner,
                                spec,
                                trees,
                                host,
                                remote_projects,
                            )
                        } else {
                            this.register_comparison(&owner, spec, trees, environment)
                        };
                        match result {
                            Ok(()) => this.file_notice(
                                "Comparison paths recorded; inspect preparation errors before separately starting agents",
                                cx,
                            ),
                            Err(e) => this.file_notice(e, cx),
                        }
                        this.persist_settings(cx);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
    }
    fn confirm_comparison_start(&mut self, target: String, w: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.comparison_guard(false) {
            self.file_notice(e, cx);
            return;
        }
        let connected = self
            .parked_runtimes
            .values()
            .filter(|r| r.state.harness.is_some())
            .count()
            + usize::from(self.harness.harness.is_some());
        if connected > 6 {
            self.file_notice("Two comparison adapters need two free slots (eight total); disconnect idle adapters explicitly first",cx);
            return;
        }
        let Some(run) = self
            .comparisons
            .runs
            .iter()
            .find(|r| {
                r.id == target
                    && r.owner == self.project().id
                    && r.source == self.project().path
                    && r.started_ms.is_none()
                    && r.error.is_none()
                    && r.sides.len() == 2
            })
            .cloned()
        else {
            return;
        };
        let owner = self.project().clone();
        let commands = run
            .spec
            .choices
            .iter()
            .map(|c| {
                std::env::var("DUNLIN_HARNESS_COMMAND")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| {
                        crate::harness::harness_command(&c.harness, &self.custom_harness_command)
                    })
            })
            .collect::<Vec<_>>();
        let remote_run = run.sides.iter().any(|side| {
            self.projects
                .iter()
                .find(|project| project.id == side.project)
                .is_some_and(Project::is_remote)
        });
        let execution = if remote_run {
            "Starts two ACP sessions through the relay host using its configured harness and Ask permissions. Relay worktrees are separate working directories, NOT security sandboxes; hooks and external writers are not transactional. No automatic retries or paid app-restart resume.".to_string()
        } else {
            format!(
                "Starts two local ACP adapters (`{}`, `{}`) with inherited configured environment/credentials and Ask permissions. Worktrees are separate working directories, NOT security sandboxes. No automatic retries or paid app-restart resume.",
                commands[0], commands[1]
            )
        };
        let summary = format!(
            "Task {} · commit {}\nA: {} / {} · {}\nB: {} / {} · {}\nShared cap: {} client requests / {}s from start. Reported USD may be UNKNOWN; there is NO USD billing cap.\n\n{}\n\nBoth receive this EXACT prompt:\n\n{}",
            run.spec.label,
            run.head,
            run.sides[0].choice.harness,
            run.sides[0].choice.model,
            if remote_run {
                "relay-configured"
            } else {
                commands[0].as_str()
            },
            run.sides[1].choice.harness,
            run.sides[1].choice.model,
            if remote_run {
                "relay-configured"
            } else {
                commands[1].as_str()
            },
            run.spec.requests,
            run.spec.seconds,
            execution,
            run.spec.prompt
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Authorize both agent requests (may incur costs)?",
            Some(&summary),
            &["Cancel", "Start A and B once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        let mut validators = Vec::new();
        for side in &run.sides {
            let Some(project) = self.projects.iter().find(|p| p.id == side.project).cloned() else {
                self.file_notice("Comparison project disappeared", cx);
                return;
            };
            if project.is_remote() {
                match self.remote_workspace_for_project(&project) {
                    Ok(workspace) => validators.push(Some(workspace)),
                    Err(error) => {
                        self.file_notice(error, cx);
                        return;
                    }
                }
            } else {
                validators.push(None);
            }
        }
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.comparison_guard(false).is_err() || !owns(&owner, this.project()) {
                    return;
                }
                let approved = commands;
                if !remote_run {
                    for (i, choice) in run.spec.choices.iter().enumerate() {
                        let command = std::env::var("DUNLIN_HARNESS_COMMAND")
                            .ok()
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or_else(|| {
                                crate::harness::harness_command(
                                    &choice.harness,
                                    &this.custom_harness_command,
                                )
                            });
                        if command != approved[i] {
                            this.file_notice(
                                "Adapter command changed after confirmation; no agents started",
                                cx,
                            );
                            return;
                        }
                    }
                }
                this.comparison_lock(&owner, true);
                let check_run = run.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            for (side, remote) in check_run.sides.iter().zip(validators) {
                                match remote {
                                    Some(workspace) => {
                                        workspace.compare_validate(&check_run.head)?;
                                    }
                                    None => {
                                        api::validate_tree(&side.path, &check_run.head)?;
                                    }
                                }
                            }
                            Ok::<_, String>(())
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.comparison_lock(&owner, false);
                        if !owns(&owner, this.project())
                            || this.comparison_guard(false).is_err()
                        {
                            return;
                        }
                        // Validation yields to the UI: consent must still match
                        // at the adapter boundary, not just before the worker
                        // starts.
                        if !remote_run {
                            for (i, choice) in run.spec.choices.iter().enumerate() {
                                let command = std::env::var("DUNLIN_HARNESS_COMMAND")
                                    .ok()
                                    .filter(|s| !s.trim().is_empty())
                                    .unwrap_or_else(|| {
                                        crate::harness::harness_command(
                                            &choice.harness,
                                            &this.custom_harness_command,
                                        )
                                    });
                                if command != approved[i] {
                                    this.file_notice(
                                        "Adapter command changed during validation; no agents started",
                                        cx,
                                    );
                                    return;
                                }
                            }
                        }
                        match result.and_then(|_| this.launch_comparison(&target, cx)) {
                            Ok(()) => this.file_notice(
                                "Comparison adapters started; each approved prompt is durably claimed once",
                                cx,
                            ),
                            Err(e) => this.file_notice(e, cx),
                        }
                        cx.notify();
                    });
                })
                .detach();
            });
        })
        .detach();
    }
    fn inspect_comparison(&mut self, target: String, cx: &mut Context<Self>) {
        let Some(run) = self
            .comparisons
            .runs
            .iter()
            .find(|r| r.id == target)
            .cloned()
        else {
            return;
        };
        let Some(v) = &self.git_history.view else {
            return;
        };
        let id = v.id;
        let mut remote_sides = Vec::new();
        for side in &run.sides {
            let Some(project) = self.projects.iter().find(|p| p.id == side.project).cloned() else {
                self.file_notice("Comparison project disappeared", cx);
                return;
            };
            if project.is_remote() {
                match self.remote_workspace_for_project(&project) {
                    Ok(workspace) => remote_sides.push(Some(workspace)),
                    Err(error) => {
                        self.file_notice(error, cx);
                        return;
                    }
                }
            } else {
                remote_sides.push(None);
            }
        }
        cx.spawn(async move|this,cx|{
            let previews=cx.background_executor().spawn(async move{
                run.sides.iter().enumerate().map(|(index, side)|{
                    let changes = match remote_sides.get(index).and_then(Option::as_ref) {
                        Some(workspace) => workspace.compare_changes(&side.baseline)?,
                        None => task_review::changes(&side.baseline)?,
                    };
                    let mut text=format!("{} changed saved files since shared commit {}. Author/test coverage not inferred.\n",changes.len(),run.head);
                    for c in changes.iter().take(20){
                        text.push_str(&format!("\n{}\n",c.path));
                        let old=c.before.as_ref().and_then(|i|i.text());
                        let new=c.after.as_ref().and_then(|i|i.text());
                        if (c.before.is_some()&&old.is_none())||(c.after.is_some()&&new.is_none()){
                            text.push_str("Binary/non-UTF-8 change\n");
                        }else{
                            text.push_str(&similar::TextDiff::configure().timeout(std::time::Duration::from_millis(60)).diff_lines(old.as_deref().unwrap_or(""),new.as_deref().unwrap_or("")).unified_diff().context_radius(2).to_string());
                        }
                        if text.len()>80_000{break;}
                    }
                    Ok::<_,String>(dunlin_workspace::checks::display_text(&text,80_000))
                }).map(|r|r.unwrap_or_else(|e|format!("Evidence unavailable, not an empty diff: {e}"))).collect::<Vec<_>>()
            }).await;
            let _=this.update(cx,|this,cx|{if let Some(v)=&mut this.git_history.view&&v.id==id{v.compare.preview=Some((target,previews));}cx.notify();});
        }).detach();
    }
    fn check_both_comparison_trees(
        &mut self,
        target: String,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = self.comparison_guard(true) {
            self.file_notice(e, cx);
            return;
        }
        let Some(v) = &self.git_history.view else {
            return;
        };
        let Ok(argv) = serde_json::from_str::<Vec<String>>(&v.compare.check.read(cx).value())
        else {
            self.file_notice(
                "Enter a JSON argv array for the same checks on both trees",
                cx,
            );
            return;
        };
        let Some(run) = self
            .comparisons
            .runs
            .iter()
            .find(|r| r.id == target && r.owner == self.project().id && r.sides.len() == 2)
            .cloned()
        else {
            return;
        };
        let mut plans = Vec::new();
        for side in &run.sides {
            let Some(project) = self.projects.iter().find(|p| p.id == side.project).cloned() else {
                self.file_notice("Comparison project disappeared", cx);
                return;
            };
            if project.is_remote() {
                match self.remote_workspace_for_project(&project) {
                    Ok(workspace) => plans.push(CompareCheckPlan::Remote {
                        workspace,
                        project_id: project
                            .remote
                            .as_ref()
                            .map(|remote| remote.project_id.clone())
                            .unwrap_or_default(),
                    }),
                    Err(error) => {
                        self.file_notice(error, cx);
                        return;
                    }
                }
            } else {
                match checks::prepare(&side.path, argv.clone()) {
                    Ok(plan) => plans.push(CompareCheckPlan::Local(plan)),
                    Err(error) => {
                        self.file_notice(error, cx);
                        return;
                    }
                }
            }
        }
        let summary = format!(
            "Runs this argv ONCE in EACH comparison tree:\n{}\n{}\n{}\nNative project code is NOT sandboxed; local checks run with this Mac's permissions and remote checks with the relay process's permissions. Commands may change files or use credentials/network. 120s and 512 KiB per stream per command. Logs are stored locally and may contain secrets. No test success is inferred from text. No automatic retry.",
            serde_json::to_string(&argv).unwrap_or_default(),
            run.sides[0].path,
            run.sides[1].path
        );
        let owner = self.project().clone();
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Run the same checks in A and B?",
            Some(&summary),
            &["Cancel", "Run both checks once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.comparison_guard(true).is_err() || !owns(&owner, this.project()) {
                    return;
                }
                this.comparison_lock(&owner, true);
                let cancel = Arc::new(AtomicBool::new(false));
                this.git_history.cancel_check = Some(cancel.clone());
                cx.spawn(async move |this, cx| {
                    let results = cx
                        .background_executor()
                        .spawn(async move {
                            plans
                                .into_iter()
                                .enumerate()
                                .map(|(i, plan)| {
                                    let result = match plan {
                                        CompareCheckPlan::Local(plan) => {
                                            checks::run(plan, cancel.clone())
                                        }
                                        CompareCheckPlan::Remote {
                                            workspace,
                                            project_id,
                                        } => crate::remote::run_check(
                                            &workspace.host.url,
                                            &workspace.token,
                                            &project_id,
                                            argv.clone(),
                                        ),
                                    };
                                    (i, result)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.comparison_lock(&owner, false);
                        this.git_history.cancel_check = None;
                        if let Some(run) =
                            this.comparisons.runs.iter_mut().find(|r| r.id == target)
                        {
                            run.reports.clear();
                            for (side, result) in results {
                                match result {
                                    Ok(mut report) => {
                            report.stdout = dunlin_workspace::checks::display_text(
                                            &report.stdout,
                                            40_000,
                                        );
                            report.stderr = dunlin_workspace::checks::display_text(
                                            &report.stderr,
                                            40_000,
                                        );
                                        run.reports.push((side, report));
                                    }
                                    Err(error) => {
                                        run.error = Some(format!(
                                            "Check {} unavailable: {error}",
                                            side + 1
                                        ))
                                    }
                                }
                            }
                        }
                        if let Err(error) = this.save_comparisons() {
                            this.file_notice(error, cx);
                        }
                        for tab in &mut this.workbench.tabs {
                            tab.git_changed = true;
                            tab.reload_generation = tab.reload_generation.wrapping_add(1);
                            tab.conflict = Some(crate::app::EditorConflict {
                                path: tab.inspector.path.clone(),
                                message: "Comparison checks ran while this buffer was open; review/reload explicitly".into(),
                            });
                        }
                        this.refresh_git_status(cx);
                        this.refresh_file_tree(cx);
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
    }
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let s = &this.git_history.view.as_ref().unwrap().compare;
    let p = palette(cx);
    let busy = this.git_history.running;
    let mut body=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.)).child("Same committed baseline + identical approved prompt. Separate worktrees are not security sandboxes. Creating trees does not start agents; starting both requires a separate cost confirmation and shared durable request/time budget.")
        .child(Input::new(&s.label).small()).child(Textarea::new(&s.prompt).h(px(100.)));
    for i in 0..2 {
        body = body
            .child(format!(
                "{} harness / model",
                if i == 0 { "A" } else { "B" }
            ))
            .child(Input::new(&s.harness[i]).small())
            .child(Input::new(&s.model[i]).small());
    }
    body = body
        .child("Shared wall time (seconds) / client request attempts")
        .child(Input::new(&s.seconds).small())
        .child(Input::new(&s.requests).small())
        .child(
            Button::new("compare-prepare")
                .small()
                .label("Prepare identical worktrees…")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.prepare_comparison(cx))),
        );
    if let Some((_, plan, _)) = &s.pending {
        let paths = plan.paths();
        body = body
            .child(format!(
                "Commit {}\n{}\n{}",
                plan.head(),
                paths.first().map(String::as_str).unwrap_or("unknown"),
                paths.get(1).map(String::as_str).unwrap_or("unknown"),
            ))
            .child(
                Button::new("compare-create")
                    .small()
                    .label("Confirm worktree creation…")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, w, cx| this.confirm_comparison_trees(w, cx))),
            );
    }
    body = body.child(Input::new(&s.check).small());
    for run in this
        .comparisons
        .runs
        .iter()
        .rev()
        .filter(|r| r.owner == this.project().id && r.source == this.project().path)
    {
        let start = run.id.clone();
        let inspect = start.clone();
        let check = start.clone();
        let forget = start.clone();
        body = body
            .child(
                div()
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(p.border)
                    .child(format!(
                        "{} · commit {} · preferred {:?}",
                        run.spec.label,
                        run.head,
                        run.preferred.map(|i| if i == 0 { "A" } else { "B" })
                    )),
            )
            .child(
                Button::new(format!("compare-start-{start}"))
                    .small()
                    .label("Authorize A + B agent requests…")
                    .disabled(
                        busy || run.started_ms.is_some()
                            || run.error.is_some()
                            || run.sides.len() != 2,
                    )
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.confirm_comparison_start(start.clone(), w, cx)
                    })),
            )
            .child(
                Button::new(format!("compare-inspect-{inspect}"))
                    .ghost()
                    .small()
                    .label("Refresh A/B changes")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.inspect_comparison(inspect.clone(), cx)
                    })),
            )
            .child(
                Button::new(format!("compare-check-{check}"))
                    .ghost()
                    .small()
                    .label("Run identical checks in A/B…")
                    .disabled(busy || run.sides.len() != 2)
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.check_both_comparison_trees(check.clone(), w, cx)
                    })),
            );
        body =
            body.child(
                Button::new(format!("compare-forget-{forget}"))
                    .ghost()
                    .small()
                    .label("Forget record and logs (keep trees)…")
                    .disabled(busy || this.comparisons.active.contains(&run.id))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        this.forget_comparison(forget.clone(), w, cx)
                    })),
            );
        if let Some(e) = &run.error {
            body = body.child(div().text_color(p.red).child(e.clone()));
        }
        for path in &run.paths {
            body = body.child(path.clone());
        }
        if let Some(budget) = run.budget.as_ref().and_then(|id| {
            this.task_budgets
                .registry
                .tasks
                .iter()
                .find(|t| &t.id == id)
        }) {
            body = body.child(format!(
                "Shared requests {}/{} · reported USD {} · {}",
                budget.attempts(),
                budget.max_attempts,
                budget
                    .spent()
                    .map(|v| format!("{:.6}", v as f64 / 1_000_000.))
                    .unwrap_or("unknown".into()),
                budget.stopped.as_deref().unwrap_or("approved")
            ));
        }
        for (i, side) in run.sides.iter().enumerate() {
            let project = side.project.clone();
            let session = side.session.clone();
            let select = run.id.clone();
            body=body.child(format!("{} · {} / requested {} / observed {} · {} · {:?}",if i==0{"A"}else{"B"},side.choice.harness,side.choice.model,side.actual_model.as_deref().unwrap_or("unknown"),side.status,side.error))
            .child(Button::new(format!("compare-open-{}-{i}",run.id)).ghost().small().label("Open side project/session").on_click(cx.listener(move|this,_,_,cx|{if let Some(index)=this.projects.iter().position(|p|p.id==project){this.select_project_internal(index,false,cx);if this.project().id==project&&let Some(index)=this.sessions.iter().position(|s|s.id==session){let auto=this.auto_connect;this.auto_connect=false;this.select_session(index,cx);this.auto_connect=auto;}}})))
            .child(Button::new(format!("compare-prefer-{}-{i}",run.id)).ghost().small().label(if i==0{"Prefer A (no merge)"}else{"Prefer B (no merge)"}).on_click(cx.listener(move|this,_,w,cx|{let target=select.clone();let receiver=w.prompt(PromptLevel::Warning,"Record preferred solution?",Some("Records a local human decision only. No files are copied, merged, committed, pushed or deleted. Inspect diffs and test evidence independently."),&["Cancel","Record preference"],cx);let receiver=this.overlay_backdrop.track_native(receiver,cx);cx.spawn(async move|this,cx|{if receiver.await!=Ok(1){return;}let _=this.update(cx,|this,cx|{if let Some(r)=this.comparisons.runs.iter_mut().find(|r|r.id==target){r.preferred=Some(i);if let Err(e)=this.save_comparisons(){this.file_notice(e,cx);}}cx.notify();});}).detach();})));
        }
        if run.reports.is_empty() {
            body = body.child("Checks not run/unknown; no passing result assumed.");
        }
        for (side, report) in &run.reports {
            body = body
                .child(format!(
                    "{} check · {} · exit {:?} · argv {:?} · {}ms (may predate later edits)",
                    if *side == 0 { "A" } else { "B" },
                    report.result,
                    report.exit_code,
                    report.argv,
                    report.elapsed_ms
                ))
                .child(div().font_family("Menlo").text_size(px(11.)).child(
                    dunlin_workspace::checks::display_text(
                        &format!("{}\n{}", report.stdout, report.stderr),
                        16_000,
                    ),
                ));
        }
    }
    if let Some((id, previews)) = &s.preview {
        body = body.child(format!(
            "Observed A/B diff snapshot for {id}; first 20 changed files per side, bounded text"
        ));
        for (i, preview) in previews.iter().enumerate() {
            body = body.child(if i == 0 { "A" } else { "B" }).child(
                div()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(preview.clone()),
            );
        }
    }
    if let Some(e) = s.error.as_ref().or(this.comparisons.error.as_ref()) {
        body = body.child(div().text_color(p.red).child(e.clone()));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.bg)
        .text_color(p.fg)
        .child(
            div()
                .p(px(10.))
                .flex_shrink_0()
                .flex()
                .gap(px(8.))
                .child("Compare solutions")
                .child(
                    Button::new("compare-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.compare.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("compare-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}
