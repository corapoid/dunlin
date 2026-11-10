//! Disposable native acceptance. Only tiny folders created by this fixture are
//! moved to the real system Trash; it never empties Trash or accesses user repos.
use super::*;
use crate::gui_test::Step;
use std::{fs, path::Path};

pub(crate) fn prepare(root: &Path) {
    assert!(
        std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["init", "--template=", "-q"])
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("package.json"), "{}").unwrap();
    fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\ndist/\ntarget/\n").unwrap();
    for (name, data) in [
        ("node_modules", "12345"),
        ("dist", "1234567"),
        ("target", "123456789"),
    ] {
        fs::create_dir(root.join(name)).unwrap();
        fs::write(root.join(name).join("dunlin-cleanup-smoke-fixture"), data).unwrap();
    }
}

fn ready(expected: usize) -> Step {
    Step::until(
        "cleanup preview has measured all folders",
        move |owner, _, cx| {
            let app = owner.read(cx);
            let Some(dialog) = &app.group_cleanup.dialog else {
                return false;
            };
            if dialog.phase != Phase::Reviewing {
                return false;
            }
            assert_eq!(dialog.folders.len(), expected);
            assert_eq!(dialog.total(), (expected, (expected / 3 * 21) as u64));
            assert!(app.group_cleanup.busy && app.navigation_blocked());
            true
        },
    )
}

fn prompt_answer(index: usize) -> Step {
    Step::until(
        format!("answer cleanup confirmation {index}"),
        move |_, window, cx| {
            if !window.has_active_prompt() {
                return false;
            }
            let Some(bounds) = dunlin_ui::debug_confirmation_answer(index) else {
                return false;
            };
            crate::workspace_ui_smoke::click(window, bounds.center(), cx);
            true
        },
    )
}

pub(crate) fn steps(root: &Path) -> Vec<Step> {
    let mut steps = vec![
        Step::act("open project settings", |owner, _, cx| {
            owner.update(cx, |app, cx| {
                app.open_project_settings(app.selected_project, cx);
            });
        }),
        Step::click("project-settings-clean"),
        ready(3),
        Step::click("cleanup-select-none"),
        Step::act(
            "none selected means zero bytes and no cleanup",
            |owner, _, cx| {
                assert_eq!(
                    owner
                        .read(cx)
                        .group_cleanup
                        .dialog
                        .as_ref()
                        .unwrap()
                        .total(),
                    (0, 0)
                );
            },
        ),
        // Sorted preview: dist, node_modules, target.
        Step::click("cleanup-folder-1"),
        Step::act(
            "checkbox changes only the selected dependency size",
            |owner, _, cx| {
                assert_eq!(
                    owner
                        .read(cx)
                        .group_cleanup
                        .dialog
                        .as_ref()
                        .unwrap()
                        .total(),
                    (1, 5)
                );
            },
        ),
        Step::click("cleanup-confirm"),
        prompt_answer(0),
        Step::until(
            "cancel confirmation retains the review and selection",
            |owner, _, cx| {
                let app = owner.read(cx);
                let dialog = app.group_cleanup.dialog.as_ref().unwrap();
                if dialog.phase != Phase::Reviewing {
                    return false;
                }
                assert_eq!(dialog.total(), (1, 5));
                assert!(dialog.folders.iter().all(|f| f.path.is_dir()));
                true
            },
        ),
        Step::key("escape"),
        Step::until(
            "Escape closes preview without changing files",
            |owner, _, cx| {
                let app = owner.read(cx);
                app.group_cleanup.dialog.is_none() && !app.group_cleanup.busy
            },
        ),
        Step::act(
            "close project settings after cleanup review",
            |owner, _, cx| {
                owner.update(cx, |app, cx| app.close_project_settings(cx));
            },
        ),
    ];
    let root = fs::canonicalize(root).unwrap();
    let child = root.parent().unwrap().join("child-project");
    let setup_child = child.clone();
    steps.push(Step::act(
        "fixture: nested group with another local and missing project",
        move |owner, window, cx| {
            fs::create_dir(&setup_child).unwrap();
            prepare(&setup_child);
            owner.update(cx, |app, cx| {
                let group = ProjectGroup {
                    id: "cleanup-root".into(),
                    name: "Cleanup".into(),
                    parent_id: None,
                    tone: dunlin_domain::ProjectTone::Blue,
                    settings: Default::default(),
                };
                let child_group = ProjectGroup {
                    id: "cleanup-child".into(),
                    name: "Child".into(),
                    parent_id: Some(group.id.clone()),
                    ..group.clone()
                };
                app.projects[app.selected_project].group_id = Some(group.id.clone());
                let template = app.project().clone();
                app.projects.push(Project {
                    id: "cleanup-child-project".into(),
                    name: "Child".into(),
                    path: setup_child.to_string_lossy().into(),
                    group_id: Some(child_group.id.clone()),
                    ..template.clone()
                });
                app.projects.push(Project {
                    id: "cleanup-missing".into(),
                    name: "Missing".into(),
                    path: setup_child.join("missing").to_string_lossy().into(),
                    ..template
                });
                app.project_groups.extend([group, child_group]);
                app.open_group_settings("cleanup-root", window, cx);
            });
        },
    ));
    steps.extend([
        Step::act("scroll group settings to cleanup action", |owner, _, cx| {
            owner.update(cx, |app, cx| {
                app.group_settings_draft
                    .as_ref()
                    .unwrap()
                    .scroll
                    .set_offset(point(px(0.), px(-10000.)));
                cx.notify();
            });
        }),
        Step::click("group-clear"),
        ready(6),
        Step::act(
            "subgroup included and missing project reported",
            |owner, _, cx| {
                let app = owner.read(cx);
                let dialog = app.group_cleanup.dialog.as_ref().unwrap();
                assert_eq!(dialog.skipped.len(), 1);
                assert_eq!(dialog.members.len(), 3);
                assert!(app.group_settings_draft.is_some());
            },
        ),
        // Leave root target intact while cleaning both projects.
        Step::act(
            "reveal the last folder in the virtualized preview",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    app.group_cleanup
                        .dialog
                        .as_ref()
                        .unwrap()
                        .scroll
                        .scroll_to_item(5, ScrollStrategy::Nearest);
                    cx.notify();
                });
            },
        ),
        Step::click("cleanup-folder-5"),
        Step::click("cleanup-confirm"),
        prompt_answer(1),
    ]);
    let check_root = root.clone();
    steps.push(Step::until(
        "system Trash results include both projects and preserve unselected/source data",
        move |owner, _, cx| {
            let app = owner.read(cx);
            let dialog = app.group_cleanup.dialog.as_ref().unwrap();
            if dialog.phase != Phase::Finished {
                return false;
            }
            assert_eq!(dialog.moved.len(), 5);
            assert_eq!(dialog.skipped.len(), 2);
            assert!(dialog.skipped.iter().any(|s| s.reason == "Not selected"));
            assert!(!app.group_cleanup.busy);
            for path in [
                check_root.join("dist"),
                check_root.join("node_modules"),
                child.join("dist"),
                child.join("node_modules"),
                child.join("target"),
            ] {
                assert!(!path.exists());
            }
            assert!(
                check_root
                    .join("target/dunlin-cleanup-smoke-fixture")
                    .exists()
            );
            assert_eq!(
                fs::read_to_string(check_root.join("file.txt")).unwrap(),
                "original"
            );
            assert!(check_root.join(".git").is_dir() && child.join(".git").is_dir());
            true
        },
    ));
    steps.extend([
        Step::click("cleanup-close"),
        Step::act(
            "closing summary preserves group settings draft",
            |owner, _, cx| {
                let app = owner.read(cx);
                assert!(app.group_cleanup.dialog.is_none());
                assert!(app.group_settings_draft.is_some());
            },
        ),
    ]);
    steps
}
