//! Left sidebar: project switcher, explorer, search, source control,
//! session history and the subscription usage footer.

use std::path::Path;

use kestrel_domain::*;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::Icon;
use kestrel_ui::component::input::Input;
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;

use crate::app::{KestrelApp, SettingsTab, status_palette, tone_color, tone_soft};
use crate::harness::HarnessStatus;
use crate::tree::FileEntry;

pub(crate) fn render(
    this: &mut KestrelApp,
    _window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let body = match this.section {
        SidebarSection::Projects => projects_body(this, cx),
        SidebarSection::Files => files_body(this, cx),
        SidebarSection::Search => search_body(this, cx),
        SidebarSection::SourceControl => source_control_body(this, cx),
        SidebarSection::Sessions => sessions_body(this, cx),
    };
    let header = section_header(this, cx);
    let footer = usage_footer(this, cx);
    let menu = this.project_menu_open.then(|| project_menu(this, cx));

    let mut root = div()
        .relative()
        .w(px(278.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(p.sidebar)
        .border_r_1()
        .border_color(p.border)
        .child(project_switcher(this, cx))
        .child(header)
        .child(
            div()
                .id("sidebar-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .child(footer);

    if let Some(menu) = menu {
        root = root.child(menu);
    }
    root.into_any_element()
}

fn project_switcher(this: &KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let project = this.project();
    let tone = tone_color(&p, project.tone);
    let soft = tone_soft(&p, project.tone);
    let monogram = monogram(&project.name);

    div()
        .id("project-switcher")
        .h(px(50.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(|this, _, _, cx| {
            this.project_menu_open = !this.project_menu_open;
            cx.notify();
        }))
        .child(
            div()
                .size(px(28.))
                .flex_shrink_0()
                .rounded(px(8.))
                .bg(soft)
                .border_1()
                .border_color(tone.opacity(0.18))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::BOLD)
                        .font_family("Geist Mono")
                        .text_color(tone)
                        .child(monogram),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.fg)
                        .truncate()
                        .child(project.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(project.path_tail().to_string()),
                ),
        )
        .child(
            div()
                .h(px(19.))
                .px(px(6.))
                .rounded(px(5.))
                .bg(p.muted)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(tone, 4.5))
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(format!("{} live", project.running_sessions)),
                ),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .size(px(13.))
                .text_color(p.muted_fg),
        )
        .into_any_element()
}

fn section_header(this: &KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let (title, count) = match this.section {
        SidebarSection::Projects => ("Projects", Some(this.projects.len())),
        SidebarSection::Files => ("Explorer", None),
        SidebarSection::Search => ("Search", Some(this.search_hits.len())),
        SidebarSection::SourceControl => ("Source control", Some(this.git.changes.len())),
        SidebarSection::Sessions => ("Sessions", Some(this.sessions.len())),
    };

    let right: AnyElement = match this.section {
        SidebarSection::Files => div()
            .id("explorer-actions")
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .id("collapse-all")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.file_tree.collapse_all();
                        cx.notify();
                    }))
                    .child(
                        Icon::new(IconName::FoldVertical)
                            .size(px(12.5))
                            .text_color(p.muted_fg),
                    ),
            )
            .child(
                div()
                    .id("new-file")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_file(window, cx);
                    }))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(13.))
                            .text_color(p.muted_fg),
                    ),
            )
            .into_any_element(),
        SidebarSection::Projects => div()
            .id("add-project")
            .size(px(22.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_folder(cx);
            }))
            .child(
                Icon::new(IconName::Plus)
                    .size(px(13.))
                    .text_color(p.muted_fg),
            )
            .into_any_element(),
        SidebarSection::Sessions => div()
            .id("new-session")
            .h(px(22.))
            .px(px(7.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_session(cx);
            }))
            .child(
                Icon::new(IconName::Plus)
                    .size(px(11.5))
                    .text_color(p.muted_fg),
            )
            .child(div().text_size(px(9.5)).text_color(p.muted_fg).child("New"))
            .into_any_element(),
        _ => div().into_any_element(),
    };

    div()
        .h(px(34.))
        .flex_shrink_0()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(9.))
                .font_family("Geist Mono")
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child(title.to_uppercase()),
        )
        .when_some(count, |this, count| {
            this.child(
                div()
                    .text_size(px(8.5))
                    .font_family("Geist Mono")
                    .text_color(p.muted_fg.opacity(0.7))
                    .child(count.to_string()),
            )
        })
        .child(div().flex_1())
        .child(right)
        .into_any_element()
}

fn projects_body(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let mut list = div().flex().flex_col().gap(px(2.)).px(px(6.)).pb(px(10.));

    for (index, project) in this.projects.iter().enumerate() {
        let active = index == this.selected_project;
        let tone = tone_color(&p, project.tone);
        let soft = tone_soft(&p, project.tone);
        let monogram = monogram(&project.name);
        let name = project.name.clone();
        let tail = project.path_tail().to_string();
        let branch = project.branch.clone().unwrap_or_else(|| "no branch".into());
        let running = project.running_sessions;
        let pinned = project.pinned;

        list = list.child(
            div()
                .id(("project-row", index))
                .h(px(46.))
                .px(px(8.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .when(active, |this| {
                    this.bg(p.card).border_1().border_color(p.border)
                })
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
                .child(
                    div()
                        .size(px(26.))
                        .flex_shrink_0()
                        .rounded(px(7.))
                        .bg(soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .font_weight(FontWeight::BOLD)
                                .text_color(tone)
                                .child(monogram),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.fg)
                                        .truncate()
                                        .child(name),
                                )
                                .when(pinned, |this| {
                                    this.child(
                                        Icon::new(IconName::Pin)
                                            .size(px(9.))
                                            .text_color(p.muted_fg),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg)
                                        .truncate()
                                        .child(branch),
                                )
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(p.border_strong)
                                        .child("·"),
                                )
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .font_family("Geist Mono")
                                        .text_color(p.muted_fg.opacity(0.72))
                                        .truncate()
                                        .child(tail),
                                ),
                        ),
                )
                .when(running > 0, |this| {
                    this.child(
                        div()
                            .h(px(17.))
                            .min_w(px(17.))
                            .px(px(4.))
                            .rounded_full()
                            .bg(soft)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(8.5))
                                    .font_family("Geist Mono")
                                    .text_color(tone)
                                    .child(running.to_string()),
                            ),
                    )
                }),
        );
    }

    list.child(
        div()
            .id("open-folder")
            .mt(px(4.))
            .h(px(34.))
            .px(px(8.))
            .rounded(px(7.))
            .border_1()
            .border_color(p.border)
            .border_dashed()
            .flex()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_folder(cx);
            }))
            .child(
                Icon::new(IconName::FolderPlus)
                    .size(px(13.))
                    .text_color(p.muted_fg),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child("Open local folder…"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(9.))
                    .font_family("Geist Mono")
                    .text_color(p.muted_fg.opacity(0.6))
                    .child("⌘O"),
            ),
    )
    .into_any_element()
}

fn files_body(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let project_name = this.project().name.clone();
    let branch = this
        .project()
        .branch
        .clone()
        .unwrap_or_else(|| "main".into());
    let selected = this
        .inspector
        .as_ref()
        .map(|inspector| inspector.path.clone());
    let rows = file_rows(&this.file_tree.entries, selected.as_deref(), cx);

    let mut root = div().flex().flex_col().pb(px(10.)).child(
        div()
            .mx(px(6.))
            .px(px(7.))
            .h(px(26.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(6.))
            .bg(p.muted)
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(px(11.))
                    .text_color(p.muted_fg),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.fg)
                    .truncate()
                    .child(project_name.to_uppercase()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(8.5))
                    .font_family("Geist Mono")
                    .text_color(p.muted_fg)
                    .child(branch),
            ),
    );

    if let Some(error) = this.file_tree.error.clone() {
        root = root.child(
            div()
                .mx(px(8.))
                .mt(px(8.))
                .p(px(9.))
                .rounded(px(7.))
                .bg(p.amber_soft)
                .text_size(px(10.))
                .text_color(p.amber)
                .child(error),
        );
    } else {
        root = root.children(rows);
    }
    root.into_any_element()
}

fn file_rows(
    entries: &[FileEntry],
    selected: Option<&str>,
    cx: &mut Context<KestrelApp>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let mut rows = Vec::new();

    for entry in entries {
        let path = entry.path.clone();
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let expanded = entry.expanded;
        let depth = entry.depth;
        let active = selected == Some(path.as_str());
        let icon_name = if is_dir {
            if expanded {
                IconName::FolderOpen
            } else {
                IconName::Folder
            }
        } else {
            file_icon(&name)
        };
        let icon_color = if is_dir { p.blue } else { p.muted_fg };

        let mut row = div()
            .id(format!("file-row:{path}"))
            .h(px(24.))
            .pl(px(7. + depth as f32 * 13.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            .when(active, |this| this.bg(p.accent))
            .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if is_dir {
                    this.toggle_folder(&path, cx);
                } else {
                    this.open_file(&path, window, cx);
                    cx.notify();
                }
            }));

        if is_dir {
            row = row.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(10.))
                .text_color(p.muted_fg.opacity(0.7)),
            );
        } else {
            row = row.child(div().w(px(10.)));
        }

        row = row
            .child(Icon::new(icon_name).size(px(12.5)).text_color(icon_color))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(if active { p.fg } else { p.fg.opacity(0.86) })
                    .truncate()
                    .child(name),
            );

        rows.push(row.into_any_element());

        if is_dir && expanded {
            rows.extend(file_rows(&entry.children, selected, cx));
        }
    }

    rows
}

fn search_body(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let root = this.project().path.clone();
    let query = this.search_query.clone();
    let hits = this.search_hits.clone();

    let input = Input::new(&this.search_input)
        .appearance(false)
        .bordered(false)
        .cleanable(true)
        .h(px(28.))
        .prefix(
            Icon::new(IconName::Search)
                .size(px(12.5))
                .text_color(p.muted_fg),
        )
        .suffix(
            div()
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg.opacity(0.65))
                .child("↵"),
        );

    let mut root_el = div()
        .flex()
        .flex_col()
        .pb(px(10.))
        .child(
            div()
                .mx(px(6.))
                .px(px(6.))
                .h(px(30.))
                .rounded(px(7.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().flex_1().min_w(px(0.)).child(input))
                .child(
                    div()
                        .id("run-search")
                        .h(px(21.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .bg(p.fg)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.run_search(cx)))
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.bg)
                                .child("Run"),
                        ),
                ),
        )
        .child(
            div()
                .px(px(12.))
                .h(px(26.))
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(if query.is_empty() {
                            "RECENT MATCHES".to_string()
                        } else {
                            format!("{} RESULTS", hits.len())
                        }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(9.))
                        .text_color(p.muted_fg.opacity(0.6))
                        .child("project"),
                ),
        );

    for (path, line, preview) in &hits {
        let display = Path::new(path)
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.clone());
        let file = Path::new(&display)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&display)
            .to_string();
        let directory = Path::new(&display)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default();
        let hit_path = path.clone();
        let hit_line = *line;

        root_el = root_el.child(
            div()
                .id(format!("search-hit:{path}:{line}"))
                .px(px(9.))
                .py(px(6.))
                .mx(px(6.))
                .rounded(px(6.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_search_hit(&hit_path, hit_line, window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            Icon::new(file_icon(&file))
                                .size(px(11.5))
                                .text_color(p.blue),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(file),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg.opacity(0.7))
                                .truncate()
                                .child(directory),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child(format!(":{line}")),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .line_height(relative(1.45))
                        .truncate()
                        .child(preview.clone()),
                ),
        );
    }

    if hits.is_empty() {
        root_el = root_el.child(empty_state(
            IconName::SearchX,
            "No matches yet",
            "Type a query and press Enter to search the whole project.",
            &p,
        ));
    }

    root_el.into_any_element()
}

fn source_control_body(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let branch = if this.git.branch.is_empty() {
        this.project()
            .branch
            .clone()
            .unwrap_or_else(|| "main".into())
    } else {
        this.git.branch.clone()
    };
    let changes = this.git.changes.clone();
    let loading = this.git_loading;
    let error = this.git.error.clone();
    let change_count = changes.len();

    let header = div()
        .mx(px(6.))
        .p(px(9.))
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(IconName::GitBranch)
                        .size(px(12.5))
                        .text_color(p.blue),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg)
                        .child(branch),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("git-refresh")
                        .size(px(18.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_git_status(cx)))
                        .child(
                            Icon::new(if loading {
                                IconName::LoaderCircle
                            } else {
                                IconName::RefreshCw
                            })
                            .size(px(11.))
                            .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{change_count} {}",
                            if change_count == 1 {
                                "change"
                            } else {
                                "changes"
                            }
                        )),
                ),
        )
        .child(
            div()
                .h(px(26.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(p.border)
                .bg(p.sidebar)
                .flex()
                .items_center()
                .child(
                    Input::new(&this.commit_input)
                        .appearance(false)
                        .bordered(false)
                        .h(px(24.)),
                ),
        )
        .child(
            div()
                .id("commit-button")
                .h(px(25.))
                .rounded(px(5.))
                .bg(p.fg)
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .cursor_pointer()
                .hover(move |style| style.opacity(0.9))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.commit_changes(window, cx);
                }))
                .child(
                    Icon::new(IconName::GitCommitHorizontal)
                        .size(px(11.5))
                        .text_color(p.bg),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.bg)
                        .child("Commit staged"),
                ),
        );

    let mut list = div().flex().flex_col().pb(px(10.)).child(header).child(
        div()
            .px(px(12.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                div()
                    .text_size(px(9.))
                    .font_family("Geist Mono")
                    .text_color(p.muted_fg)
                    .child("CHANGES"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("stage-all")
                    .text_size(px(9.5))
                    .text_color(if changes.is_empty() {
                        p.muted_fg.opacity(0.5)
                    } else {
                        p.blue
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.stage_all_changes(cx)))
                    .child("Stage all"),
            ),
    );

    if let Some(error) = error {
        list = list.child(
            div()
                .mx(px(6.))
                .p(px(9.))
                .rounded(px(7.))
                .bg(p.red_soft)
                .flex()
                .items_start()
                .gap(px(7.))
                .child(
                    Icon::new(IconName::CircleAlert)
                        .size(px(12.))
                        .text_color(p.red),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(9.5))
                        .line_height(relative(1.5))
                        .text_color(p.red)
                        .child(error),
                ),
        );
    } else if changes.is_empty() {
        list = list.child(
            div()
                .mx(px(6.))
                .py(px(22.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .child(
                    Icon::new(if loading {
                        IconName::LoaderCircle
                    } else {
                        IconName::CircleCheck
                    })
                    .size(px(18.))
                    .text_color(if loading { p.muted_fg } else { p.green }),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(if loading {
                            "Reading the working tree…"
                        } else {
                            "Working tree clean"
                        }),
                ),
        );
    }

    for change in &changes {
        let path = change.path.clone();
        let status = change.status;
        let (tone, soft) = match status {
            'A' | 'U' => (p.green, p.green_soft),
            'D' => (p.red, p.red_soft),
            'R' => (p.violet, p.violet_soft),
            _ => (p.amber, p.amber_soft),
        };
        let label = status.to_string();
        let additions = change.additions;
        let deletions = change.deletions;
        let staged = change.staged;
        let group = format!("change-row:{path}");
        let action_path = path.clone();
        let diff_path = path.clone();

        list = list.child(
            div()
                .id(format!("change:{path}"))
                .group(group.clone())
                .mx(px(6.))
                .px(px(7.))
                .h(px(29.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_file(&path, window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(15.))
                        .rounded(px(4.))
                        .bg(soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .font_weight(FontWeight::BOLD)
                                .text_color(tone)
                                .child(label),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(10.))
                        .font_family("Geist Mono")
                        .text_color(p.fg.opacity(0.85))
                        .truncate()
                        .child(change.path.clone()),
                )
                .when(staged, |this| {
                    this.child(div().size(px(4.)).rounded_full().bg(p.green.opacity(0.8)))
                })
                .when(additions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
                            .text_color(p.green)
                            .child(format!("+{additions}")),
                    )
                })
                .when(deletions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(8.5))
                            .font_family("Geist Mono")
                            .text_color(p.red)
                            .child(format!("−{deletions}")),
                    )
                })
                .child(
                    div()
                        .id(format!("change-diff:{diff_path}"))
                        .size(px(18.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .opacity(0.35)
                        .group_hover(group.clone(), move |style| style.opacity(1.))
                        .hover(move |style| style.bg(p.blue_soft))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_diff(&diff_path, staged, cx);
                        }))
                        .child(
                            Icon::new(IconName::Diff)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id(format!("change-stage:{action_path}"))
                        .size(px(18.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .opacity(0.35)
                        .group_hover(group, move |style| style.opacity(1.))
                        .hover(move |style| {
                            style.bg(if staged { p.amber_soft } else { p.green_soft })
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if staged {
                                this.unstage_file(&action_path, cx);
                            } else {
                                this.stage_file(&action_path, cx);
                            }
                        }))
                        .child(
                            Icon::new(if staged {
                                IconName::Minus
                            } else {
                                IconName::Plus
                            })
                            .size(px(10.5))
                            .text_color(if staged {
                                p.amber
                            } else {
                                p.green
                            }),
                        ),
                ),
        );
    }

    list.child(
        div()
            .mx(px(6.))
            .mt(px(6.))
            .p(px(9.))
            .rounded(px(7.))
            .bg(p.blue_soft)
            .flex()
            .items_start()
            .gap(px(7.))
            .child(Icon::new(IconName::Info).size(px(12.)).text_color(p.blue))
            .child(
                div()
                    .text_size(px(9.5))
                    .line_height(relative(1.5))
                    .text_color(p.blue)
                    .child("Status and diffs come from the system git CLI. Stage, commit and branch actions never leave this machine."),
            ),
    )
    .into_any_element()
}

fn sessions_body(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let selected = this.selected_session;
    let sessions = this.sessions.clone();

    let mut list = div()
        .flex()
        .flex_col()
        .pb(px(10.))
        .child(group_label("TODAY", &p));

    for (index, session) in sessions.iter().enumerate() {
        if index == 2 {
            list = list.child(group_label("EARLIER", &p));
        }
        let active = index == selected;
        let (status_color, status_soft) = status_palette(&p, session.status);
        let title = session.title.clone();
        let harness = session.harness.clone();
        let model = session.model.clone();
        let updated = session.updated_at.clone();
        let cost = session.cost_usd;
        let duration = session.duration_secs;
        let group = format!("session-row-{index}");

        list = list.child(
            div()
                .id(("session-row", index))
                .group(group.clone())
                .mx(px(6.))
                .px(px(8.))
                .py(px(7.))
                .rounded(px(7.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .cursor_pointer()
                .when(active, |this| {
                    this.bg(p.card).border_1().border_color(p.border)
                })
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(index, cx);
                }))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(6.))
                        .child(
                            div()
                                .mt(px(4.))
                                .size(px(5.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(status_color),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(10.5))
                                .font_weight(if active {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .line_height(relative(1.35))
                                .text_color(p.fg)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg.opacity(0.75))
                                .child(updated),
                        ),
                )
                .child(
                    div()
                        .pl(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .h(px(16.))
                                .px(px(5.))
                                .rounded_full()
                                .bg(status_soft)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(8.))
                                        .font_family("Geist Mono")
                                        .text_color(status_color)
                                        .child(session.status.label().to_uppercase()),
                                ),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(9.))
                                .text_color(p.muted_fg)
                                .child(format!("{harness} · {model}")),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(8.5))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child(format!(
                                    "${cost:.2} · {}",
                                    crate::app::format_duration(duration)
                                )),
                        )
                        .child(
                            div()
                                .id(("session-delete", index))
                                .size(px(18.))
                                .flex_shrink_0()
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .opacity(0.35)
                                .group_hover(group, move |style| style.opacity(1.))
                                .hover(move |style| style.bg(p.red_soft))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.request_delete_session(index, cx);
                                }))
                                .child(
                                    Icon::new(IconName::Trash)
                                        .size(px(10.5))
                                        .text_color(p.muted_fg),
                                ),
                        ),
                ),
        );
    }

    list.child(
        div()
            .mx(px(12.))
            .mt(px(10.))
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                Icon::new(IconName::Archive)
                    .size(px(11.))
                    .text_color(p.muted_fg),
            )
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(p.muted_fg)
                    .child("Sessions are stored locally and resumable."),
            ),
    )
    .into_any_element()
}

fn usage_footer(this: &mut KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let mut usages = Vec::new();
    if let Some(live) = this.session_usage() {
        usages.push(live);
    }
    usages.extend(this.usages.iter().cloned());
    let (status_dot, status_label) = match &this.harness.status {
        HarnessStatus::Ready { agent, version } => (p.green, format!("{agent} {version} · ACP")),
        HarnessStatus::Starting => (p.amber, "Connecting harness…".to_string()),
        HarnessStatus::Failed(_) => (p.red, "Harness failed".to_string()),
        HarnessStatus::Exited(_) => (p.red, "Harness exited".to_string()),
        HarnessStatus::Offline => (p.muted_fg, "Harness offline".to_string()),
    };

    let mut footer = div()
        .flex_shrink_0()
        .border_t_1()
        .border_color(p.border)
        .bg(p.sidebar)
        .px(px(10.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(9.))
                        .font_family("Geist Mono")
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_fg)
                        .child("SUBSCRIPTION USAGE"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("usage-details")
                        .text_size(px(9.))
                        .text_color(p.muted_fg)
                        .cursor_pointer()
                        .hover(move |style| style.text_color(p.fg))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_tab = SettingsTab::Usage;
                            cx.notify();
                        }))
                        .child("Details"),
                ),
        );

    for usage in usages {
        let Some(window) = usage.primary() else {
            continue;
        };
        let tone = tone_color(&p, usage.accent);
        let soft = tone_soft(&p, usage.accent);
        let percent = window.used_percent.clamp(0.0, 100.0);
        let provider = usage.provider.clone();
        let plan = usage.plan.clone();
        let source = match window.source {
            UsageSource::Live => "live",
            UsageSource::Session => "session",
            UsageSource::Estimated => "est.",
            UsageSource::Unavailable => "n/a",
        };
        let source_color = match window.source {
            UsageSource::Live => p.green,
            UsageSource::Session => p.blue,
            UsageSource::Estimated => p.amber,
            UsageSource::Unavailable => p.muted_fg,
        };

        footer = footer.child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            div()
                                .text_size(px(9.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg.opacity(0.88))
                                .child(provider),
                        )
                        .child(
                            div()
                                .h(px(13.))
                                .px(px(4.))
                                .rounded(px(3.))
                                .bg(soft)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(7.5))
                                        .font_family("Geist Mono")
                                        .text_color(tone)
                                        .child(plan),
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(8.))
                                .font_family("Geist Mono")
                                .text_color(source_color)
                                .child(source),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .font_family("Geist Mono")
                                .text_color(p.muted_fg)
                                .child(format!("{percent:.0}%")),
                        ),
                )
                .child(
                    div()
                        .h(px(3.))
                        .w_full()
                        .rounded_full()
                        .bg(p.muted)
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(relative(percent / 100.0))
                                .rounded_full()
                                .bg(if percent > 85. { p.red } else { tone }),
                        ),
                ),
        );
    }

    footer
        .child(
            div()
                .mt(px(1.))
                .flex()
                .items_center()
                .gap(px(5.))
                .child(dot(status_dot, 4.5))
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child(status_label),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.7))
                        .child("v0.1"),
                ),
        )
        .into_any_element()
}

fn project_menu(this: &KestrelApp, cx: &mut Context<KestrelApp>) -> AnyElement {
    let p = palette(cx);
    let mut menu = div()
        .absolute()
        .top(px(45.))
        .left(px(6.))
        .right(px(6.))
        .p(px(5.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.card)
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .px(px(8.))
                .py(px(5.))
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg)
                .child("SWITCH PROJECT"),
        );

    for (index, project) in this.projects.iter().enumerate() {
        let active = index == this.selected_project;
        let tone = tone_color(&p, project.tone);
        let soft = tone_soft(&p, project.tone);
        let name = project.name.clone();
        let tail = project.path_tail().to_string();
        let monogram = monogram(&name);

        menu = menu.child(
            div()
                .id(("project-menu-row", index))
                .h(px(34.))
                .px(px(6.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .when(active, |this| this.bg(p.accent))
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
                .child(
                    div()
                        .size(px(20.))
                        .rounded(px(5.))
                        .bg(soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(8.))
                                .font_family("Geist Mono")
                                .font_weight(FontWeight::BOLD)
                                .text_color(tone)
                                .child(monogram),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(p.fg)
                        .child(name),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg.opacity(0.7))
                        .child(tail),
                )
                .when(active, |this| {
                    this.child(Icon::new(IconName::Check).size(px(11.)).text_color(p.fg))
                }),
        );
    }

    menu.child(div().h(px(1.)).my(px(3.)).bg(p.border))
        .child(
            div()
                .id("menu-open-folder")
                .h(px(32.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.open_folder(cx);
                }))
                .child(
                    Icon::new(IconName::FolderPlus)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.5))
                        .text_color(p.fg)
                        .child("Open local folder…"),
                )
                .child(
                    div()
                        .text_size(px(8.5))
                        .font_family("Geist Mono")
                        .text_color(p.muted_fg)
                        .child("⌘O"),
                ),
        )
        .child(
            div()
                .id("project-settings")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.settings_open = true;
                    this.settings_tab = SettingsTab::General;
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Settings)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.fg)
                        .child("Project settings"),
                ),
        )
        .child(
            div()
                .id("project-reveal")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    let path = this.project().path.clone();
                    cx.reveal_path(Path::new(&path));
                    this.set_notice("Revealed the project in Finder", cx);
                }))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .text_color(p.fg)
                        .child("Reveal in Finder"),
                ),
        )
        .child(
            div()
                .id("project-copy-path")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    let path = this.project().path.clone();
                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                    this.set_notice("Copied the project path", cx);
                }))
                .child(
                    Icon::new(IconName::Copy)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(10.))
                        .text_color(p.fg)
                        .child("Copy path"),
                ),
        )
        .child(div().h(px(1.)).my(px(3.)).bg(p.border))
        .child(
            div()
                .id("project-remove")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.red_soft))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.request_delete_project(this.selected_project, cx);
                }))
                .child(Icon::new(IconName::Trash).size(px(12.)).text_color(p.red))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.red)
                        .child("Remove project…"),
                ),
        )
        .into_any_element()
}

fn group_label(label: &str, p: &Palette) -> AnyElement {
    div()
        .px(px(12.))
        .h(px(24.))
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(8.5))
                .font_family("Geist Mono")
                .text_color(p.muted_fg.opacity(0.75))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn empty_state(icon_name: IconName, title: &str, body: &str, p: &Palette) -> AnyElement {
    div()
        .px(px(16.))
        .py(px(28.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .size(px(30.))
                .rounded(px(9.))
                .bg(p.muted)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon_name).size(px(15.)).text_color(p.muted_fg)),
        )
        .child(
            div()
                .text_size(px(10.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.fg.opacity(0.8))
                .child(title.to_string()),
        )
        .child(
            div()
                .max_w(px(180.))
                .text_center()
                .text_size(px(9.5))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child(body.to_string()),
        )
        .into_any_element()
}

fn monogram(name: &str) -> String {
    let mut letters = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>();
    if letters.is_empty() {
        letters = name.chars().take(2).collect();
    }
    letters.to_uppercase()
}

fn file_icon(name: &str) -> IconName {
    let lower = name.to_lowercase();
    if lower.ends_with(".rs") {
        IconName::FileCode
    } else if lower.ends_with(".toml") || lower.ends_with(".yaml") || lower.ends_with(".yml") {
        IconName::FileCog
    } else if lower.ends_with(".json") || lower.ends_with(".lock") {
        IconName::FileBraces
    } else if lower.ends_with(".md") || lower.ends_with(".txt") {
        IconName::FileText
    } else if lower.ends_with(".svg") || lower.ends_with(".png") || lower.ends_with(".jpg") {
        IconName::FileImage
    } else if lower.starts_with(".git") {
        IconName::GitBranch
    } else {
        IconName::File
    }
}
