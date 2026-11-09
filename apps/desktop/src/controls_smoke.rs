//! Isolated native control checks. No application data, clipboard, harness or OS input.
use dunlin_ui::{
    assets::IconName,
    component::{
        Disableable, ThemeMode,
        input::{InputEvent, InputState},
    },
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
    loading_actions: usize,
    choice_actions: usize,
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
    link_bounds: [Probe; 2],
    links: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    fields: Vec<Entity<InputState>>,
    field_bounds: [Probe; 5],
    action_bounds: Probe,
    loading_bounds: Probe,
    choice_bounds: Probe,
}

impl Controls {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fields = (0..5)
            .map(|_| cx.new(|cx| InputState::new(window, cx)))
            .collect::<Vec<_>>();
        // Input updates must repaint the compound form just like application fields.
        for field in &fields {
            cx.subscribe(field, |_, _, _: &InputEvent, cx| cx.notify())
                .detach();
        }
        Self {
            fields,
            ..Default::default()
        }
    }
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
            .child(measured(
                dunlin_ui::action_button("smoke-action")
                    .label("Action")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.actions += 1;
                        cx.notify();
                    })),
                self.action_bounds.clone(),
            ))
            .child(measured(
                dunlin_ui::action_button("smoke-disabled")
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
                            .focusable()
                            .tab_stop(true)
                            .prefix(marker(self.tab_bounds.clone()).w(px(8.)).h(px(16.)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tab = 1;
                                cx.notify();
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.tab = 1;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
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
                menu_button("smoke-menu", "Menu item")
                    .w(px(300.))
                    .child(marker(self.menu_marker.clone()).w(px(8.)).h(px(16.))),
                self.menu_bounds.clone(),
            ))
            .children(
                [
                    "https://example.com/plain",
                    "[Documentation](https://example.com/docs)",
                ]
                .into_iter()
                .enumerate()
                .map(|(index, text)| {
                    let links = self.links.clone();
                    measured(
                        crate::views::chat::markdown_text(
                            format!("response-link:{index}"),
                            text.into(),
                        )
                        .on_link_click(move |url, _, _, _| {
                            links.lock().unwrap().push(url.to_string())
                        }),
                        self.link_bounds[index].clone(),
                    )
                }),
            )
            .children(
                [
                    dunlin_ui::input(&self.fields[0]).aria_label("Form field"),
                    compact_input(&self.fields[1]).aria_label("Compact field"),
                    code_input(&self.fields[2]).aria_label("Command field"),
                    search_input(&self.fields[3], "Search fixture"),
                    dunlin_ui::input(&self.fields[4])
                        .aria_label("Disabled field")
                        .disabled(true),
                ]
                .into_iter()
                .enumerate()
                .map(|(index, field)| {
                    measured(field.w(px(300.)), self.field_bounds[index].clone())
                }),
            )
            .child(measured(
                choice_card(
                    "smoke-choice",
                    "Choose permission mode",
                    self.choice_actions > 0,
                    p,
                )
                .w(px(300.))
                .child(choice_indicator(self.choice_actions > 0, p))
                .child("Choice card")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.choice_actions += 1;
                    cx.notify();
                })),
                self.choice_bounds.clone(),
            ))
            .child(measured(
                dunlin_ui::button("smoke-loading")
                    .label("Loading")
                    .loading(true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.loading_actions += 1;
                        cx.notify();
                    })),
                self.loading_bounds.clone(),
            ))
    }
}

pub(crate) fn key(window: &mut Window, cx: &mut App, key: &str) {
    let keystroke = Keystroke::parse_with_os(key, cx.platform_info().os).unwrap();
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
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(40.)), size(px(640.), px(880.))))),
            ..dunlin_ui::window_options()
        }, cx, |window, cx| cx.new(|cx| Controls::new(window, cx))).unwrap();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let mut stage = 0;
            let mut variant = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(30), "controls smoke stalled: {stage}");
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
                            7 => { assert_eq!(owner.read(cx).closed, 1); assert_eq!(owner.read(cx).tab, 0, "closing a tab also selected it"); window.blur(cx); for _ in 0..5 { window.focus_next(cx); } key(window, cx, "space"); let probe = owner.read(cx).row_bounds.clone(); click(window, cx, &probe); }
                            8 => {
                                assert_eq!(owner.read(cx).row_actions, 1);
                                assert_eq!(owner.read(cx).closed, 2, "focused close did not activate once from Space");
                                assert_eq!(owner.read(cx).tab, 0, "keyboard close also selected its parent tab");
                                let controls = owner.read(cx);
                                let menu = controls.menu_bounds.get().unwrap();
                                let marker = controls.menu_marker.get().unwrap();
                                let inset = menu.right() - marker.right();
                                assert!(inset >= px(0.) && inset <= px(14.), "menu label did not fill its row");
                                 let probe = controls.link_bounds[0].clone();
                                 click(window, cx, &probe);
                             }
                             9 => {
                                 assert_eq!(*owner.read(cx).links.lock().unwrap(), ["https://example.com/plain"]);
                                 let probe = owner.read(cx).link_bounds[1].clone();
                                 click(window, cx, &probe);
                             }
                              10 => {
                                  assert_eq!(*owner.read(cx).links.lock().unwrap(), ["https://example.com/plain", "https://example.com/docs"]);
                                  let controls = owner.read(cx);
                                  let heights: Vec<_> = controls.field_bounds.iter().map(|probe| probe.get().unwrap().size.height).collect();
                                  assert_eq!(heights[0], controls.action_bounds.get().unwrap().size.height, "form fields and actions must align");
                                  assert_eq!(heights[0], heights[2], "command and form fields must align");
                                  assert_eq!(heights[1], heights[3], "compact and search fields must align");
                                  assert!(heights[1] < heights[0]);
                                  let probe = controls.field_bounds[0].clone();
                                  click(window, cx, &probe);
                              }
                              11 => {
                                  assert!(owner.read(cx).fields[0].read(cx).focus_handle(cx).is_focused(window), "form field must accept pointer focus");
                                  let probe = owner.read(cx).field_bounds[4].clone();
                                  click(window, cx, &probe);
                                  assert!(!owner.read(cx).fields[4].read(cx).focus_handle(cx).is_focused(window), "disabled field must not accept focus");
                                  let probe = owner.read(cx).loading_bounds.clone();
                                  click(window, cx, &probe);
                                  let probe = owner.read(cx).choice_bounds.clone();
                                  click(window, cx, &probe);
                              }
                              12 => {
                                  assert_eq!(owner.read(cx).loading_actions, 0, "loading actions must not fire");
                                  assert_eq!(owner.read(cx).choice_actions, 1);
                                  if variant == 3 { return true; }
                                  variant += 1;
                                  scale::set_density(if variant < 2 { scale::Density::Comfortable } else { scale::Density::Compact });
                                  set_mode(if variant % 2 == 0 { ThemeMode::Light } else { ThemeMode::Dark }, Some(window), cx);
                                 owner.update(cx, |this, cx| {
                                     this.actions = 0; this.disabled_actions = 0; this.row_actions = 0; this.closed = 0;
                                     this.loading_actions = 0; this.choice_actions = 0;
                                     this.toggled = false; this.checked = false; cx.notify();
                                     this.links.lock().unwrap().clear();
                                });
                             }
                            _ => unreachable!(),
                        }
                        false
                    }).unwrap()
                });
                if done {
                    println!("PASS: controls in both themes and densities: form/compact/search geometry, input focus, disabled fields/actions, loading actions, choice cards, keyboard, tab close, menus and links");
                    cx.update(|cx| cx.quit());
                    break;
                }
                stage = if stage == 12 { 0 } else { stage + 1 };
            }
        }).detach();
    });
}
