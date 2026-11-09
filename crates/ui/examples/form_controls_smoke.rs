//! Standalone native form regression; no desktop app, project, network or user data.
use dunlin_ui::{
    component::{
        ActiveTheme, Disableable, Sizable, ThemeMode, button::ButtonVariants, input::InputState,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

type Probe = Rc<Cell<Option<Bounds<Pixels>>>>;

struct Forms {
    fields: Vec<Entity<InputState>>,
    bounds: [Probe; 10],
    choice_bounds: Probe,
    loading_bounds: Probe,
    dynamic_bounds: Probe,
    navigation_bounds: Probe,
    expanded_label: bool,
    initial_action_size: Option<Size<Pixels>>,
    settings_recipe: bool,
    choices: usize,
    disabled_clicks: usize,
    loading_clicks: usize,
}

impl Forms {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            fields: (0..5)
                .map(|_| cx.new(|cx| InputState::new(window, cx)))
                .collect(),
            bounds: Default::default(),
            choice_bounds: Default::default(),
            loading_bounds: Default::default(),
            dynamic_bounds: Default::default(),
            navigation_bounds: Default::default(),
            expanded_label: false,
            initial_action_size: None,
            settings_recipe: false,
            choices: 0,
            disabled_clicks: 0,
            loading_clicks: 0,
        }
    }
}

fn measured(view: impl IntoElement, probe: Probe) -> Div {
    div().relative().flex().child(view).child(
        canvas(move |bounds, _, _| probe.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .inset_0(),
    )
}

fn painted_fill(window: &Window, bounds: Bounds<Pixels>) -> Option<Background> {
    let bounds = bounds.scale(window.scale_factor());
    window
        .painted_quads()
        .iter()
        .find(|quad| {
            (quad.bounds.left() - bounds.left()).0.abs() < 0.6
                && (quad.bounds.top() - bounds.top()).0.abs() < 0.6
                && (quad.bounds.right() - bounds.right()).0.abs() < 0.6
                && (quad.bounds.bottom() - bounds.bottom()).0.abs() < 0.6
                && !quad.background.is_transparent()
        })
        .map(|quad| quad.background)
}

fn expected_fill(cx: &App, action: usize, phase: usize) -> Background {
    let theme = cx.theme();
    match (action, phase) {
        (0, 0) => palette(cx).subtle.into(),
        (0, 7) => palette(cx).accent.into(),
        (0, _) => palette(cx).hover.into(),
        (2, 0) => theme.tokens.button_primary.into(),
        (2, 7) => theme.tokens.button_primary_active.into(),
        (2, _) => theme.tokens.button_primary_hover.into(),
        (3, 0) => theme.tokens.danger.background.opacity(0.1).opacity(0.4),
        (3, 7) => theme
            .tokens
            .danger_active
            .background
            .opacity(0.4)
            .opacity(0.3),
        (3, _) => theme
            .tokens
            .danger_hover
            .background
            .opacity(0.2)
            .opacity(0.4),
        _ => unreachable!(),
    }
}

impl Render for Forms {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let fields = if self.settings_recipe {
            [
                settings::input(&self.fields[0], p).aria_label("Form input"),
                settings::input(&self.fields[1], p).aria_label("Compact input"),
                settings::input(&self.fields[2], p)
                    .font_family(theme::code_font())
                    .aria_label("Code input"),
                settings::search(&self.fields[3], "Search input", p),
                settings::input(&self.fields[4], p)
                    .aria_label("Disabled input")
                    .disabled(true),
            ]
        } else {
            [
                input(&self.fields[0]).aria_label("Form input"),
                compact_input(&self.fields[1]).aria_label("Compact input"),
                code_input(&self.fields[2]).aria_label("Code input"),
                search_input(&self.fields[3], "Search input"),
                input(&self.fields[4])
                    .aria_label("Disabled input")
                    .disabled(true),
            ]
        };
        div()
            .size_full()
            .p(px(20.))
            .bg(p.card)
            .text_color(p.fg)
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(fields.into_iter().enumerate().map(|(index, field)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(measured(field.w(px(280.)), self.bounds[index].clone()))
                    .child(measured(
                        (if self.settings_recipe {
                            settings::button(("form-action", index))
                        } else {
                            action_button(("form-action", index))
                        })
                        .label("Action")
                        .when(index == 2, |button| button.primary())
                        .when(index == 3, |button| button.danger().outline())
                        .when(
                            !self.settings_recipe && (index == 1 || index == 3),
                            |button| button.small(),
                        )
                        .disabled(index == 4)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index == 4 {
                                this.disabled_clicks += 1;
                            }
                            cx.notify();
                        })),
                        self.bounds[index + 5].clone(),
                    ))
            }))
            .child(measured(
                choice_card("choice", "Choose permission mode", self.choices > 0, p)
                    .w(px(380.))
                    .child(choice_indicator(self.choices > 0, p))
                    .child("Choice")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.choices += 1;
                        cx.notify();
                    })),
                self.choice_bounds.clone(),
            ))
            .child(measured(
                (if self.settings_recipe {
                    settings::button("loading")
                } else {
                    action_button("loading")
                })
                .label("Loading")
                .loading(true)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.loading_clicks += 1;
                    cx.notify();
                })),
                self.loading_bounds.clone(),
            ))
            .child(
                div().flex().child(measured(
                    (if self.settings_recipe {
                        settings::button("dynamic-label")
                    } else {
                        action_button("dynamic-label")
                    })
                    .label(if self.expanded_label {
                        "Updated globally"
                    } else {
                        "Install"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded_label = !this.expanded_label;
                        cx.notify();
                    })),
                    self.dynamic_bounds.clone(),
                )),
            )
            .child(div().flex().child(measured(
                button("quiet-navigation").label("Navigation"),
                self.navigation_bounds.clone(),
            )))
    }
}

fn click(window: &mut Window, cx: &mut App, probe: &Probe) {
    let position = probe.get().expect("control must be laid out").center();
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
}

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        dunlin_ui::init(cx);
        set_mode(ThemeMode::Light, None, cx);
        let (window, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(80.)), size(px(500.), px(500.))))),
            ..dunlin_ui::window_options()
        }, cx, |window, cx| cx.new(|cx| Forms::new(window, cx))).unwrap();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let mut variant = 0;
            let mut stage = 0;
            let mut action_index = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| window.update(cx, |_, window, cx| {
                    assert!(started.elapsed() < Duration::from_secs(40), "form smoke stalled");
                    if stage == 0 { window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: point(px(1.), px(1.)), ..Default::default() }), cx); }
                    window.refresh(); window.draw(cx).clear(cx);
                    match stage {
                        0 => {
                            let forms = owner.read(cx);
                            assert_eq!(painted_fill(window, forms.bounds[5].get().unwrap()), Some(palette(cx).subtle.into()), "ordinary action must use the current theme's solid fill");
                            assert!(painted_fill(window, forms.navigation_bounds.get().unwrap()).is_none(), "action styling must never leak into quiet navigation");
                            for index in 0..5 {
                                assert_eq!(forms.bounds[index].get().unwrap().size.height,
                                    forms.bounds[index + 5].get().unwrap().size.height,
                                    "input/action mismatch for variant {variant}, field {index}");
                            }
                             if forms.settings_recipe {
                                  assert!((forms.bounds[0].get().unwrap().size.height - px(scale::control(settings::Tokens::CONTROL_HEIGHT))).abs() <= px(0.5), "settings height must match the recipe after device-pixel snapping");
                                 assert_eq!(painted_fill(window, forms.bounds[0].get().unwrap()), Some(settings::Tokens::new(palette(cx)).field.into()));
                             } else {
                                 assert!(forms.bounds[1].get().unwrap().size.height < forms.bounds[0].get().unwrap().size.height);
                             }
                            let probe = forms.bounds[0].clone(); click(window, cx, &probe);
                        }
                        1 => {
                            assert!(owner.read(cx).fields[0].read(cx).focus_handle(cx).is_focused(window));
                            let probe = owner.read(cx).bounds[4].clone(); click(window, cx, &probe);
                            assert!(!owner.read(cx).fields[4].read(cx).focus_handle(cx).is_focused(window));
                            let probe = owner.read(cx).bounds[9].clone(); click(window, cx, &probe);
                            let probe = owner.read(cx).loading_bounds.clone(); click(window, cx, &probe);
                            let probe = owner.read(cx).choice_bounds.clone(); click(window, cx, &probe);
                        }
                        2 => {
                            let forms = owner.read(cx);
                            assert_eq!((forms.disabled_clicks, forms.loading_clicks, forms.choices), (0, 0, 1));
                            let initial = forms.dynamic_bounds.get().unwrap().size;
                            let probe = forms.dynamic_bounds.clone();
                            owner.update(cx, |forms, _| forms.initial_action_size = Some(initial));
                            click(window, cx, &probe);
                        }
                        3..=5 => {
                            let forms = owner.read(cx);
                            let current = forms.dynamic_bounds.get().unwrap().size;
                            let initial = forms.initial_action_size.unwrap();
                            assert_eq!(current.height, initial.height, "dynamic label must not change action height");
                            if stage < 5 { stage += 1; return false; }
                            assert!(current.width > initial.width, "action must resize to its updated label");
                            let bounds = forms.bounds[5 + action_index].get().unwrap();
                            assert_eq!(painted_fill(window, bounds), Some(expected_fill(cx, action_index, 0)), "idle action fill");
                            let position = bounds.center();
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx);
                        }
                        6 => {
                            let bounds = owner.read(cx).bounds[5 + action_index].get().unwrap();
                            assert_eq!(painted_fill(window, bounds), Some(expected_fill(cx, action_index, 6)), "hover must use the theme's hover fill");
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: bounds.center(), button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }
                        7 => {
                            let bounds = owner.read(cx).bounds[5 + action_index].get().unwrap();
                            assert_eq!(painted_fill(window, bounds), Some(expected_fill(cx, action_index, 7)), "pressed fill must remain distinct from hover");
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: bounds.center(), button: MouseButton::Left, click_count: 1, ..Default::default() }), cx);
                        }
                        8 => {
                            let bounds = owner.read(cx).bounds[5 + action_index].get().unwrap();
                            assert_eq!(painted_fill(window, bounds), Some(expected_fill(cx, action_index, 8)), "release must restore hover, not leave a stuck pressed state");
                            if action_index != 3 { action_index = if action_index == 0 { 2 } else { 3 }; stage = 5; return false; }
                            action_index = 0;
                             if variant == 7 { return true; }
                            variant += 1;
                             scale::set_density(if variant % 4 < 2 { scale::Density::Comfortable } else { scale::Density::Compact });
                            set_mode(if variant % 2 == 0 { ThemeMode::Light } else { ThemeMode::Dark }, Some(window), cx);
                             owner.update(cx, |forms, cx| { forms.settings_recipe = variant >= 4; forms.choices = 0; forms.expanded_label = false; cx.notify(); });
                        }
                        _ => unreachable!(),
                    }
                    stage = if stage == 8 { 0 } else { stage + 1 };
                    false
                }).unwrap());
                if done {
                    println!("PASS: forms in both themes/densities; geometry, focus, disabled/loading, choices, animated labels, isolated navigation and idle/hover/pressed/released action colors");
                    cx.update(|cx| cx.quit());
                    break;
                }
            }
        }).detach();
    });
}
