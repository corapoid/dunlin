//! Isolated paint regression gallery; no database, agent, network or OS input.
use dunlin_ui::{
    component::{Selectable, ThemeMode},
    prelude::FluentBuilder as _,
    *,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

type Probe = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Default)]
struct Surfaces {
    bounds: [Probe; 14],
    action: Probe,
    scroll: ScrollHandle,
    expanded: bool,
    broken: bool,
}

fn measured(surface: Div, geometry: SurfaceGeometry, probe: &Probe) -> Div {
    let probe = probe.clone();
    surface.relative().child(
        canvas(
            move |bounds, _, _| probe.set(Some(geometry.outer_bounds(bounds))),
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .size_full(),
    )
}

impl Render for Surfaces {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let action = self.action.clone();
        let shell = SurfaceGeometry::SETTINGS
            .paint(p.subtle, p.border_strong)
            .h(px(130.))
            .flex()
            .overflow_hidden()
            .child(div().w(px(96.)).p(px(12.)).child("Settings"))
            .child(
                SurfaceGeometry::SETTINGS_CONTENT
                    .paint(p.bg, p.border)
                    .flex_1()
                    .m(px(8.))
                    .p(px(16.))
                    .child("Transparent navigation; one shell owns the corners"),
            )
            .when(self.broken, |shell| {
                // Negative control: reproduce the old square navigation fill.
                shell.child(div().absolute().inset_0().bg(p.subtle))
            });
        let card = dunlin_ui::settings::card(p)
            .h(px(88.))
            .overflow_hidden()
            .child(
                dunlin_ui::settings::card_header(p)
                    .h(px(28.))
                    .px(px(12.))
                    .child("Settings / billing header"),
            )
            .child(div().p(px(12.)).child("Card body"));
        let code = code_surface(p)
            .h(px(88.))
            .overflow_hidden()
            .child(code_header(p).child("DIFF"))
            .child(div().p(px(10.)).child("+ native rounded code header"));
        let table = SurfaceGeometry::TABLE
            .paint(p.card, p.border)
            .h(px(88.))
            .child(
                SurfaceGeometry::TABLE
                    .round_child(div(), SurfaceEdges::Top)
                    .h(px(28.))
                    .bg(p.muted)
                    .px(px(6.))
                    .child("Markdown table header"),
            )
            .child(div().p(px(6.)).child("Square internal joins"));
        let row = card_surface(p).h(px(88.)).overflow_hidden().child(
            card_action_row("surface-card-action", "Expand card", self.expanded)
                .h(px(if self.expanded { 44. } else { 88. }))
                .label("Hover, focus and selected card edges")
                .selected(self.expanded)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expanded = !this.expanded;
                    cx.notify();
                }))
                .child(
                    canvas(
                        move |bounds, _, _| action.set(Some(bounds)),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0()
                    .size_full(),
                ),
        );
        let capsule = segmented_bar([(0.0001, p.red), (1., p.blue), (0.0001, p.green)]).h(px(8.));
        let single_capsule = segmented_bar([(1., p.violet)]).h(px(8.));
        let scroller = SurfaceGeometry::SETTINGS_CARD
            .paint(p.card, p.border)
            .h(px(110.))
            .child(
                dunlin_ui::settings::card_header(p)
                    .h(px(28.))
                    .child("Scroll viewport"),
            )
            .child(
                div()
                    .id("surface-scroll")
                    .h(px(68.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .h(px(500.))
                            .bg(p.subtle)
                            .p(px(12.))
                            .child("Long content"),
                    ),
            )
            // The scroll viewport ends above the bottom curves, not behind them.
            .pb(px(SurfaceGeometry::SETTINGS_CARD.inner_radius()));
        let dialog = SurfaceGeometry::DIALOG
            .paint(p.card, p.border_strong)
            .h(px(88.))
            .p(px(16.))
            .child("Dialog, shared with native backdrop");
        let node = SurfaceGeometry::GRAPH_NODE
            .paint(p.card, p.border)
            .h(px(88.))
            .child(
                SurfaceGeometry::GRAPH_NODE
                    .round_child(div(), SurfaceEdges::Top)
                    .h(px(30.))
                    .bg(p.subtle)
                    .px(px(10.))
                    .child("Running graph node"),
            )
            .child(
                SurfaceGeometry::GRAPH_NODE
                    .bottom_indicator(div())
                    .h(px(2.))
                    .bg(p.muted)
                    .child(div().h_full().w(relative(0.62)).rounded_full().bg(p.blue)),
            );
        div()
            .size_full()
            .bg(p.bg)
            .text_color(p.fg)
            .p(px(20.5))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(measured(shell, SurfaceGeometry::SETTINGS, &self.bounds[0]))
            .child(div().flex().gap(px(12.)).children([
                measured(card, SurfaceGeometry::SETTINGS_CARD, &self.bounds[1]).flex_1(),
                measured(code, SurfaceGeometry::CARD, &self.bounds[2]).flex_1(),
                measured(table, SurfaceGeometry::TABLE, &self.bounds[3]).flex_1(),
            ]))
            .child(measured(row, SurfaceGeometry::CARD, &self.bounds[4]))
            .child(measured(
                capsule,
                SurfaceGeometry::new(radii::CODE, 0.),
                &self.bounds[5],
            ))
            .child(measured(
                single_capsule,
                SurfaceGeometry::new(radii::CODE, 0.),
                &self.bounds[6],
            ))
            .child(
                div().flex().gap(px(12.)).children(
                    [0., 0.1, 35., 100.]
                        .into_iter()
                        .enumerate()
                        .map(|(index, percent)| {
                            measured(
                                progress_bar(percent, p.blue, p.muted).h(px(8.)),
                                SurfaceGeometry::new(radii::CODE, 0.),
                                &self.bounds[10 + index],
                            )
                            .flex_1()
                        }),
                ),
            )
            .child(measured(
                scroller,
                SurfaceGeometry::SETTINGS_CARD,
                &self.bounds[7],
            ))
            .child(div().flex().gap(px(12.)).children([
                measured(dialog, SurfaceGeometry::DIALOG, &self.bounds[8]).flex_1(),
                measured(node, SurfaceGeometry::GRAPH_NODE, &self.bounds[9]).flex_1(),
            ]))
    }
}

pub(crate) fn run() {
    application().with_assets(assets::AllAssets).run(|cx| {
        dunlin_ui::init(cx);
        let (handle, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.), px(80.)), size(px(1000.), px(700.))))),
            ..dunlin_ui::window_options()
        }, cx, |_, cx| cx.new(|_| Surfaces::default())).unwrap();
        cx.spawn(async move |cx| {
            let started = Instant::now();
            for variant in 0..8 {
                handle.update(cx, |_, window, cx| {
                    set_mode(if variant % 2 == 0 { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
                    scale::set_density(if variant % 4 < 2 { scale::Density::Comfortable } else { scale::Density::Compact });
                    window.resize(size(px(if variant < 4 { 1000. } else { 780. }), px(700.)));
                    owner.update(cx, |this, cx| { this.expanded = false; this.broken = false; cx.notify(); });
                }).unwrap();
                for stage in 0..6 {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    cx.update(|cx| handle.update(cx, |_, window, cx| {
                        assert!(started.elapsed() < Duration::from_secs(30), "surface smoke stalled");
                        window.refresh(); window.draw(cx).clear(cx);
                        let this = owner.read(cx);
                        for (index, probe) in this.bounds.iter().enumerate() {
                            let bounds = probe.get().expect("surface must be measured");
                            if this.broken && index == 0 {
                                assert!(surface_test::check_painted_corners(window, bounds, true).is_err(), "checker must catch the original settings regression");
                            } else {
                                surface_test::assert_painted_corners(window, bounds, true, &format!("surface-{index}, variant-{variant}, stage-{stage}"));
                            }
                        }
                        let position = this.action.get().unwrap().center();
                        match stage {
                            0 => { window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx); }
                            1 => { window.blur(cx); window.focus_next(cx); }
                            2 => { owner.update(cx, |this, cx| { this.expanded = true; this.scroll.set_offset(point(px(0.), -px(220.))); cx.notify(); }); }
                            3 => { owner.update(cx, |this, cx| { this.broken = true; cx.notify(); }); }
                            4 => { owner.update(cx, |this, cx| { this.broken = false; cx.notify(); }); }
                            _ => {}
                        }
                    }).unwrap());
                }
            }
            cx.update(|cx| { println!("PASS: surface paint in both themes/densities/sizes; hover, focus, expanded, scroll, fractional layout and negative control"); cx.quit(); });
        }).detach();
    });
}
