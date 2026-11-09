//! Isolated native text-motion regression, without projects, agents or user data.
use dunlin_ui::{component::ThemeMode, *};
use std::{cell::Cell, rc::Rc, time::Duration};

type Probe = Rc<Cell<Option<Bounds<Pixels>>>>;

struct Labels {
    values: [String; 6],
    scope: usize,
    reduced: bool,
    bounds: [Probe; 6],
    reference: [Probe; 6],
    renders: Rc<Cell<usize>>,
}

fn measured(child: impl IntoElement, probe: Probe) -> Div {
    div().relative().flex().flex_shrink_0().child(child).child(
        canvas(move |bounds, _, _| probe.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .inset_0(),
    )
}

impl Labels {
    fn assert_settled(&self) {
        for i in 0..6 {
            assert_eq!(
                self.bounds[i].get().unwrap().size,
                self.reference[i].get().unwrap().size,
                "label {i} must match native text layout: {}",
                self.values[i]
            );
        }
    }
}

impl Render for Labels {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let p = palette(cx);
        div()
            .size_full()
            .p(px(24.))
            .bg(p.card)
            .text_color(p.fg)
            .text_size(px(13.))
            .font_family(theme::ui_font())
            .flex()
            .flex_col()
            .gap(px(18.))
            .children(self.values.iter().enumerate().map(|(i, value)| {
                let mut text = MorphText::new(("smoke-label", i), value.clone())
                    .scope(self.scope.to_string())
                    .reduce_motion(self.reduced);
                if i != 1 {
                    text = text.numeric();
                }
                div()
                    .flex()
                    .items_center()
                    .gap(px(32.))
                    .child(measured(text, self.bounds[i].clone()))
                    .child(measured(value.clone(), self.reference[i].clone()))
            }))
    }
}

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        dunlin_ui::init(cx);
        cx.set_reduce_motion(false);
        set_mode(ThemeMode::Light, None, cx);
        let (window, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(80.), px(80.)), size(px(600.), px(330.)),
            ))),
            ..dunlin_ui::window_options()
        }, cx, |_, cx| cx.new(|_| Labels {
            values: ["$0.09", "RUNNING", "9", "99", "5", "19%"].map(str::to_owned),
            scope: 0,
            reduced: false,
            bounds: Default::default(),
            reference: Default::default(),
            renders: Default::default(),
        })).unwrap();
        cx.activate(true);
        cx.spawn(async move |cx| {
            let mut idle_renders = 0;
            for stage in 0..9 {
                cx.background_executor().timer(Duration::from_millis(450)).await;
                cx.update(|cx| window.update(cx, |_, window, cx| {
                    // Draw before assertions, but do not force an invalidation:
                    // settled components must stop scheduling animation frames.
                    window.draw(cx).clear(cx);
                    owner.read(cx).assert_settled();
                    match stage {
                        0 => {
                            owner.update(cx, |labels, cx| {
                                labels.values = ["$0.12", "DONE", "10", "100", "6", "21%"].map(str::to_owned);
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx).clear(cx);
                        }
                        1 => idle_renders = owner.read(cx).renders.get(),
                        2 => {
                            assert!(owner.read(cx).renders.get() <= idle_renders + 1,
                                "idle text keeps rendering beyond the explicit snapshot draw");
                            owner.update(cx, |labels, cx| {
                                labels.reduced = true;
                                labels.values = ["$100.00", "FAILED", "123", "9999", "100", "99%"].map(str::to_owned);
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx).clear(cx);
                            owner.read(cx).assert_settled();
                        }
                        3 => {
                            cx.set_reduce_motion(true);
                            owner.update(cx, |labels, cx| {
                                labels.reduced = false;
                                labels.values = ["$1.00", "WAITING", "1", "1", "1", "1%"].map(str::to_owned);
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx).clear(cx);
                            owner.read(cx).assert_settled();
                        }
                        4 => {
                            cx.set_reduce_motion(false);
                            owner.update(cx, |labels, cx| {
                                labels.scope += 1;
                                labels.values = ["$99.99", "OFFLINE", "99", "99", "99", "99%"].map(str::to_owned);
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx).clear(cx);
                            owner.read(cx).assert_settled();
                        }
                        5 => {
                            set_mode(ThemeMode::Dark, Some(window), cx);
                            owner.update(cx, |labels, cx| {
                                labels.values = ["$0.01", "Ready 👩‍💻", "0", "0", "0", "0%"].map(str::to_owned);
                                cx.notify();
                            });
                            window.refresh();
                            window.draw(cx).clear(cx);
                        }
                        6 => {
                            // Force many updates before a roll can finish.
                            for i in 1..30 {
                                owner.update(cx, |labels, cx| {
                                    labels.values[0] = format!("${i}.00");
                                    cx.notify();
                                });
                                window.refresh();
                                window.draw(cx).clear(cx);
                            }
                        }
                        7 => idle_renders = owner.read(cx).renders.get(),
                        8 => assert!(owner.read(cx).renders.get() <= idle_renders + 1),
                        _ => unreachable!(),
                    }
                }).unwrap());
            }
            println!("PASS: six native labels, both themes, native font metrics, coalescing, scope changes, app/system Reduce Motion and idle frames");
            cx.update(|cx| cx.quit());
        }).detach();
    });
}
