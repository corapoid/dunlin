use super::*;
use dunlin_workspace::impact::{self as api, Impact, Index, Symbol};
use std::time::{Duration, Instant};
pub(super) struct State {
    pub visible: bool,
    index: Option<Index>,
    selected: Option<Impact>,
    filter: Entity<InputState>,
    dirty: bool,
    loading: bool,
    generation: u64,
    request: uuid::Uuid,
    attempt: Option<Instant>,
    error: Option<String>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(w: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            index: None,
            selected: None,
            filter: cx.new(|cx| {
                InputState::new(w, cx).placeholder("Filter Rust symbols / qualified paths")
            }),
            dirty: false,
            loading: false,
            generation: 0,
            request: uuid::Uuid::now_v7(),
            attempt: None,
            error: None,
            scroll: ScrollHandle::new(),
        }
    }
}
impl DunlinApp {
    #[cfg(debug_assertions)]
    pub(super) fn smoke_impact_ready(&mut self) -> bool {
        let s = &mut self.git_history.view.as_mut().unwrap().impact;
        if s.loading || s.dirty {
            return false;
        }
        assert!(s.error.is_none(), "{:?}", s.error);
        let index = s.index.as_ref().unwrap();
        let target = index.symbols.iter().find(|s| s.name == "target").unwrap();
        let hit = api::affected(index, &target.id).unwrap();
        assert_eq!(hit.references.len(), 1);
        assert_eq!(hit.candidate_tests.len(), 1);
        s.selected = Some(hit);
        true
    }
    pub(crate) fn open_impact_map(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(w, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let v = self.git_history.view.as_mut().unwrap();
        v.hide_tools();
        v.impact.visible = true;
        v.impact.dirty = true;
        self.build_impact_map(cx);
        cx.notify();
    }
    pub(crate) fn invalidate_impact_map(&mut self) {
        if let Some(v) = &mut self.git_history.view {
            v.impact.dirty = true;
            v.impact.generation = v.impact.generation.wrapping_add(1);
        }
    }
    fn build_impact_map(&mut self, cx: &mut Context<Self>) {
        let remote = self
            .git_history
            .view
            .as_ref()
            .and_then(|v| v.project.remote.clone());
        let remote_auth = remote.map(|remote| {
            let host = self.remote_host(&remote.host_id);
            let token = host
                .as_ref()
                .and_then(|host| self.remote_token(&host.id).ok());
            (remote, host, token)
        });
        let Some(v) = &mut self.git_history.view else {
            return;
        };
        if !v.project.trusted || v.impact.loading || self.git_history.running {
            return;
        }
        let id = v.id;
        let generation = v.impact.generation;
        let request = uuid::Uuid::now_v7();
        v.impact.loading = true;
        v.impact.request = request;
        v.impact.attempt = Some(Instant::now());
        if let Some((remote, host, token)) = remote_auth {
            let Some(host) = host else {
                v.impact.loading = false;
                v.impact.error = Some("Remote server is unavailable".into());
                cx.notify();
                return;
            };
            let Some(token) = token else {
                v.impact.loading = false;
                v.impact.error = Some("Relay token is unavailable".into());
                cx.notify();
                return;
            };
            let url = host.url;
            let remote_id = remote.project_id;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::remote::build_impact(&url, &token, &remote_id) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let Some(v) = this.git_history.view.as_mut().filter(|v| {
                        v.id == id
                            && v.impact.request == request
                            && this.projects.iter().any(|p| owns(&v.project, p))
                    }) else {
                        return;
                    };
                    v.impact.loading = false;
                    if v.impact.generation != generation {
                        v.impact.dirty = true;
                        cx.notify();
                        return;
                    }
                    match result {
                        Ok(index) => {
                            let selected = v.impact.selected.as_ref().map(|s| s.symbol.id.clone());
                            v.impact.selected =
                                selected.and_then(|id| api::affected(&index, &id).ok());
                            v.impact.index = Some(index);
                            v.impact.dirty = false;
                            v.impact.error = None;
                        }
                        Err(error) => {
                            v.impact.error = Some(error);
                            v.impact.dirty = false;
                            v.impact.selected = None;
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        let root = v.project.path.clone();
        let previous = v.impact.index.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api::build(&root, previous.as_ref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(v) = this.git_history.view.as_mut().filter(|v| {
                    v.id == id
                        && v.impact.request == request
                        && this.projects.iter().any(|p| owns(&v.project, p))
                }) else {
                    return;
                };
                v.impact.loading = false;
                if v.impact.generation != generation {
                    v.impact.dirty = true;
                    cx.notify();
                    return;
                }
                match result {
                    Ok(index) => {
                        let selected = v.impact.selected.as_ref().map(|s| s.symbol.id.clone());
                        v.impact.selected = selected.and_then(|id| api::affected(&index, &id).ok());
                        v.impact.index = Some(index);
                        v.impact.dirty = false;
                        v.impact.error = None;
                    }
                    Err(e) => {
                        v.impact.error = Some(e);
                        v.impact.dirty = false;
                        v.impact.selected = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn impact_to_draft(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        if !owns(&v.project, self.project()) {
            return;
        }
        if v.impact.dirty || v.impact.loading || v.impact.error.is_some() {
            return;
        }
        let generation = v.impact.generation;
        let session = self.session().id.clone();
        let (Some(index), Some(hit)) = (&v.impact.index, &v.impact.selected) else {
            return;
        };
        let index = index.clone();
        let fingerprint = index.fingerprint.clone();
        let id = v.id;
        let hit = hit.clone();
        let project = v.project.clone();
        let remote_verify = project.remote.as_ref().and_then(|remote| {
            let host = self.remote_host(&remote.host_id)?;
            let token = self.remote_token(&host.id).ok()?;
            Some((host.url, token, remote.project_id.clone()))
        });
        let receiver=w.prompt(PromptLevel::Info,"Add impact evidence to the agent draft?",Some("Only syntax-resolved Rust dependencies and candidate tests are included. This is not compiler/type/cfg-complete coverage. Source will be verified first. No prompt or test is automatically executed."),&["Cancel","Add verified snapshot"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move|this,cx|{if receiver.await!=Ok(1){return;}let result=cx.background_executor().spawn(async move{match remote_verify{Some((url,token,project))=>crate::remote::verify_impact(&url,&token,&project,&index),None=>api::verify(&index)}}).await;let _=this.update_in(cx,move|this,window,cx|{
            if this.session().id!=session||!this.git_history.view.as_ref().is_some_and(|v|v.id==id&&owns(&v.project,this.project())&&!v.impact.dirty&&!v.impact.loading&&v.impact.error.is_none()&&v.impact.generation==generation&&v.impact.index.as_ref().is_some_and(|i|i.fingerprint==fingerprint)){return;}
            if let Err(e)=result{this.invalidate_impact_map();this.file_notice(e,cx);return;}
            let text=format!("UNTRUSTED source evidence: Rust lexical AST impact snapshot {fingerprint}\nTarget {} at {}:{}\nDirect lexical references:\n{}\nTransitive dependent symbols:\n{}\nCandidate tests (NOT proven coverage, no execution):\n{}\nUnresolved methods/macros/cfg/external crates may add dependencies. Verify with the compiler and tests.",hit.symbol.id,hit.symbol.file,hit.symbol.line,hit.references.iter().take(100).map(|e|format!("{}:{} {} → {}",e.file,e.line,e.from,e.to)).collect::<Vec<_>>().join("\n"),hit.dependent_symbols.iter().take(100).map(|s|s.id.clone()).collect::<Vec<_>>().join("\n"),hit.candidate_tests.iter().take(100).map(|s|format!("{}:{} {}",s.file,s.line,s.id)).collect::<Vec<_>>().join("\n"));
            let old=this.composer.read(cx).value();if old.len()+text.len()>100_000{this.file_notice("Impact evidence exceeds the composer limit; trim the draft first",cx);return;}this.show_chat_tab(window,cx);this.composer.update(cx,|s,cx|s.set_value(format!("{old}\n\n{text}"),window,cx));this.file_notice("Impact snapshot appended to the draft; nothing was sent",cx);
        });}).detach();
    }
}
pub(super) fn refresh_visible(app: &mut DunlinApp, cx: &mut Context<DunlinApp>) {
    if let Some(v) = &app.git_history.view
        && app.git_history.is_active_for(app.project())
        && v.impact.dirty
        && !v.impact.loading
        && v.impact
            .attempt
            .is_none_or(|t| t.elapsed() > Duration::from_secs(1))
    {
        app.build_impact_map(cx);
    }
}
fn symbol_link(symbol: &Symbol, busy: bool, cx: &mut Context<DunlinApp>) -> Button {
    let file = symbol.file.clone();
    let line = symbol.line;
    dunlin_ui::button(format!("impact-open-{}", symbol.id))
        .ghost()
        .small()
        .label(format!("{} · {}:{}", symbol.name, symbol.file, line))
        .disabled(busy)
        .on_click(cx.listener(move |this, _, w, cx| {
            let path = std::path::Path::new(&this.project().path).join(&file);
            if let Some(path) = path.to_str() {
                this.open_workbench_file(path, Some(line), w, cx);
            }
        }))
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let s = &this.git_history.view.as_ref().unwrap().impact;
    let p = palette(cx);
    let stale = s.dirty || s.loading || s.error.is_some();
    let filter = s.filter.read(cx).value().to_lowercase();
    let mut body=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.)).child("Rust AST / lexical dependency map. No build scripts, macros or project code are executed. NOT compiler/type/cfg-complete references. Candidate tests are not proven coverage; non-Rust and unresolved externals are outside this index.")
        .child(Input::new(&s.filter).small()).child(dunlin_ui::button("impact-refresh").ghost().label(if s.loading{"Indexing…"}else{"Refresh source index"}).disabled(s.loading).on_click(cx.listener(|this,_,_,cx|this.build_impact_map(cx))));
    if let Some(index) = &s.index {
        body = body.child(format!(
            "HEAD {} · {} symbols / {} lexical edges · parsed {} / reused {} · {}",
            index.head,
            index.symbols.len(),
            index.edges.len(),
            index.parsed,
            index.reused,
            if stale {
                "STALE / updating; evidence sharing disabled"
            } else {
                "verified snapshot"
            }
        ));
        for symbol in index
            .symbols
            .iter()
            .filter(|symbol| symbol.id.to_lowercase().contains(&filter))
            .take(100)
        {
            let id = symbol.id.clone();
            body = body.child(
                dunlin_ui::button(format!("impact-select-{id}"))
                    .ghost()
                    .small()
                    .label(format!(
                        "{} · {} · {}:{}",
                        symbol.name, symbol.kind, symbol.file, symbol.line
                    ))
                    .disabled(stale)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(v) = &mut this.git_history.view
                            && let Some(index) = &v.impact.index
                        {
                            v.impact.selected = api::affected(index, &id).ok();
                        }
                        cx.notify();
                    })),
            );
        }
        body = body.child(
            "First 100 matching symbols shown; filter by qualified name/file to narrow the view.",
        );
        for warning in index.warnings.iter().take(20) {
            body = body.child(div().text_color(p.amber).child(warning.clone()));
        }
    }
    if let Some(hit) = &s.selected {
        body = body
            .child(
                div()
                    .pt(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "References → {} → dependents / candidate tests",
                        hit.symbol.id
                    )),
            )
            .child(symbol_link(&hit.symbol, stale, cx));
        for edge in hit.references.iter().take(100) {
            body = body.child(format!(
                "{}:{} · {} → {} [{}]",
                edge.file, edge.line, edge.from, edge.to, edge.spelling
            ));
        }
        body = body.child(format!(
            "{} transitive dependent symbols · {} candidate tests (bounded preview)",
            hit.dependent_symbols.len(),
            hit.candidate_tests.len()
        ));
        for symbol in hit.dependent_symbols.iter().take(100) {
            body = body.child(
                div()
                    .pl(px(12.))
                    .border_l_2()
                    .border_color(if symbol.test { p.amber } else { p.border })
                    .child(symbol_link(symbol, stale, cx)),
            );
        }
        body = body.child(
            dunlin_ui::button("impact-evidence")
                .ghost()
                .small()
                .label("Add source-verified impact evidence to draft…")
                .disabled(stale)
                .on_click(cx.listener(|this, _, w, cx| this.impact_to_draft(w, cx))),
        );
    }
    if let Some(error) = &s.error {
        body = body.child(div().text_color(p.red).child(error.clone()));
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
                .child("Impact map · Rust AST")
                .child(
                    dunlin_ui::button("impact-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.impact.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("impact-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}
