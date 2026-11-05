//! Isolated native control checks. No application data, clipboard, harness or OS input.
use dunlin_ui::{
    assets::IconName,
    component::{Disableable, ThemeMode},
    *,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

type Probe = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Default)]
struct Controls {
    actions: usize,
    disabled_actions: usize,
    row_actions: usize,
    closed: usize,
    toggled: bool,
    checked: bool,
    tab: usize,
    disabled_bounds: Probe,
    close_bounds: Probe,
    tab_bounds: Probe,
    row_bounds: Probe,
    menu_bounds: Probe,
    menu_marker: Probe,
}

fn marker(probe: Probe) -> impl IntoElement + Styled {
    canvas(move |bounds, _, _| probe.set(Some(bounds)), |_, _, _, _| {})
}

fn measured(element: impl IntoElement, probe: Probe) -> Div {
    div()
        .relative()
        .flex()
        .child(element)
        .child(marker(probe).absolute().top_0().left_0().size_full())
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        div()
            .size_full()
            .p(px(20.))
            .bg(p.bg)
            .text_color(p.fg)
            .flex()
            .flex_col()
            .items_start()
            .gap(px(12.))
            .child(
                dunlin_ui::button("smoke-action")
                    .label("Action")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.actions += 1;
                        cx.notify();
                    })),
            )
            .child(measured(
                dunlin_ui::button("smoke-disabled")
                    .label("Disabled")
                    .disabled(true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.disabled_actions += 1;
                        cx.notify();
                    })),
                self.disabled_bounds.clone(),
            ))
            .child(
                segment_button("smoke-toggle", "Toggle", self.toggled).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.toggled = !this.toggled;
                        cx.notify();
                    },
                )),
            )
            .child(
                checkbox("smoke-checkbox", "Task completion", self.checked).on_change(cx.listener(
                    |this, checked, _, cx| {
                        this.checked = *checked;
                        cx.notify();
                    },
                )),
            )
            .child(
                document_tabs("smoke-tabs")
                    .selected_index(self.tab)
                    .child(
                        document_tab("Conversation").on_click(cx.listener(|this, _, _, cx| {
                            this.tab = 0;
                            cx.notify();
                        })),
                    )
                    .child(
                        document_tab("File •")
                            .prefix(marker(self.tab_bounds.clone()).w(px(8.)).h(px(16.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = 1;
                                cx.notify();
                            }))
                            .suffix(measured(
                                icon_button("smoke-tab-close", IconName::X, "Close file").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.closed += 1;
                                        cx.notify();
                                    }),
                                ),
                                self.close_bounds.clone(),
                            )),
                    ),
            )
            .child(measured(
                action_row("smoke-row", "Select item")
                    .child("Rich action row")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.row_actions += 1;
                        cx.notify();
                    })),
                self.row_bounds.clone(),
            ))
            .child(measured(
                menu_button("smoke-menu", "Menu item").w(px(300.))
                    .child(marker(self.menu_marker.clone()).w(px(8.)).h(px(16.))),
                self.menu_bounds.clone(),
            ))
    }
}

pub(crate) fn key(window: &mut Window, cx: &mut App, key: &str) {
    let keystroke = Keystroke::parse(key).unwrap();
    window.dispatch_event(
        PlatformInput::KeyDown(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
    window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke }), cx);
}

fn click(window: &mut Window, cx: &mut App, probe: &Probe) {
    let position = probe.get().expect("control was not laid out").center();
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
}

pub(crate) fn run() {
    application().with_assets(assets::AllAssets).run(|cx| {
        cx.set_app_identity("dev.dunlin.ide.controls-smoke", "Dunlin Controls Smoke");
        dunlin_ui::init(cx);
        set_mode(ThemeMode::Light, None, cx);
        let (window, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(80.)), size(px(640.), px(480.))))),
            ..dunlin_ui::window_options()
        }, cx, |_, cx| cx.new(|_| Controls::default())).unwrap();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let mut stage = 0;
            let mut dark = false;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(15), "controls smoke stalled: {stage}");
                    window.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                    window.update(cx, |_, window, cx| {
                        match stage {
                            0 => { window.blur(cx); window.focus_next(cx); key(window, cx, "enter"); }
                            1 => { assert_eq!(owner.read(cx).actions, 1); window.focus_next(cx); key(window, cx, "space"); }
                            2 => { assert!(owner.read(cx).toggled, "disabled action did not leave keyboard traversal"); window.focus_next(cx); key(window, cx, "space"); }
                            3 => { assert!(owner.read(cx).checked); let probe = owner.read(cx).disabled_bounds.clone(); click(window, cx, &probe); }
                            4 => { assert_eq!(owner.read(cx).disabled_actions, 0); let probe = owner.read(cx).tab_bounds.clone(); click(window, cx, &probe); }
                            5 => { assert_eq!(owner.read(cx).tab, 1); owner.update(cx, |this, cx| { this.tab = 0; cx.notify(); }); }
                            6 => { let probe = owner.read(cx).close_bounds.clone(); click(window, cx, &probe); }
                            7 => { assert_eq!(owner.read(cx).closed, 1); assert_eq!(owner.read(cx).tab, 0, "closing a tab also selected it"); let probe = owner.read(cx).row_bounds.clone(); click(window, cx, &probe); }
                            8 => {
                                assert_eq!(owner.read(cx).row_actions, 1);
                                let controls = owner.read(cx);
                                let menu = controls.menu_bounds.get().unwrap();
                                let marker = controls.menu_marker.get().unwrap();
                                let inset = menu.right() - marker.right();
                                assert!(inset >= px(0.) && inset <= px(14.), "menu label did not fill its row");
                                if dark { return true; }
                                set_mode(ThemeMode::Dark, Some(window), cx);
                                owner.update(cx, |this, cx| {
                                    this.actions = 0; this.disabled_actions = 0; this.row_actions = 0; this.closed = 0;
                                    this.toggled = false; this.checked = false; cx.notify();
                                });
                                dark = true;
                            }
                            _ => unreachable!(),
                        }
                        false
                    }).unwrap()
                });
                if done {
                    println!("PASS: native controls in light/dark themes: Enter/Space activation, disabled traversal and clicks, checkbox state, tab selection, isolated close action, rich row action and menu label layout");
                    cx.update(|cx| cx.quit());
                    break;
                }
                stage = if stage == 8 { 0 } else { stage + 1 };
            }
        }).detach();
    });
}
