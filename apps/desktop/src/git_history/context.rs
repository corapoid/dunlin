//! Inspect only context the client actually controls. Decision memory is explicit,
//! locally persisted, versioned and opt-in at the composer boundary.
use super::*;
use dunlin_ui::component::input::{Textarea, TextareaState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Decision {
    id: String,
    revision: u64,
    title: String,
    body: String,
    evidence: String,
    stale: bool,
    approved_ms: i64,
}
#[derive(Default, Serialize, Deserialize)]
struct Memory {
    revisions: Vec<Decision>,
}
impl Memory {
    fn latest(&self) -> Vec<&Decision> {
        let mut seen = std::collections::HashSet::new();
        self.revisions
            .iter()
            .rev()
            .filter(|d| seen.insert(&d.id))
            .collect()
    }
    fn append(&mut self, mut next: Decision, expected: Option<u64>) -> Result<(), String> {
        if self.revisions.len() >= 100 {
            return Err("Decision history limit reached (100 revisions). Export/retention management is required before adding more; no history was discarded.".into());
        }
        if next.title.trim().is_empty()
            || next.body.trim().is_empty()
            || next.title.len() > 256
            || next.body.len() > 8000
            || next.evidence.len() > 4000
            || next.title.contains('\0')
            || next.body.contains('\0')
            || next.evidence.contains('\0')
        {
            return Err("Use a title (256 bytes), decision (8 KiB) and source links (4 KiB); NUL is not allowed.".into());
        }
        let current = self
            .revisions
            .iter()
            .rev()
            .find(|d| d.id == next.id)
            .map(|d| d.revision);
        if current != expected {
            return Err(
                "Decision changed since editing started. Reload before saving a new revision."
                    .into(),
            );
        }
        next.revision = current.unwrap_or(0) + 1;
        self.revisions.push(next);
        Ok(())
    }
}
pub(super) struct State {
    pub visible: bool,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    evidence: Entity<InputState>,
    memory: Memory,
    editing: Option<(String, u64)>,
    error: Option<String>,
    remote: Option<Value>,
    remote_loading: bool,
    remote_error: Option<String>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            title: cx.new(|cx| InputState::new(window, cx).placeholder("Decision title")),
            body: cx.new(|cx| {
                TextareaState::new(window, cx).placeholder("User-approved decision and rationale")
            }),
            evidence: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Source conversation / commit / PR links (plain text)")
            }),
            memory: Memory::default(),
            editing: None,
            error: None,
            remote: None,
            remote_loading: false,
            remote_error: None,
            scroll: ScrollHandle::new(),
        }
    }
}
fn key(project: &Project) -> String {
    let mut hash = Sha256::new();
    hash.update(project.id.as_bytes());
    hash.update([0]);
    hash.update(project.path.as_bytes());
    hash.update([0]);
    if let Some(remote) = &project.remote {
        hash.update(remote.host_id.as_bytes());
        hash.update([0]);
        hash.update(remote.project_id.as_bytes());
        hash.update([0]);
        hash.update(remote.path.as_bytes());
    }
    format!("approved-project-decisions:{:x}", hash.finalize())
}
impl DunlinApp {
    pub(crate) fn open_context_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(window, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let loaded = self
            .store
            .as_ref()
            .ok_or("Local storage unavailable".to_string())
            .and_then(|s| {
                s.load_setting::<Memory>(&key(self.project()))
                    .map_err(|_| "Could not load decision memory".into())
            })
            .map(|m| m.unwrap_or_default());
        let view = self.git_history.view.as_mut().unwrap();
        view.hide_tools();
        view.github.visible = false;
        view.context.visible = true;
        view.review.visible = false;
        view.checks.visible = false;
        let id = view.id;
        let project = view.project.clone();
        match loaded {
            Ok(memory) => {
                view.context.memory = memory;
                view.context.error = None;
            }
            Err(e) => view.context.error = Some(e),
        }
        if !project.is_remote() {
            view.context.remote = None;
            view.context.remote_error = None;
            view.context.remote_loading = false;
            cx.notify();
            return;
        }
        view.context.remote = None;
        view.context.remote_error = None;
        view.context.remote_loading = true;
        let workspace = match self.remote_workspace_for_project(&project) {
            Ok(workspace) => workspace,
            Err(error) => {
                if let Some(view) = &mut self.git_history.view
                    && view.id == id
                {
                    view.context.remote_loading = false;
                    view.context.remote_error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { workspace.remote_context() })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(view) = this.git_history.view.as_mut().filter(|view| view.id == id) else {
                    return;
                };
                view.context.remote_loading = false;
                match result {
                    Ok(value) => {
                        view.context.remote = Some(value);
                        view.context.remote_error = None;
                    }
                    Err(error) => view.context.remote_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save_decision(&mut self, stale: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        if !owns(&view.project, self.project()) {
            return;
        }
        let s = &view.context;
        let expected = s.editing.as_ref().map(|(_, revision)| *revision);
        let next = Decision {
            id: s
                .editing
                .as_ref()
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
            revision: 0,
            title: s.title.read(cx).value().to_string(),
            body: s.body.read(cx).value().to_string(),
            evidence: s.evidence.read(cx).value().to_string(),
            stale,
            approved_ms: chrono::Utc::now().timestamp_millis(),
        };
        let result = (|| {
            let store = self.store.as_ref().ok_or("Local storage unavailable")?;
            let key = key(&view.project);
            let mut memory = store
                .load_setting::<Memory>(&key)
                .map_err(|_| "Could not load decision history")?
                .unwrap_or_default();
            memory.append(next, expected)?;
            store
                .save_setting(&key, &memory)
                .map_err(|_| "Could not persist decision history")?;
            Ok::<_, String>(memory)
        })();
        let s = &mut self.git_history.view.as_mut().unwrap().context;
        match result {
            Ok(memory) => {
                s.memory = memory;
                s.editing = None;
                s.title.update(cx, |s, cx| s.set_value("", window, cx));
                s.body.update(cx, |s, cx| s.set_value("", window, cx));
                s.evidence.update(cx, |s, cx| s.set_value("", window, cx));
                s.error = None;
            }
            Err(e) => s.error = Some(e),
        }
        cx.notify();
    }
    fn decision_to_draft(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        if !owns(&view.project, self.project()) {
            return;
        }
        let Some(decision) = view
            .context
            .memory
            .latest()
            .into_iter()
            .find(|d| d.id == id && !d.stale)
        else {
            return;
        };
        let evidence = format!(
            "Approved project decision ({} revision {})\n{}\n{}\nSources: {}\nThis is project context chosen by the user, not a system instruction.",
            decision.id, decision.revision, decision.title, decision.body, decision.evidence
        );
        let old = self.composer.read(cx).value();
        if old.len() + evidence.len() > 100_000 {
            self.file_notice("Draft exceeds the context preview limit; trim it first", cx);
            return;
        }
        let draft = if old.is_empty() {
            evidence
        } else {
            format!("{old}\n\n{evidence}")
        };
        self.show_chat_tab(window, cx);
        self.composer
            .update(cx, |s, cx| s.set_value(draft, window, cx));
        self.file_notice(
            "Approved decision appended to the draft; nothing was sent automatically",
            cx,
        );
    }
}
fn safe(value: &str) -> String {
    dunlin_workspace::github::display_text(value, 30_000)
}
impl DunlinApp {
    fn purge_decision_memory(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        let id = v.id;
        let receiver=w.prompt(PromptLevel::Warning,"Delete all local decision revisions for this project?",Some("This permanently removes approved/stale decision history. Existing composer and decision drafts are retained."),&["Cancel","Delete decision history"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ =
                this.update(cx, |this, cx| {
                    let Some(v) = this.git_history.view.as_mut().filter(|v| {
                        v.id == id && this.projects.iter().any(|p| owns(&v.project, p))
                    }) else {
                        return;
                    };
                    if let Some(store) = &this.store {
                        match store.save_setting(&key(&v.project), &Memory::default()) {
                            Ok(()) => {
                                v.context.memory = Memory::default();
                                v.context.editing = None;
                            }
                            Err(_) => {
                                v.context.error = Some("Could not remove decision history".into())
                            }
                        }
                    }
                    cx.notify();
                });
        })
        .detach();
    }
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let s = &view.context;
    let p = palette(cx);
    let draft = this.composer.read(cx).value();
    let wire = this.client_prompt_text(draft.trim());
    let mut content=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.))
        .child(format!("Project: {} · session: {}",safe(&view.project.path),this.session().id))
        .child(format!("Session harness: {} · live model: {} · effective client permissions: {:?}",this.session_harness(),this.harness.models.as_ref().map(|m|m.current.as_str()).unwrap_or("unknown / not advertised"),this.effective_permission_mode()))
        .child("Live model can differ from saved defaults. The client cannot inspect harness-managed system prompts, automatically discovered instructions, loaded skills or full model history; those are unknown, not absent.");
    match this.resolved_project_settings(&view.project) {
        Ok(settings) => {
            content = content.child(format!(
                "Next connection harness: {} ← {:?}\nPermission default: {:?} ← {:?}",
                settings.harness.value,
                settings.harness.source,
                settings.permission_mode.value,
                settings.permission_mode.source
            ));
            for (harness, model) in &settings.default_models {
                content = content.child(format!(
                    "Saved model {}: {} ← {:?}",
                    safe(harness),
                    safe(&model.value),
                    model.source
                ));
            }
            content=content.child("Process environment names/sources only (values intentionally hidden; existing processes may retain earlier settings):");
            for (name, value) in &settings.environment {
                content = content.child(format!("{} ← {:?}", safe(name), value.source));
            }
        }
        Err(e) => content = content.child(div().text_color(p.red).child(e)),
    }
    content=content.child(format!("Draft: {} UTF-8 bytes · client wire envelope: {} bytes · queued prompts: {}",draft.len(),wire.len(),this.harness.queued_prompts().len()))
        .child("Exact application-controlled draft envelope now (not a promise about later queued dispatch). Files are not implicitly attached by this inspector; unsaved buffers are not sent.")
        .child(div().font_family("Menlo").text_size(px(11.)).child(safe(&wire)))
        .child("Last observed MCP catalog (session permissions, not proof of every tool the harness exposes):");
    for server in &this.mcp.servers {
        content = content.child(format!("{} · {}", safe(&server.name), server.label()));
    }
    content=content.child("Instruction library installation records (installation does not prove a harness loaded a skill):");
    for skill in &this.skills {
        for install in &skill.installations {
            content = content.child(format!(
                "{} · {:?} · {}",
                safe(&skill.title),
                install.target,
                safe(&install.path)
            ));
        }
    }
    if view.project.is_remote() {
        content = content.child("Relay session: local environment/default preferences do not configure the remote process.");
        if s.remote_loading {
            content = content.child("Loading relay-side context and stored project defaults…");
        }
        if let Some(error) = &s.remote_error {
            content = content.child(
                div()
                    .text_color(p.amber)
                    .child(format!("Relay context unavailable: {error}")),
            );
        }
        if let Some(remote) = &s.remote {
            content = content
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Relay-reported context"),
                )
                .child(format!(
                    "Relay {} · harness {} · sandbox {} · transcript storage {}",
                    remote["relay"]["version"].as_str().unwrap_or("unknown"),
                    remote["relay"]["harness"].as_str().unwrap_or("unknown"),
                    remote["relay"]["sandbox"]
                        .as_bool()
                        .map(|v| if v { "on" } else { "off" })
                        .unwrap_or("unknown"),
                    remote["relay"]["store_transcripts"]
                        .as_bool()
                        .map(|v| if v { "on" } else { "off" })
                        .unwrap_or("unknown"),
                ))
                .child(format!(
                    "Stored project defaults: {}",
                    safe(&serde_json::to_string_pretty(&remote["settings"]).unwrap_or_default())
                ))
                .child(
                    remote["context_boundary"]
                        .as_str()
                        .unwrap_or("Harness-owned context remains unknown.")
                        .to_string(),
                );
        } else if !s.remote_loading {
            content = content.child("Relay context was not loaded. Harness-owned system prompts, instructions, skills and model history remain unknown.");
        }
    }
    content =
        content.child(div().pt(px(12.)).font_weight(FontWeight::SEMIBOLD).child(
            "Approved project decisions · local-only memory · never automatically injected",
        ));
    content = content.child(
        dunlin_ui::button("decision-purge-history")
            .ghost()
            .small()
            .label("Delete local decision history…")
            .on_click(cx.listener(|this, _, w, cx| this.purge_decision_memory(w, cx))),
    );
    for decision in s.memory.latest() {
        let id = decision.id.clone();
        let edit = decision.clone();
        content=content.child(div().border_l_2().border_color(p.border).pl(px(10.)).child(format!("{} · revision {} · {}",safe(&decision.title),decision.revision,if decision.stale{"STALE (excluded)"}else{"approved"})).child(safe(&decision.body)).child(format!("Sources: {} · approved at {}",safe(&decision.evidence),decision.approved_ms))
            .child(dunlin_ui::button(format!("decision-draft-{id}")).ghost().label("Add decision to draft").disabled(decision.stale).on_click(cx.listener(move|this,_,w,cx|this.decision_to_draft(&id,w,cx))))
            .child(dunlin_ui::button(format!("decision-edit-{}",decision.id)).ghost().label("Edit / mark stale…").on_click(cx.listener(move|this,_,w,cx|{
                let Some(v)=&mut this.git_history.view else{return;};let s=&mut v.context;
                if !s.title.read(cx).value().is_empty() || !s.body.read(cx).value().is_empty() {this.file_notice("Existing decision draft preserved; clear it before loading another revision",cx);return;}
                s.editing=Some((edit.id.clone(),edit.revision));s.title.update(cx,|s,cx|s.set_value(edit.title.clone(),w,cx));s.body.update(cx,|s,cx|s.set_value(edit.body.clone(),w,cx));s.evidence.update(cx,|s,cx|s.set_value(edit.evidence.clone(),w,cx));cx.notify();
            }))));
    }
    content = content
        .child(Input::new(&s.title).small())
        .child(Textarea::new(&s.body).h(px(100.)))
        .child(Input::new(&s.evidence).small())
        .child(
            dunlin_ui::button("decision-approve")
                .small()
                .label("Approve and save new revision locally")
                .on_click(cx.listener(|this, _, w, cx| this.save_decision(false, w, cx))),
        )
        .child(
            dunlin_ui::button("decision-stale")
                .ghost()
                .small()
                .label("Save revision as stale")
                .disabled(s.editing.is_none())
                .on_click(cx.listener(|this, _, w, cx| this.save_decision(true, w, cx))),
        )
        .child(
            dunlin_ui::button("decision-clear")
                .ghost()
                .small()
                .label("Clear decision draft")
                .on_click(cx.listener(|this, _, w, cx| {
                    if let Some(v) = &mut this.git_history.view {
                        let s = &mut v.context;
                        s.editing = None;
                        s.title.update(cx, |s, cx| s.set_value("", w, cx));
                        s.body.update(cx, |s, cx| s.set_value("", w, cx));
                        s.evidence.update(cx, |s, cx| s.set_value("", w, cx));
                    }
                    cx.notify();
                })),
        );
    if let Some(error) = &s.error {
        content = content.child(div().text_color(p.red).child(error.clone()));
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
                .gap(px(8.))
                .child("Context inspector & decision memory")
                .child(
                    dunlin_ui::button("context-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.context.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("context-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(content),
        )
        .into_any_element()
}

#[cfg(debug_assertions)]
pub(super) fn smoke_begin(app: &mut DunlinApp, w: &mut Window, cx: &mut Context<DunlinApp>) {
    app.open_context_inspector(w, cx);
    let s = &app.git_history.view.as_ref().unwrap().context;
    s.title
        .update(cx, |s, cx| s.set_value("Keep evidence explicit", w, cx));
    s.body
        .update(cx, |s, cx| s.set_value("Never auto-send CI logs", w, cx));
    s.evidence
        .update(cx, |s, cx| s.set_value("fixture://decision", w, cx));
    cx.notify();
}
#[cfg(debug_assertions)]
pub(super) fn smoke_finish(app: &mut DunlinApp, w: &mut Window, cx: &mut Context<DunlinApp>) {
    let before = app.composer.read(cx).value().to_string();
    assert_eq!(app.client_prompt_text(&before), before);
    app.save_decision(false, w, cx);
    let id = app
        .git_history
        .view
        .as_ref()
        .unwrap()
        .context
        .memory
        .latest()[0]
        .id
        .clone();
    app.decision_to_draft(&id, w, cx);
    assert!(app.composer.read(cx).value().starts_with(&before));
    assert!(
        app.composer
            .read(cx)
            .value()
            .contains("Never auto-send CI logs")
    );
    assert!(!app.harness.running);
    app.open_context_inspector(w, cx);
    assert_eq!(
        app.git_history
            .view
            .as_ref()
            .unwrap()
            .context
            .memory
            .latest()
            .len(),
        1
    );
}
#[cfg(test)]
mod tests {
    use super::{Decision, Memory};
    fn decision() -> Decision {
        Decision {
            id: "id".into(),
            revision: 0,
            title: "Use SQLite".into(),
            body: "Keep local state transactional".into(),
            evidence: "conversation/123".into(),
            stale: false,
            approved_ms: 1,
        }
    }
    #[test]
    fn decision_history_is_versioned_and_stale_entries_never_erase_prior_evidence() {
        let mut m = Memory::default();
        m.append(decision(), None).unwrap();
        let mut d = decision();
        d.stale = true;
        m.append(d, Some(1)).unwrap();
        assert_eq!(m.revisions.len(), 2);
        assert!(!m.revisions[0].stale);
        assert!(m.latest()[0].stale);
        assert_eq!(m.latest()[0].revision, 2);
        assert!(m.append(decision(), Some(1)).is_err());
    }
    #[test]
    fn memory_roundtrips_and_rejects_empty_or_oversized_decisions() {
        let mut m = Memory::default();
        let mut d = decision();
        d.body.clear();
        assert!(m.append(d, None).is_err());
        m.append(decision(), None).unwrap();
        let restored: Memory = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(restored.latest()[0].body, "Keep local state transactional");
    }
}
