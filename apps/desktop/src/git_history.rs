//! Repository-owned graphical history overlay, independent of the agent graph.
use crate::app::DunlinApp;
use dunlin_domain::Project;
use dunlin_ui::component::Disableable;
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};
use dunlin_workspace::git::history::{self, Commit};

#[derive(Default)]
pub(crate) struct State {
    pub view: Option<View>,
    pub running: bool,
}
impl State {
    pub(crate) fn locks(&self, project: &str) -> bool {
        self.running && self.view.as_ref().is_some_and(|v| v.project.id == project)
    }
}
pub(crate) struct View {
    id: uuid::Uuid,
    project: Project,
    rows: Vec<Commit>,
    selected: Option<usize>,
    name: Entity<InputState>,
    revision: Entity<InputState>,
    error: Option<String>,
    loading: bool,
    limit: usize,
}

/// Each active lane owns an expected ancestor. Edges connect every visible
/// parent, including merge parents; unshown ancestors continue below the limit.
#[derive(Clone, Debug)]
struct LaneRow {
    lane: usize,
    edges: Vec<(usize, usize)>,
    width: usize,
    incoming: usize,
}
fn layout(commits: &[Commit]) -> Vec<LaneRow> {
    let mut lanes: Vec<String> = Vec::new();
    commits
        .iter()
        .map(|commit| {
            let incoming = lanes.len();
            let lane = lanes
                .iter()
                .position(|id| id == &commit.oid)
                .unwrap_or_else(|| {
                    lanes.push(commit.oid.clone());
                    lanes.len() - 1
                });
            let before = lanes.clone();
            lanes.remove(lane);
            for (offset, parent) in commit.parents.iter().enumerate() {
                if !lanes.contains(parent) {
                    lanes.insert((lane + offset).min(lanes.len()), parent.clone());
                }
            }
            let mut edges = Vec::new();
            for (column, id) in before.iter().enumerate() {
                if id == &commit.oid {
                    for parent in &commit.parents {
                        if let Some(to) = lanes.iter().position(|id| id == parent) {
                            edges.push((column, to));
                        }
                    }
                } else if let Some(to) = lanes.iter().position(|next| next == id) {
                    edges.push((column, to));
                }
            }
            LaneRow {
                lane,
                width: before.len().max(lanes.len()),
                edges,
                incoming,
            }
        })
        .collect()
}

impl DunlinApp {
    pub(crate) fn open_git_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        let project = self.project().clone();
        if project.is_remote() {
            self.file_notice(
                "Graphical history currently supports local repositories",
                cx,
            );
            return;
        }
        if !project.trusted {
            self.file_notice("Trust the project before reading Git history", cx);
            return;
        }
        if self.git_history.running {
            return;
        }
        self.branch_ui.dismiss();
        self.project_settings_target = None;
        self.context_menu = None;
        self.session_menu_open = false;
        self.project_menu_open = false;
        self.settings_open = false;
        self.command_open = false;
        self.git_history.view = Some(View {
            id: uuid::Uuid::now_v7(),
            project,
            rows: Vec::new(),
            selected: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("feature/new-branch")),
            revision: cx.new(|cx| {
                InputState::new(window, cx).placeholder("All refs, or enter commit / branch / tag")
            }),
            error: None,
            loading: false,
            limit: 500,
        });
        self.load_git_history(cx);
    }
    fn load_git_history(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        view.loading = true;
        view.rows.clear();
        view.selected = None;
        let (id, path, limit) = (view.id, view.project.path.clone(), view.limit);
        let revision = view.revision.read(cx).value().to_string();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { history::load_from(&path, &revision, limit) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = this.git_history.view.as_mut().filter(|v| v.id == id) {
                    view.loading = false;
                    match result {
                        Ok(rows) => {
                            view.rows = rows;
                            view.selected = (!view.rows.is_empty()).then_some(0);
                            view.error = None;
                        }
                        Err(error) => {
                            view.error = Some(format!("{error}. Check the commit/ref or whether the repository has commits. No fetch was performed."))
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn create_history_branch(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.git_history.view.as_ref() else {
            return;
        };
        let Some(commit) = view.selected.and_then(|i| view.rows.get(i)) else {
            return;
        };
        let project = view.project.clone();
        let id = view.id;
        let oid = commit.oid.clone();
        let name = view.name.read(cx).value().to_string();
        let blocked = self.git_history.running
            || self.branch_ui.running()
            || self.git_mutation_in_flight(&project.id)
            || self.ai_commit_locks_project_id(&project.id)
            || self.explorer.busy
            || !self.projects.iter().any(|p| {
                p.id == project.id && p.path == project.path && p.trusted && !p.is_remote()
            })
            || self.live_agents().iter().any(|(owner, session, _)| {
                owner == &project.id
                    || self
                        .group_contexts
                        .get(session)
                        .is_some_and(|g| g.roots.iter().any(|r| r.id == project.id))
            });
        if blocked {
            if let Some(v) = &mut self.git_history.view {
                v.error = Some("Finish repository operations and agents first; reopen history if the project changed".into());
            }
            cx.notify();
            return;
        }
        self.git_history.running = true;
        self.begin_git_mutation(&project.id);
        cx.spawn(async move |this, cx| {
            let path = project.path.clone();
            let branch = name.clone();
            let result = cx
                .background_executor()
                .spawn(async move { history::create_branch(&path, &branch, &oid) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running = false;
                this.finish_git_mutation(&project.id);
                match result {
                    Ok(()) => {
                        this.repo_summaries.invalidate();
                        this.file_notice(format!("Created {name}; working branch unchanged"), cx);
                        if this.git_history.view.as_ref().is_some_and(|v| v.id == id) {
                            this.load_git_history(cx);
                        }
                    }
                    Err(error) => {
                        if let Some(v) = this.git_history.view.as_mut().filter(|v| v.id == id) {
                            v.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn graph_cell(row: LaneRow, width: f32, color: Hsla) -> AnyElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let paint = |window: &mut Window, x, y, w, h, radius| {
                window.paint_quad(quad(
                    Bounds::new(bounds.origin + point(px(x), px(y)), size(px(w), px(h))),
                    Corners::all(px(radius)),
                    color,
                    Edges::default(),
                    color,
                    BorderStyle::default(),
                ))
            };
            for (from, to) in &row.edges {
                let a = 12. + *from as f32 * 16.;
                let b = 12. + *to as f32 * 16.;
                paint(window, a, 16., 1.5, 8., 0.);
                paint(window, a.min(b), 24., (a - b).abs().max(1.5), 1.5, 0.);
                paint(window, b, 24., 1.5, 8., 0.);
            }
            // Incoming lanes from the preceding row end at this row's center.
            for lane in 0..row.incoming {
                paint(window, 12. + lane as f32 * 16., 0., 1.5, 16., 0.);
            }
            paint(window, 8.5 + row.lane as f32 * 16., 12.5, 8., 8., 4.);
        },
    )
    .w(px(width))
    .h(px(32.))
    .flex_shrink_0()
    .into_any_element()
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(view) = &this.git_history.view else {
        return div().into_any_element();
    };
    let lanes = layout(&view.rows);
    let width = (lanes.iter().map(|r| r.width).max().unwrap_or(1) as f32 * 16. + 24.).max(70.);
    let mut list = div()
        .id("git-history-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .flex()
        .flex_col();
    for (i, (commit, lane)) in view.rows.iter().zip(lanes).enumerate() {
        let selected = view.selected == Some(i);
        let date = chrono::DateTime::from_timestamp(commit.timestamp, 0)
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        list = list.child(
            div()
                .id(("history-commit", i))
                .h(px(32.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .cursor_pointer()
                .when(selected, |s| s.bg(p.blue_soft))
                .hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(v) = &mut this.git_history.view {
                        v.selected = Some(i);
                    }
                    cx.notify();
                }))
                .child(graph_cell(lane, width, p.blue))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(commit.oid[..8].to_string()),
                )
                .child(
                    div()
                        .min_w(px(160.))
                        .flex_1()
                        .text_size(px(12.))
                        .truncate()
                        .child(commit.subject.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.blue)
                        .child(commit.refs.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("{} · {date}", commit.author)),
                ),
        );
    }
    let selected = view.selected.and_then(|i| view.rows.get(i));
    let details = selected
        .map(|c| format!("{} · {}", c.oid, c.subject))
        .unwrap_or_else(|| "Select a commit".into());
    div().id("git-history-overlay").absolute().inset_0().flex().items_center().justify_center().on_click(cx.listener(|this, _, _, cx| { if !this.git_history.running { this.git_history.view = None; cx.notify(); } }))
        .child(div().id("git-history").child(crate::effects::rounded_probe(&this.overlay_backdrop, 12.)).w(px(1000.)).h(px(620.)).rounded(px(12.)).border_1().border_color(p.border_strong).bg(p.card).flex().flex_col().overflow_hidden().on_click(|_, _, cx| cx.stop_propagation())
            .child(div().p(px(12.)).flex().items_center().gap(px(10.)).child(format!("Git history · {}", view.project.name)).child(div().flex_1())
                .child(Button::new("history-more").small().label(format!("Load {} commits", (view.limit + 500).min(2000))).disabled(view.loading || this.git_history.running || view.limit >= 2000).on_click(cx.listener(|this, _, _, cx| { if let Some(v) = &mut this.git_history.view { v.limit = (v.limit + 500).min(2000); } this.load_git_history(cx); })))
                .child(Button::new("history-close").small().label("Close").disabled(this.git_history.running).on_click(cx.listener(|this, _, _, cx| { this.git_history.view = None; cx.notify(); }))))
            .child(div().px(px(12.)).pb(px(8.)).text_size(px(11.)).text_color(p.muted_fg).child(if view.loading { "Loading local history…".to_string() } else { format!("{} commits · local and remote-tracking refs · no automatic fetch · lines below the limit may continue", view.rows.len()) }))
            .child(div().px(px(12.)).pb(px(8.)).flex().gap(px(8.))
                .child(Input::new(&view.revision).w(px(470.)))
                .child(Button::new("history-revision").small().label("Show history").disabled(view.loading || this.git_history.running).on_click(cx.listener(|this, _, _, cx| this.load_git_history(cx)))))
            .child(list).child(div().p(px(12.)).flex().flex_col().gap(px(8.)).border_t_1().border_color(p.border)
                .child(div().text_size(px(11.)).truncate().child(details))
                .child(div().flex().gap(px(8.)).child(Input::new(&view.name).w(px(350.))).child(Button::new("history-create").small().primary().label(if this.git_history.running { "Creating…" } else { "Create branch from selected commit" }).disabled(selected.is_none() || this.git_history.running || view.loading).on_click(cx.listener(|this, _, _, cx| this.create_history_branch(cx)))))
                .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Creates a branch only — no checkout, stash or changes to files/staging."))
                .children(view.error.as_ref().map(|e| div().text_size(px(12.)).text_color(p.red).child(e.clone())))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn layout_connects_both_merge_parents_and_collapses_shared_ancestors() {
        let commit = |id: &str, parents: &[&str]| dunlin_workspace::git::history::Commit {
            oid: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: String::new(),
            timestamp: 0,
            subject: String::new(),
            refs: String::new(),
        };
        let rows = super::layout(&[
            commit("merge", &["a", "b"]),
            commit("a", &["root"]),
            commit("b", &["root"]),
            commit("root", &[]),
        ]);
        assert_eq!(rows[0].edges, vec![(0, 0), (0, 1)]);
        assert_eq!(rows[2].edges.len(), 2);
        assert!(rows[3].edges.is_empty());
    }
}
