//! Static manifest detection and the Run panel.
//!
//! Detection is read-only. Running a candidate requires an explicit native
//! confirmation showing the exact argv and working directory. Commands execute
//! in a local PTY, never through a shell string.
use crate::app::DunlinApp;
use dunlin_terminal::Exit;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Disableable;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::{Icon, Sizable};

use dunlin_ui::*;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_HISTORY: usize = 50;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct RunRecord {
    pub id: String,
    pub candidate_id: String,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub project: String,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub success: Option<bool>,
    pub status: String,
}

impl RunRecord {
    pub fn command_line(&self) -> String {
        let mut parts = vec![shell_quote(&self.program)];
        parts.extend(self.args.iter().map(|arg| shell_quote(arg)));
        parts.join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b':'))
    {
        return value.into();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
pub(crate) struct State {
    pub candidates: Vec<dunlin_workspace::run::RunCandidate>,
    pub warnings: Vec<String>,
    pub history: Vec<RunRecord>,
    pub loading: bool,
    pub error: Option<String>,
    pub checked_project: Option<String>,
    pub checked: Option<Instant>,
}

impl State {
    pub fn new(history: Vec<RunRecord>) -> Self {
        let mut history = history;
        for record in &mut history {
            if record.status == "running" {
                record.status = "interrupted by app restart".into();
                record.ended_ms = Some(now_ms());
                record.success = Some(false);
            }
        }
        history.truncate(MAX_HISTORY);
        Self {
            history,
            ..Default::default()
        }
    }
}

impl DunlinApp {
    pub(crate) fn ensure_run_detection(&mut self, cx: &mut Context<Self>) {
        if self.project().is_remote() {
            self.run.error = Some(
                "Run discovery is local-only; a relay project has no local manifest access.".into(),
            );
            return;
        }
        let path = self.project().path.clone();
        if self.run.loading
            || (self.run.checked_project.as_deref() == Some(path.as_str())
                && self
                    .run
                    .checked
                    .is_some_and(|checked| checked.elapsed() < Duration::from_secs(60)))
        {
            return;
        }
        self.run.loading = true;
        self.run.error = None;
        self.run.checked_project = Some(path.clone());
        let checked_path = path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { dunlin_workspace::run::detect(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.run.loading = false;
                this.run.checked = Some(Instant::now());
                if this.project().path != checked_path {
                    this.run.checked_project = None;
                    cx.notify();
                    return;
                }
                this.run.candidates = result.candidates;
                this.run.warnings = result.warnings;
                this.run.error = None;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn refresh_run_detection(&mut self, cx: &mut Context<Self>) {
        self.run.checked_project = None;
        self.run.checked = None;
        self.ensure_run_detection(cx);
    }

    pub(crate) fn start_run(
        &mut self,
        window: &mut Window,
        candidate: dunlin_workspace::run::RunCandidate,
        cx: &mut Context<Self>,
    ) {
        let project = self.project().clone();
        if project.is_remote() {
            self.set_notice("Run is local-only until the relay exposes a PTY grant", cx);
            return;
        }
        let cwd = if candidate.cwd == "." {
            project.path.clone()
        } else {
            format!("{}/{}", project.path.trim_end_matches('/'), candidate.cwd)
        };
        let environment = self
            .resolved_project_settings(&project)
            .and_then(|settings| settings.process_environment())
            .unwrap_or_default();
        let env_names = environment.keys().cloned().collect::<Vec<_>>();
        let preview = format!(
            "Command:\n{}\n\nWorking directory:\n{}\n\nEnvironment names from project settings (values are not displayed):\n{}\n\nThe command runs in a local PTY with this Mac's permissions. It is not sandboxed. A shell profile may change the environment. No output is sent to a relay or model automatically.",
            candidate.command_line(),
            cwd,
            if env_names.is_empty() {
                "none".into()
            } else {
                env_names.join(", ")
            }
        );
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Run this detected command?",
            Some(&preview),
            &["Cancel", "Run once"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update_in(cx, move |this, window, cx| {
                if this.project().id != project.id || this.project().path != project.path {
                    this.file_notice(
                        "Project changed after confirmation; no command was started",
                        cx,
                    );
                    return;
                }
                let record_id = uuid::Uuid::now_v7().to_string();
                let record = RunRecord {
                    id: record_id.clone(),
                    candidate_id: candidate.id.clone(),
                    label: candidate.label.clone(),
                    program: candidate.program.clone(),
                    args: candidate.args.clone(),
                    cwd: cwd.clone(),
                    project: project.path.clone(),
                    started_ms: now_ms(),
                    ended_ms: None,
                    exit_code: None,
                    signal: None,
                    success: None,
                    status: "running".into(),
                };
                this.run.history.insert(0, record.clone());
                this.run.history.truncate(MAX_HISTORY);
                this.persist_settings(cx);
                match this.open_run_terminal(window, candidate, record.id.clone(), cx) {
                    Ok(()) => this.set_notice("Detected command started in a terminal tab", cx),
                    Err(error) => {
                        if let Some(record) = this
                            .run
                            .history
                            .iter_mut()
                            .find(|record| record.id == record_id)
                        {
                            record.status = format!("failed to start: {error}");
                            record.ended_ms = Some(now_ms());
                        }
                        this.run.error = Some(error.clone());
                        this.file_notice(error, cx);
                        this.persist_settings(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn rerun_record(
        &mut self,
        window: &mut Window,
        record: RunRecord,
        cx: &mut Context<Self>,
    ) {
        if record.project != self.project().path {
            self.set_notice(
                "This run belongs to another project; open that project before rerunning",
                cx,
            );
            return;
        }
        let relative = record
            .cwd
            .strip_prefix(self.project().path.trim_end_matches('/'))
            .unwrap_or("")
            .trim_start_matches('/');
        let candidate = dunlin_workspace::run::RunCandidate {
            id: record.candidate_id,
            label: record.label,
            program: record.program,
            args: record.args,
            cwd: if relative.is_empty() {
                ".".into()
            } else {
                relative.into()
            },
            source: "run history".into(),
            kind: "history".into(),
        };
        self.start_run(window, candidate, cx);
    }

    pub(crate) fn stop_run(&mut self, record_id: &str, cx: &mut Context<Self>) {
        let tab = self
            .terminal
            .tabs
            .iter()
            .find(|tab| matches!(&tab.origin, crate::terminal::Origin::Run { record } if record == record_id))
            .map(|tab| tab.id.clone());
        if let Some(tab) = tab {
            self.close_terminal_tab(&tab, cx);
        } else if self
            .run
            .history
            .iter()
            .any(|record| record.id == record_id && record.status == "running")
        {
            self.finish_run(record_id, None, cx);
            cx.notify();
        }
    }

    pub(crate) fn finish_run(
        &mut self,
        record_id: &str,
        exit: Option<Exit>,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.run.history.iter_mut().find(|r| r.id == record_id) else {
            return;
        };
        record.ended_ms = Some(now_ms());
        match exit {
            Some(exit) => {
                record.exit_code = exit.code.map(|code| code as i32);
                record.signal = exit.signal.clone();
                record.success = Some(exit.success);
                record.status = if exit.success {
                    "succeeded".into()
                } else {
                    exit.signal
                        .as_deref()
                        .map(|signal| format!("terminated ({signal})"))
                        .unwrap_or_else(|| {
                            format!(
                                "exited {}",
                                exit.code
                                    .map(|code| code.to_string())
                                    .unwrap_or_else(|| "without a code".into())
                            )
                        })
                };
            }
            None => {
                record.status = "terminal closed by user".into();
                record.success = Some(false);
            }
        }
        self.persist_settings(cx);
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    this.ensure_run_detection(cx);
    let p = palette(cx);
    let remote = this.project().is_remote();
    let toolbar = div()
        .h(px(38.))
        .flex_shrink_0()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .bg(p.subtle)
        .child(
            Icon::new(IconName::SquareTerminal)
                .size(px(14.))
                .text_color(p.teal),
        )
        .child(
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Run"),
        )
        .child(
            div()
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child(this.project().path.clone()),
        )
        .child(div().flex_1())
        .child(
            Button::new("run-refresh")
                .ghost()
                .small()
                .label(if this.run.loading {
                    "Detecting…"
                } else {
                    "Refresh"
                })
                .disabled(remote || this.run.loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh_run_detection(cx))),
        );
    let mut body = div()
        .id("run-scroll")
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .overflow_y_scroll()
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(10.));
    if remote {
        body = body.child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.amber.opacity(0.4))
                .bg(p.amber_soft)
                .p(px(10.))
                .text_size(px(11.5))
                .text_color(p.amber)
                .child("Run detection reads manifests from the local project only. A relay project does not fall back to this Mac or an unapproved server path."),
        );
    } else if let Some(error) = &this.run.error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    if !remote {
        let mut groups: std::collections::BTreeMap<
            &str,
            Vec<&dunlin_workspace::run::RunCandidate>,
        > = std::collections::BTreeMap::new();
        for candidate in &this.run.candidates {
            groups
                .entry(dunlin_workspace::run::kind_label(&candidate.kind))
                .or_default()
                .push(candidate);
        }
        if this.run.loading && this.run.candidates.is_empty() {
            body = body.child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted_fg)
                    .child("Scanning manifests and script folders…"),
            );
        } else if this.run.candidates.is_empty() {
            body = body.child(
                div()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.fg)
                            .child("No runnable commands detected."),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child("Supported manifests: package.json, Cargo.toml, Makefile, justfile, pyproject.toml, go.mod, deno.json, composer.json, Rakefile, mix.exs, gradlew/mvnw and scripts/*.sh|py|js|rb|pl."),
                    ),
            );
        }
        for (kind, candidates) in groups {
            body = body
                .child(
                    div()
                        .pt(px(4.))
                        .text_size(px(10.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_fg)
                        .child(kind.to_uppercase()),
                )
                .child(
                    div()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.card)
                        .children(candidates.into_iter().map(|candidate| {
                            let run_candidate = candidate.clone();
                            let copy_candidate = candidate.clone();
                            let command = candidate.command_line();
                            div()
                                .px(px(9.))
                                .py(px(7.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .border_b_1()
                                .border_color(p.border.opacity(0.55))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .flex()
                                        .flex_col()
                                        .gap(px(2.))
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(p.fg)
                                                .child(candidate.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .font_family(dunlin_ui::theme::code_font())
                                                .text_color(p.muted_fg)
                                                .truncate()
                                                .child(format!(
                                                    "{} · {} · cwd {}",
                                                    command, candidate.source, candidate.cwd
                                                )),
                                        ),
                                )
                                .child(
                                    Button::new(format!("run-copy:{}", candidate.id))
                                        .ghost()
                                        .small()
                                        .label("Copy")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy_candidate.command_line(),
                                            ));
                                            this.set_notice("Copied command; nothing was run", cx);
                                        })),
                                )
                                .child(
                                    Button::new(format!("run-start:{}", candidate.id))
                                        .small()
                                        .label("Run…")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.start_run(window, run_candidate.clone(), cx)
                                        })),
                                )
                                .into_any_element()
                        })),
                );
        }
        if !this.run.warnings.is_empty() {
            body = body.child(
                div()
                    .pt(px(4.))
                    .text_size(px(10.5))
                    .text_color(p.amber)
                    .child(this.run.warnings.join(" · ")),
            );
        }
    }
    body = body.child(
        div()
            .pt(px(6.))
            .text_size(px(10.5))
            .font_family(dunlin_ui::theme::code_font())
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.muted_fg)
            .child("RECENT RUNS"),
    );
    if this.run.history.is_empty() {
        body = body.child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("No recorded runs. History is local and bounded to 50 entries."),
        );
    }
    for record in this.run.history.iter().take(20) {
        let rerun = record.clone();
        let stop = record.id.clone();
        let running = record.status == "running";
        let state_color = match record.status.as_str() {
            "running" => p.blue,
            "succeeded" => p.green,
            _ => p.red,
        };
        body = body.child(
            div()
                .rounded(px(7.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .px(px(9.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().size(px(5.)).rounded_full().bg(state_color))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.fg)
                                .truncate()
                                .child(format!("{} · {}", record.label, record.status)),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .truncate()
                                .child(format!(
                                    "{} · exit {} · {}",
                                    record.command_line(),
                                    record
                                        .exit_code
                                        .map(|code| code.to_string())
                                        .unwrap_or_else(|| "unknown".into()),
                                    record.cwd
                                )),
                        ),
                )
                .child(
                    Button::new(format!("run-stop:{}", record.id))
                        .ghost()
                        .small()
                        .label("Stop")
                        .disabled(remote || !running)
                        .on_click(cx.listener(move |this, _, _, cx| this.stop_run(&stop, cx))),
                )
                .child(
                    Button::new(format!("run-rerun:{}", record.id))
                        .ghost()
                        .small()
                        .label("Rerun…")
                        .disabled(remote || running)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.rerun_record(window, rerun.clone(), cx)
                        })),
                ),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .bg(p.bg)
        .child(toolbar)
        .child(body)
        .into_any_element()
}
