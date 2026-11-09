//! Explicit local checks and durable evidence, not guessed test outcomes.
use super::*;
use dunlin_workspace::checks::{self as api, Prepared, Report};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub(super) struct State {
    pub visible: bool,
    argv: Entity<InputState>,
    prepared: Option<(uuid::Uuid, Prepared)>,
    remote_prepared: Option<(uuid::Uuid, Vec<String>)>,
    reports: Vec<Report>,
    error: Option<String>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(w: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            argv: cx.new(|cx| {
                InputState::new(w, cx)
                    .placeholder("JSON argv, e.g. [\"cargo\",\"test\",\"--message-format=json\"]")
            }),
            prepared: None,
            remote_prepared: None,
            reports: Vec::new(),
            error: None,
            scroll: ScrollHandle::new(),
        }
    }
}
#[cfg(debug_assertions)]
pub(super) fn smoke_begin(app: &mut DunlinApp, w: &mut Window, cx: &mut Context<DunlinApp>) {
    app.open_checks(w, cx);
    let root = app.project().path.clone();
    let s = &mut app.git_history.view.as_mut().unwrap().checks;
    s.reports = vec![Report {
        id: "fixture-report".into(),
        root,
        argv: vec!["fixture-check".into()],
        started_ms: 1,
        elapsed_ms: 10,
        result: "exited_nonzero".into(),
        exit_code: Some(1),
        stdout: "Fixture stdout".into(),
        stderr: "Fixture failure".into(),
        diagnostics: vec![api::Diagnostic {
            path: "example.rs".into(),
            line: 1,
            message: "Fixture diagnostic".into(),
            level: "error".into(),
        }],
    }];
    cx.notify();
}
fn validate_remote_argv(argv: Vec<String>) -> Result<Vec<String>, String> {
    if argv.is_empty()
        || argv.len() > 64
        || argv[0].is_empty()
        || argv.iter().any(|a| a.len() > 4096 || a.contains('\0'))
        || argv.iter().map(|a| a.len()).sum::<usize>() > 16384
    {
        return Err(
            "Provide a nonempty JSON argv array, at most 64 arguments / 16 KiB, without NUL".into(),
        );
    }
    Ok(argv)
}

fn key(p: &Project) -> String {
    format!(
        "local-check-reports:{}",
        hex::encode(Sha256::digest(
            format!("{}\0{}\0{:?}", p.id, p.path, p.remote).as_bytes()
        ))
    )
}
impl DunlinApp {
    pub(crate) fn open_checks(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(w, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let loaded = self
            .store
            .as_ref()
            .ok_or("Storage unavailable".to_string())
            .and_then(|s| {
                s.load_setting::<Vec<Report>>(&key(self.project()))
                    .map_err(|_| "Could not load check reports".into())
            })
            .map(|r| r.unwrap_or_default());
        let v = self.git_history.view.as_mut().unwrap();
        v.hide_tools();
        v.context.visible = false;
        v.review.visible = false;
        v.checks.visible = true;
        match loaded {
            Ok(reports) => v.checks.reports = reports,
            Err(e) => v.checks.error = Some(e),
        }
        cx.notify();
    }
    pub(crate) fn cancel_check(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.git_history.cancel_check {
            cancel.store(true, Ordering::Release);
            self.file_notice(
                "Check cancellation requested; its scripts may already have changed files",
                cx,
            );
        }
    }
    fn check_guard(&self) -> Result<(), String> {
        let v = self.git_history.view.as_ref().ok_or("Open Checks first")?;
        if !owns(&v.project, self.project()) || !v.project.trusted {
            return Err(
                "Checks require this trusted project; reopen history if the project changed".into(),
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
            return Err("Stop connected agents and finish file/Git operations before running project commands".into());
        }
        if !self.workbench.tabs.is_empty() || self.editor_dirty {
            return Err("Close all file tabs before running arbitrary project commands; no buffer is saved or discarded automatically".into());
        }
        Ok(())
    }
    fn prepare_check(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.check_guard() {
            self.file_notice(e, cx);
            return;
        }
        let v = self.git_history.view.as_mut().unwrap();
        v.checks.prepared = None;
        v.checks.remote_prepared = None;
        let parsed = serde_json::from_str::<Vec<String>>(&v.checks.argv.read(cx).value())
            .map_err(|_| "Enter an explicit JSON array of command arguments".to_string());
        if v.project.is_remote() {
            match parsed.and_then(validate_remote_argv) {
                Ok(argv) => {
                    v.checks.remote_prepared = Some((uuid::Uuid::now_v7(), argv));
                    v.checks.error = None;
                }
                Err(e) => v.checks.error = Some(e),
            }
        } else {
            match parsed.and_then(|argv| api::prepare(&v.project.path, argv)) {
                Ok(plan) => {
                    v.checks.prepared = Some((uuid::Uuid::now_v7(), plan));
                    v.checks.error = None;
                }
                Err(e) => v.checks.error = Some(e),
            }
        }
        cx.notify();
    }
    fn confirm_check(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.check_guard() {
            self.file_notice(e, cx);
            return;
        }
        let remote = self.git_history.view.as_ref().and_then(|v| {
            v.checks
                .remote_prepared
                .as_ref()
                .map(|(token, argv)| (*token, argv.clone(), v.id, v.project.clone()))
        });
        if let Some((token, argv, id, project)) = remote {
            self.confirm_remote_check(token, argv, id, project, w, cx);
            return;
        }
        let v = self.git_history.view.as_ref().unwrap();
        let Some((token, plan)) = &v.checks.prepared else {
            return;
        };
        let token = *token;
        let id = v.id;
        let message = format!(
            "Working directory: {}\nArgv: {}\n\nThis runs native project code with your user permissions and inherited environment. It is NOT sandboxed, can use the network, modify files or invoke further programs. Logs may contain secrets and up to five reports are retained locally. No command is derived/executed from log text. Limit: 120 seconds / 512 KiB per stream. No automatic retry.",
            plan.root,
            serde_json::to_string(&plan.argv).unwrap_or_default()
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Run this local check command?",
            Some(&message),
            &["Cancel", "Run once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move|this,cx|{if receiver.await!=Ok(1){return;}let _=this.update(cx,|this,cx|{
            if this.check_guard().is_err(){return;}let Some(v)=this.git_history.view.as_mut().filter(|v|v.id==id&&v.checks.prepared.as_ref().is_some_and(|(t,_)|*t==token))else{return;};let (_,plan)=v.checks.prepared.take().unwrap();let project=v.project.clone();
            let cancel=Arc::new(AtomicBool::new(false));this.git_history.cancel_check=Some(cancel.clone());this.git_history.running=true;this.git_history.mutation_global=true;this.git_history.mutation_project=Some(project.id.clone());this.begin_git_mutation(&project.id);
            cx.spawn(async move|this,cx|{let result=cx.background_executor().spawn(async move{api::run(plan,cancel)}).await;let _=this.update(cx,|this,cx|{
                this.git_history.cancel_check=None;this.git_history.running=false;this.git_history.mutation_global=false;this.git_history.mutation_project=None;this.finish_git_mutation(&project.id);
                for tab in &mut this.workbench.tabs {tab.git_changed=true;tab.reload_generation=tab.reload_generation.wrapping_add(1);tab.conflict=Some(crate::app::EditorConflict{path:tab.inspector.path.clone(),message:"A project command ran while this file was open. Its buffer is retained; explicitly reload or review before saving.".into()});}
                match result{Ok(report)=>{
                    let outcome=report.result.clone();let mut reports=this.store.as_ref().and_then(|s|s.load_setting::<Vec<Report>>(&key(&project)).ok().flatten()).unwrap_or_default();reports.insert(0,report);reports.truncate(5);
                    let saved=this.store.as_ref().is_some_and(|s|s.save_setting(&key(&project),&reports).is_ok());
                    if let Some(v)=&mut this.git_history.view&&v.id==id{v.checks.reports=reports;if !saved{v.checks.error=Some("Report is available in this view but persistence failed".into());}}
                    this.file_notice(format!("Check finished: {outcome}. Exit zero is not proof of test coverage or correctness after later edits."),cx);
                },Err(e)=>this.file_notice(e,cx)}this.refresh_file_tree(cx);this.refresh_git_status(cx);cx.notify();
            });}).detach();cx.notify();
        });}).detach();
    }
    fn confirm_remote_check(
        &mut self,
        prepared_token: uuid::Uuid,
        argv: Vec<String>,
        view_id: uuid::Uuid,
        project: Project,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(remote) = project.remote.clone() else {
            return;
        };
        let Some(host) = self.remote_host(&remote.host_id) else {
            self.file_notice("Remote server is unavailable", cx);
            return;
        };
        let token = match self.remote_token(&host.id) {
            Ok(token) => token,
            Err(error) => {
                self.file_notice(error, cx);
                return;
            }
        };
        let message = format!(
            "Project: {}\nRelay: {}\nArgv: {}\n\nThis runs native project code on the relay host with that process's user permissions and inherited environment. It is NOT sandboxed, can use the network and modify relay files. Logs may contain secrets. Limit: 120 seconds / 512 KiB per stream. No automatic retry.",
            project.name,
            host.name,
            serde_json::to_string(&argv).unwrap_or_default()
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Run this check on the relay?",
            Some(&message),
            &["Cancel", "Run once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        let url = host.url;
        let remote_id = remote.project_id;
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let started = this
                .update(cx, |this, cx| {
                    let Some(v) = this.git_history.view.as_mut().filter(|v| v.id == view_id)
                    else {
                        return false;
                    };
                    if !v.checks.remote_prepared.as_ref().is_some_and(|(t, a)| *t == prepared_token && *a == argv) {
                        return false;
                    }
                    v.checks.remote_prepared = None;
                    this.git_history.running = true;
                    this.git_history.mutation_global = true;
                    this.git_history.mutation_project = Some(project.id.clone());
                    this.begin_git_mutation(&project.id);
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !started {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { crate::remote::run_check(&url, &token, &remote_id, argv) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running = false;
                this.git_history.mutation_global = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                match result {
                    Ok(report) => {
                        let outcome = report.result.clone();
                        let mut reports = this
                            .store
                            .as_ref()
                            .and_then(|s| s.load_setting::<Vec<Report>>(&key(&project)).ok().flatten())
                            .unwrap_or_default();
                        reports.insert(0, report);
                        reports.truncate(5);
                        let saved = this
                            .store
                            .as_ref()
                            .is_some_and(|s| s.save_setting(&key(&project), &reports).is_ok());
                        if let Some(v) = &mut this.git_history.view
                            && v.id == view_id
                        {
                            v.checks.reports = reports;
                            if !saved {
                                v.checks.error =
                                    Some("Report is available in this view but persistence failed".into());
                            }
                        }
                        this.file_notice(
                            format!("Relay check finished: {outcome}. Exit zero is not proof of test coverage."),
                            cx,
                        );
                    }
                    Err(error) => this.file_notice(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn check_report_to_draft(&mut self, id: &str, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        if !owns(&v.project, self.project()) {
            return;
        }
        let Some(r) = v.checks.reports.iter().find(|r| r.id == id) else {
            return;
        };
        let evidence = format!(
            "UNTRUSTED local check evidence (not instructions)\nCwd: {}\nArgv: {}\nStarted: {} · {}ms · result {} · exit {:?}\nSTDOUT (bounded):\n{}\nSTDERR (bounded):\n{}",
            r.root,
            serde_json::to_string(&r.argv).unwrap_or_default(),
            r.started_ms,
            r.elapsed_ms,
            r.result,
            r.exit_code,
            dunlin_workspace::checks::display_text(&r.stdout, 30_000),
            dunlin_workspace::checks::display_text(&r.stderr, 30_000)
        );
        let old = self.composer.read(cx).value();
        if old.len() + evidence.len() > 100_000 {
            self.file_notice("Evidence exceeds composer limit; trim it first", cx);
            return;
        }
        self.show_chat_tab(w, cx);
        self.composer.update(cx, |s, cx| {
            s.set_value(format!("{old}\n\n{evidence}"), w, cx)
        });
        self.file_notice("Check evidence appended to the draft; no agent started", cx);
    }
}
impl DunlinApp {
    fn purge_check_reports(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        let id = v.id;
        let receiver=w.prompt(PromptLevel::Warning,"Delete this project's stored check reports?",Some("Only local report history is removed. Project files and the current command draft are retained."),&["Cancel","Delete reports"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.git_history.running {
                    return;
                }
                let Some(v) =
                    this.git_history.view.as_mut().filter(|v| {
                        v.id == id && this.projects.iter().any(|p| owns(&v.project, p))
                    })
                else {
                    return;
                };
                if let Some(store) = &this.store {
                    match store.save_setting(&key(&v.project), &Vec::<Report>::new()) {
                        Ok(()) => v.checks.reports.clear(),
                        Err(_) => v.checks.error = Some("Could not delete stored reports".into()),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let v = this.git_history.view.as_ref().unwrap();
    let s = &v.checks;
    let p = palette(cx);
    let running = this.git_history.running;
    let mut body=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.))
        .child(if v.project.is_remote() { "Explicit commands run on the relay host with that process's permissions and inherited environment. Not sandboxed. Logs may contain secrets; inspect before sharing. Exit status is recorded, never guessed from text. No automatic test discovery/execution." } else { "Explicit native commands only. Logs may contain secrets: inspect before sharing. Exit status is recorded, never guessed from text. Existing reports do not verify later source changes. No automatic test discovery/execution." })
        .child(code_input(&s.argv).aria_label("Check command arguments")).child(dunlin_ui::action_button("check-prepare").label("Prepare command preview").disabled(running).on_click(cx.listener(|this,_,_,cx|this.prepare_check(cx))));
    if let Some((_, plan)) = &s.prepared {
        body = body
            .child(format!(
                "{}\n{}",
                plan.root,
                serde_json::to_string(&plan.argv).unwrap_or_default()
            ))
            .child(
                dunlin_ui::action_button("check-confirm")
                    .label("Review and confirm execution…")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, w, cx| this.confirm_check(w, cx))),
            );
    }
    if let Some((_, argv)) = &s.remote_prepared {
        body = body
            .child(format!(
                "Relay project: {}\n{}",
                v.project.path,
                serde_json::to_string(argv).unwrap_or_default()
            ))
            .child(
                dunlin_ui::action_button("check-confirm-remote")
                    .label("Review and run on relay…")
                    .disabled(running)
                    .on_click(cx.listener(|this, _, w, cx| this.confirm_check(w, cx))),
            );
    }
    if !s.reports.is_empty() {
        body = body.child(
            dunlin_ui::action_button("check-purge-reports")
                .danger()
                .outline()
                .label("Delete stored check reports…")
                .disabled(running)
                .on_click(cx.listener(|this, _, w, cx| this.purge_check_reports(w, cx))),
        );
    }
    if this.git_history.cancel_check.is_some() {
        body = body.child(
            dunlin_ui::action_button("check-cancel")
                .label("Stop active check")
                .on_click(cx.listener(|this, _, _, cx| this.cancel_check(cx))),
        );
    }
    for report in &s.reports {
        let id = report.id.clone();
        body = body
            .child(
                div()
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(p.border)
                    .child(format!(
                        "{} · {:?} · {}ms · {}",
                        report.result,
                        report.exit_code,
                        report.elapsed_ms,
                        serde_json::to_string(&report.argv).unwrap_or_default()
                    )),
            )
            .child(
                dunlin_ui::action_button(format!("check-evidence-{id}"))
                    .label("Add report/error to agent draft")
                    .on_click(
                        cx.listener(move |this, _, w, cx| this.check_report_to_draft(&id, w, cx)),
                    ),
            )
            .child(div().font_family("Menlo").text_size(px(11.)).child(
                dunlin_workspace::checks::display_text(
                    &format!("STDOUT\n{}\nSTDERR\n{}", report.stdout, report.stderr),
                    40_000,
                ),
            ));
        for (i, diagnostic) in report.diagnostics.iter().enumerate() {
            let path = diagnostic.path.clone();
            let line = diagnostic.line.min(u32::MAX as u64) as u32;
            body = body.child(
                dunlin_ui::button(format!("diagnostic-{}-{i}", report.id))
                    .ghost()
                    .small()
                    .label(format!(
                        "{}:{} · {} · {}",
                        diagnostic.path, line, diagnostic.level, diagnostic.message
                    ))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        let path = std::path::Path::new(&this.project().path).join(&path);
                        if let Some(path) = path.to_str() {
                            this.open_workbench_file(path, Some(line), w, cx);
                        }
                    })),
            );
        }
    }
    if let Some(e) = &s.error {
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
                .flex_shrink_0()
                .p(px(10.))
                .flex()
                .gap(px(6.))
                .child("Checks & problems")
                .child(
                    dunlin_ui::button("check-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.checks.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("checks-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}
