//! A disposable native-window scenario: actual pointer/key dispatch and the
//! same historical diff overlay used by the application. No provider calls.
use crate::{gui_test::Step, harness::ChatItem};
use dunlin_agent::{FileChange, ToolCallView};
use dunlin_domain::{MainView, ToolKind, ToolStatus};
use dunlin_ui::*;
use std::path::Path;

fn edit(id: &str, path: &str, at_ms: u64, old: &str, new: &str) -> ChatItem {
    let diff = similar::TextDiff::from_lines(old, new);
    let additions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == similar::ChangeTag::Insert)
        .count() as u32;
    let deletions = diff
        .iter_all_changes()
        .filter(|change| change.tag() == similar::ChangeTag::Delete)
        .count() as u32;
    let patch = diff
        .unified_diff()
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    ChatItem::Tool {
        view: ToolCallView {
            id: id.into(),
            title: format!("Edit {path}"),
            kind: ToolKind::Edit,
            status: ToolStatus::Done,
            detail: path.into(),
            locations: vec![path.into()],
            additions,
            deletions,
            diff: Some(patch.clone()),
            file_changes: vec![FileChange {
                path: path.into(),
                additions,
                deletions,
                diff: patch,
            }],
        },
        at_ms,
        duration_ms: 600,
    }
}

pub(crate) fn steps(root: &Path) -> Vec<Step> {
    let root = root.to_path_buf();
    let project = root.clone();
    let captures = root.parent().unwrap().to_path_buf();
    let mut capture_started = None;
    vec![
        Step::act(
            "fixture: recorded edits, two prompts and a subagent",
            move |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    let base = crate::harness::now_ms().saturating_sub(240_000);
                    let page_path = project.join("app/page.tsx").to_string_lossy().to_string();
                    let page_before = format!(
                        "export const page = 1; // {}\n",
                        "long preview source ".repeat(20)
                    );
                    let page_after = page_before.replacen("page = 1", "page = 2", 1);
                    app.harness.transcript = vec![
                        ChatItem::User {
                            text: "Build the landing page".into(),
                            timing: None,
                            at_ms: base,
                        },
                        edit("page", &page_path, base + 10_000, "", &page_before),
                        edit(
                            "hero",
                            "components/hero.tsx",
                            base + 40_000,
                            "old\n",
                            "new\nmore\n",
                        ),
                        ChatItem::User {
                            text: "Refine and add tests".into(),
                            timing: None,
                            at_ms: base + 80_000,
                        },
                        edit(
                            "page-2",
                            &page_path,
                            base + 120_000,
                            &page_before,
                            &page_after,
                        ),
                        ChatItem::Child {
                            id: "tests".into(),
                            parent_id: "root".into(),
                            title: Some("Test agent".into()),
                            at_ms: base + 150_000,
                            items: vec![edit(
                                "test",
                                "tests/hero.test.tsx",
                                base + 180_000,
                                "",
                                "test('hero', () => {});\n",
                            )],
                        },
                    ];
                    app.harness.running = false;
                    app.set_view(MainView::Chat, cx);
                    app.reduce_motion = true;
                    app.workbench.show_file = false;
                    assert_eq!(
                        super::project(&app.harness.transcript).totals(None),
                        (5, 2, 3)
                    );
                });
            },
        ),
        Step::key("cmd-3"),
        Step::until(
            "Timeline is the third native view and Trace is absent",
            |owner, _, cx| {
                assert_eq!(owner.read(cx).view, MainView::Timeline);
                assert!(
                    owner
                        .read(cx)
                        .commands()
                        .iter()
                        .all(|command| !command.label.contains("Trace"))
                );
                crate::gui_probe::bounds("file-timeline-chart").is_some()
            },
        ),
        Step::click("file-timeline-chart"),
        Step::key("home"),
        Step::until(
            "keyboard selection pins the first recorded edit",
            |_, _, _| crate::gui_probe::bounds("timeline-open-diff").is_some(),
        ),
        Step::act(
            "timeline uses application-sized chrome and a bounded single-line preview",
            |_, _, _| {
                let bounds = |id: &str| {
                    crate::gui_probe::bounds(id)
                        .unwrap_or_else(|| panic!("missing sizing probe: {id}"))
                };
                assert!(
                    bounds("timeline-header").size.height <= px(50.),
                    "header must use workspace typography and padding"
                );
                assert!(
                    bounds("timeline-toolbar").size.height <= px(40.),
                    "toolbar must use shared compact controls"
                );
                assert!(bounds("timeline-lane-0").size.height <= px(24.));
                assert!(bounds("timeline-edit-details").size.width <= px(361.));
                assert!(
                    bounds("timeline-edit-details").size.height <= px(150.),
                    "long paths and source lines must not enlarge the inspector"
                );
                assert!(bounds("timeline-detail-path").size.height <= px(17.));
                assert!(
                    bounds("timeline-diff-preview").size.height <= px(19.),
                    "one source line must remain one preview row"
                );
                assert_eq!(bounds("timeline-open-diff").size.height, px(28.));
            },
        ),
        Step::until("optional timeline preview capture", move |_, window, _| {
            if std::env::var_os("DUNLIN_PREVIEW_CAPTURE").is_some() {
                window.present_debug_frame();
                if capture_started
                    .get_or_insert_with(std::time::Instant::now)
                    .elapsed()
                    < std::time::Duration::from_millis(600)
                {
                    return false;
                }
                crate::workspace_ui_smoke::capture_preview(window, &captures.join("timeline.png"));
            }
            true
        }),
        Step::click("timeline-open-diff"),
        Step::until(
            "full diff uses the historical patch without consulting Git",
            |owner, window, cx| {
                let app = owner.read(cx);
                let Some(diff) = &app.diff_view else {
                    return false;
                };
                assert!(diff.path.ends_with("/app/page.tsx"));
                assert!(diff.error.is_none());
                owner.update(cx, |app, cx| app.close_diff(window, cx));
                true
            },
        ),
        Step::until(
            "pinned edit remains inspectable after closing its diff",
            |_, _, _| crate::gui_probe::bounds("timeline-open-chat").is_some(),
        ),
        Step::click("timeline-open-chat"),
        Step::act(
            "the exact source tool is revealed, and the fixture file is untouched",
            move |owner, _, cx| {
                let app = owner.read(cx);
                assert_eq!(app.view, MainView::Chat);
                assert_eq!(app.expanded_tool, Some(1));
                assert_eq!(
                    std::fs::read_to_string(root.join("file.txt")).unwrap(),
                    "original"
                );
            },
        ),
        Step::key("cmd-3"),
        Step::act(
            "a live proposed patch does not increase totals",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    let mut item = edit(
                        "live",
                        "live.rs",
                        crate::harness::now_ms().saturating_sub(600),
                        "",
                        "confirmed\n",
                    );
                    if let ChatItem::Tool { view, .. } = &mut item {
                        view.status = ToolStatus::Running;
                    }
                    app.harness.transcript.push(item);
                    app.harness.running = true;
                    app.reduce_motion = false;
                    assert_eq!(
                        super::project(&app.harness.transcript).totals(None),
                        (5, 2, 3)
                    );
                    cx.notify();
                });
            },
        ),
        Step::until("running edit is rendered", |_, _, _| {
            crate::gui_probe::bounds("file-timeline-chart").is_some()
        }),
        Step::act(
            "complete the same tool in place, without appending transcript rows",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    let count = app.harness.transcript.len();
                    if let Some(ChatItem::Tool { view, .. }) = app.harness.transcript.last_mut() {
                        view.status = ToolStatus::Done;
                    }
                    app.harness.file_revision += 1;
                    assert_eq!(app.harness.transcript.len(), count);
                    assert_eq!(
                        super::project(&app.harness.transcript).totals(None),
                        (6, 2, 4)
                    );
                    cx.notify();
                });
            },
        ),
        Step::act("focus chart outside the pinned popover", |_, window, cx| {
            let bounds = crate::gui_probe::bounds("file-timeline-chart").unwrap();
            let position = point(bounds.right() - px(12.), bounds.bottom() - px(12.));
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    position,
                    button: MouseButton::Left,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
        }),
        Step::key("end"),
        Step::click("timeline-open-diff"),
        Step::until(
            "End selects the newly confirmed live patch, not a stale cached edit",
            |owner, window, cx| {
                let Some(diff) = &owner.read(cx).diff_view else {
                    return false;
                };
                assert_eq!(diff.path, "live.rs");
                assert!(diff.error.is_none());
                owner.update(cx, |app, cx| {
                    app.harness.running = false;
                    app.close_diff(window, cx);
                });
                true
            },
        ),
    ]
}
