//! Standalone native form regression; no desktop app, project, network or user data.
use dunlin_ui::{
    component::{Disableable, Sizable, ThemeMode, input::InputState},
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

impl Render for Forms {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let fields = [
            input(&self.fields[0]).aria_label("Form input"),
            compact_input(&self.fields[1]).aria_label("Compact input"),
            code_input(&self.fields[2]).aria_label("Code input"),
            search_input(&self.fields[3], "Search input"),
            input(&self.fields[4])
                .aria_label("Disabled input")
                .disabled(true),
        ];
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
                        button(("form-action", index))
                            .label("Action")
                            .when(index == 1 || index == 3, |button| button.small())
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
                button("loading")
                    .label("Loading")
                    .loading(true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.loading_clicks += 1;
                        cx.notify();
                    })),
                self.loading_bounds.clone(),
            ))
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
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(80.)), size(px(500.), px(440.))))),
            ..dunlin_ui::window_options()
        }, cx, |window, cx| cx.new(|cx| Forms::new(window, cx))).unwrap();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let mut variant = 0;
            let mut stage = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = cx.update(|cx| window.update(cx, |_, window, cx| {
                    assert!(started.elapsed() < Duration::from_secs(20), "form smoke stalled");
                    window.refresh(); window.draw(cx).clear(cx);
                    match stage {
                        0 => {
                            let forms = owner.read(cx);
                            for index in 0..5 {
                                assert_eq!(forms.bounds[index].get().unwrap().size.height,
                                    forms.bounds[index + 5].get().unwrap().size.height,
                                    "input/action mismatch for variant {variant}, field {index}");
                            }
                            assert!(forms.bounds[1].get().unwrap().size.height < forms.bounds[0].get().unwrap().size.height);
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
                            if variant == 3 { return true; }
                            variant += 1;
                            scale::set_density(if variant < 2 { scale::Density::Comfortable } else { scale::Density::Compact });
                            set_mode(if variant % 2 == 0 { ThemeMode::Light } else { ThemeMode::Dark }, Some(window), cx);
                            owner.update(cx, |forms, cx| { forms.choices = 0; cx.notify(); });
                        }
                        _ => unreachable!(),
                    }
                    stage = if stage == 2 { 0 } else { stage + 1 };
                    false
                }).unwrap());
                if done {
                    println!("PASS: forms in both themes/densities; aligned input/action sizes, pointer focus, disabled inputs/actions, loading and choices");
                    cx.update(|cx| cx.quit());
                    break;
                }
            }
        }).detach();
    });
}
