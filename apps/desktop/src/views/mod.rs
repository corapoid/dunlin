pub(crate) mod chat;
pub(crate) mod chat_controls;
pub(crate) mod graph;
pub(crate) mod palette;
pub(crate) mod settings;
pub(crate) mod sidebar;
pub(crate) mod titlebar;
pub(crate) mod trace;

/// Empty conversations have no execution data, not a sample agent run.
fn empty_activity(
    title: &'static str,
    description: &'static str,
    icon_name: dunlin_ui::assets::IconName,
    cx: &mut dunlin_ui::Context<crate::app::DunlinApp>,
) -> dunlin_ui::AnyElement {
    use dunlin_ui::component::Icon;
    use dunlin_ui::*;
    let p = palette(cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .items_center()
        .justify_center()
        .gap(px(10.))
        .p(px(24.))
        .bg(p.bg)
        .child(Icon::new(icon_name).size(px(28.)).text_color(p.muted_fg))
        .child(
            div()
                .text_size(px(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(p.muted_fg)
                .text_center()
                .child(description),
        )
        .into_any_element()
}
