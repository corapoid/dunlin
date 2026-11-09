//! In-process GPUI input, not OS/IME/VoiceOver acceptance. Never dispatch input
//! inside owner.update: handlers must be able to borrow the app themselves.
use crate::app::DunlinApp;
use dunlin_ui::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

type Action = dyn FnMut(&Entity<DunlinApp>, &mut Window, &mut App) -> bool;
pub(crate) struct Step {
    name: String,
    action: Box<Action>,
}
impl Step {
    pub(crate) fn until(
        name: impl Into<String>,
        action: impl FnMut(&Entity<DunlinApp>, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            action: Box::new(action),
        }
    }
    pub(crate) fn act(
        name: impl Into<String>,
        mut action: impl FnMut(&Entity<DunlinApp>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::until(name, move |owner, window, cx| {
            action(owner, window, cx);
            true
        })
    }
    pub(crate) fn click(id: impl Into<String>) -> Self {
        let id = id.into();
        Self::until(format!("click {id}"), move |_, window, cx| {
            let Some(bounds) = crate::gui_probe::bounds(&id) else {
                return false;
            };
            assert!(
                bounds.size.width > px(0.) && bounds.size.height > px(0.),
                "empty hit target: {id}"
            );
            let position = bounds.center();
            assert!(
                position.x >= px(0.)
                    && position.y >= px(0.)
                    && position.x < window.viewport_size().width
                    && position.y < window.viewport_size().height,
                "offscreen target: {id}"
            );
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    click_count: 1,
                    ..Default::default()
                }),
                cx,
            );
            true
        })
    }
    pub(crate) fn key(key: &'static str) -> Self {
        Self::act(format!("press {key}"), move |_, window, cx| {
            window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
        })
    }
    pub(crate) fn type_text(text: &'static str) -> Self {
        Self::act("type into focused input", move |_, window, cx| {
            for ch in text.chars() {
                assert!(
                    window.dispatch_keystroke(
                        Keystroke {
                            key: ch.to_string(),
                            key_char: Some(ch.to_string()),
                            ..Default::default()
                        },
                        cx
                    ),
                    "focused input rejected text"
                );
            }
        })
    }
}

pub(crate) fn run(
    scenario: &'static str,
    directory: PathBuf,
    steps: Vec<Step>,
    window: AnyWindowHandle,
    owner: Entity<DunlinApp>,
    cx: &mut App,
) {
    run_with_timeout(scenario, directory, steps, window, owner, cx, 10);
}

pub(crate) fn run_with_timeout(
    scenario: &'static str,
    directory: PathBuf,
    mut steps: Vec<Step>,
    window: AnyWindowHandle,
    owner: Entity<DunlinApp>,
    cx: &mut App,
    step_timeout_secs: u64,
) {
    let total = steps.len();
    assert!(total > 0, "a GUI scenario must contain assertions");
    cx.spawn(async move |cx| {
        let mut index = 0;
        let mut started = Instant::now();
        let mut announced = false;
        loop {
            // A frame opportunity, not a correctness delay. Every step waits on
            // its own condition; old hit targets are discarded before drawing.
            cx.background_executor().timer(Duration::from_millis(20)).await;
            let done = cx.update(|cx| {
                let step = &mut steps[index];
                if !announced {
                    eprintln!("GUI_STEP: {}", serde_json::json!({"scenario":scenario,"step":step.name,"status":"started"}));
                    announced = true;
                }
                crate::gui_probe::clear();
                window.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                // macOS dialogs are owner-attached GPUI windows, not children
                // of the parent's render tree. Measure and route input there.
                let child = owner.read(cx).overlay_window.debug_window();
                if let Some(child) = child {
                    child.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                }
                child.unwrap_or(window).update(cx, |_, window, cx| {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (step.action)(&owner, window, cx)));
                let failure = match &result {
                    Err(_) => Some("assertion failed"),
                    Ok(false) if started.elapsed() > Duration::from_secs(step_timeout_secs) => Some("condition timed out"),
                    _ => None,
                };
                if let Some(reason) = failure {
                    crate::remote_e2e::ui::capture_failure(window);
                    let app = owner.read(cx);
                    let evidence = serde_json::json!({"scenario":scenario,"step":step.name,"reason":reason,"notice":app.notice,"running":app.harness.running,"status":format!("{:?}",app.harness.status),"editor_dirty":app.editor_dirty,"pending_editor":app.pending_editor.is_some(),"queue":app.harness.queued_prompts().len(),"transcript":app.harness.transcript,"targets":crate::gui_probe::snapshot()});
                    std::fs::write(directory.join("failure.json"), serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
                    eprintln!("GUI_STEP: {}", serde_json::json!({"scenario":scenario,"step":step.name,"status":"failed","reason":reason}));
                    eprintln!("FAIL: {scenario} / {}: {reason}; evidence: {}", step.name, directory.display());
                    // Avoid GPUI panic-unwinding obscuring the original failure.
                    std::process::exit(1);
                }
                if result.unwrap_or(false) {
                    eprintln!("GUI_STEP: {}", serde_json::json!({"scenario":scenario,"step":step.name,"status":"passed","elapsed_ms":started.elapsed().as_millis()}));
                    index += 1; started = Instant::now(); announced = false;
                }
                index == total
                }).unwrap()
            });
            if done {
                std::fs::write(directory.join("PASS"), scenario).unwrap();
                println!("PASS: {scenario} ({total} named GUI steps)");
                cx.update(|cx| cx.quit());
                break;
            }
        }
    }).detach();
}
