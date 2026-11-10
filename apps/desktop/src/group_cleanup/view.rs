use super::*;
use dunlin_ui::component::{Disableable, button::ButtonVariants};
use dunlin_ui::prelude::FluentBuilder as _;

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(dialog) = &this.group_cleanup.dialog else {
        return div().into_any_element();
    };
    let (count, bytes) = dialog.total();
    let reviewing = dialog.phase == Phase::Reviewing;
    let status = match dialog.phase {
        Phase::Inspecting => "Inspecting local repositories and measuring folders…".to_owned(),
        Phase::Reviewing | Phase::Confirming => format!(
            "{count} selected · approximately {} · {} skipped",
            crate::app::format_size(bytes),
            dialog.skipped.len()
        ),
        Phase::Moving => "Moving selected folders to system Trash…".to_owned(),
        Phase::Finished => format!(
            "{} folders moved to Trash · {} skipped",
            dialog.moved.len(),
            dialog.skipped.len()
        ),
    };
    let mut body = div().flex_1().min_h(px(0.)).flex().flex_col();
    if dialog.row_count() == 0 {
        body = body.child(div().p(px(16.)).text_color(p.muted_fg).child(match dialog.phase {
            Phase::Inspecting => "Scanning runs in the background. You can cancel without changing any files.",
            Phase::Reviewing => "No safely removable ignored dependency, cache or build folders found.",
            _ => "No folders were moved.",
        }));
    } else {
        body = body.child(
            uniform_list(
                "cleanup-folder-list",
                dialog.row_count(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    render_rows(this, range, cx)
                }),
            )
            .track_scroll(&dialog.scroll)
            .flex_1()
            .min_h(px(0.)),
        );
    }
    let panel = dialog_surface(p)
        .id("project-cleanup-dialog")
        .relative()
        .children(crate::gui_probe::marker("project-cleanup-dialog"))
        .role(Role::Dialog)
        .aria_label(dialog.title.clone())
        .tab_group()
        .track_focus(&dialog.focus)
        .w(px(760.)).max_w_full().h(px(620.)).max_h(relative(0.9))
        .p(px(0.)).gap(px(0.)).overflow_hidden()
        .child(crate::effects::panel_probe(&this.overlay_backdrop, SurfaceGeometry::DIALOG, true))
        .child(div().p(px(16.)).flex_shrink_0().flex().flex_col().gap(px(8.))
            .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(dialog.title.clone()))
            .child(div().text_size(px(12.)).text_color(p.muted_fg).child(status))
            .child(dunlin_ui::settings::notice("System Trash only. Disk space is reclaimed after you empty Trash yourself. Dunlin never empties it.", p))
            .when(reviewing, |view| view
                .child(dunlin_ui::settings::description("Only recognized generated folders ignored by Git and containing no tracked files are offered. Sizes are estimates, not guaranteed disk savings. Stop external build/install tools before continuing.", p))
                .child(div().flex().gap(px(8.))
                    .child(dunlin_ui::toolbar_button("cleanup-select-all").label("Select all").relative().children(crate::gui_probe::marker("cleanup-select-all")).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(dialog) = &mut this.group_cleanup.dialog && dialog.phase == Phase::Reviewing { dialog.selected.fill(true); }
                        cx.notify();
                    })))
                    .child(dunlin_ui::toolbar_button("cleanup-select-none").label("Select none").relative().children(crate::gui_probe::marker("cleanup-select-none")).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(dialog) = &mut this.group_cleanup.dialog && dialog.phase == Phase::Reviewing { dialog.selected.fill(false); }
                        cx.notify();
                    }))))))
        .child(body)
        .when_some(dialog.error.clone(), |view, error| view.child(div().px(px(16.)).py(px(8.)).text_size(px(12.)).text_color(p.red).child(error)))
        .child(div().p(px(16.)).flex_shrink_0().border_t_1().border_color(p.border).flex().items_center().justify_end().gap(px(8.))
            .child(dunlin_ui::settings::button("cleanup-close")
                .relative().children(crate::gui_probe::marker("cleanup-close"))
                .label(if dialog.phase == Phase::Finished { "Done" } else { "Cancel" })
                .disabled(dialog.locked())
                .on_click(cx.listener(|this, _, window, cx| this.close_cleanup(window, cx))))
            .when(dialog.phase != Phase::Finished, |view| view.child(
                dunlin_ui::settings::button("cleanup-confirm").primary().label("Move selected to Trash…")
                    .relative().children(crate::gui_probe::marker("cleanup-confirm"))
                    .disabled(!reviewing || count == 0)
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_cleanup(window, cx))))));
    div()
        .id("project-cleanup-overlay")
        .absolute()
        .inset_0()
        .size_full()
        .occlude()
        .p(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(panel)
        .into_any_element()
}

fn render_rows(
    this: &DunlinApp,
    range: std::ops::Range<usize>,
    cx: &mut Context<DunlinApp>,
) -> Vec<AnyElement> {
    let Some(dialog) = &this.group_cleanup.dialog else {
        return Vec::new();
    };
    let p = palette(cx);
    let finished = dialog.phase == Phase::Finished;
    let active_count = if finished {
        dialog.moved.len()
    } else {
        dialog.folders.len()
    };
    range
        .filter_map(|index| {
            let row = div()
                .id(("cleanup-row", index))
                .h(px(68.))
                .px(px(16.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .border_b_1()
                .border_color(p.border)
                .text_size(px(12.));
            let detail = if index < active_count {
                if finished {
                    dialog.moved.get(index)?.display().to_string()
                } else {
                    dialog.folders.get(index)?.path.display().to_string()
                }
            } else {
                let skipped = dialog.skipped.get(index - active_count)?;
                format!("{}\n{}", skipped.path.display(), skipped.reason)
            };
            let row = row.tooltip(move |window, cx| {
                component::tooltip::Tooltip::new(detail.clone()).build(window, cx)
            });
            if index < active_count {
                if finished {
                    let path = dialog.moved.get(index)?;
                    Some(
                        row.child(div().text_color(p.fg).child("Moved to Trash"))
                            .child(
                                div()
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(path.display().to_string().escape_debug().to_string()),
                            )
                            .into_any_element(),
                    )
                } else {
                    let folder = dialog.folders.get(index)?;
                    let path = folder.path.display().to_string().escape_debug().to_string();
                    Some(
                        row.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .relative()
                                        .children(crate::gui_probe::marker_named(format!(
                                            "cleanup-folder-{index}"
                                        )))
                                        .child(
                                            dunlin_ui::checkbox(
                                                ("cleanup-folder", index),
                                                format!("Include {path}"),
                                                dialog.selected[index],
                                            )
                                            .label(
                                                folder
                                                    .path
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .into_owned(),
                                            )
                                            .disabled(dialog.phase != Phase::Reviewing)
                                            .on_change(cx.listener(
                                                move |this, checked: &bool, _, cx| {
                                                    if let Some(dialog) =
                                                        &mut this.group_cleanup.dialog
                                                        && dialog.phase == Phase::Reviewing
                                                        && let Some(selected) =
                                                            dialog.selected.get_mut(index)
                                                    {
                                                        *selected = *checked;
                                                    }
                                                    cx.notify();
                                                },
                                            )),
                                        ),
                                )
                                .child(div().flex_1())
                                .child(div().text_color(p.muted_fg).child(format!(
                                    "{} · ≈ {}",
                                    folder.kind.label(),
                                    crate::app::format_size(folder.bytes)
                                ))),
                        )
                        .child(div().text_color(p.muted_fg).truncate().child(path))
                        .into_any_element(),
                    )
                }
            } else {
                let skipped = dialog.skipped.get(index - active_count)?;
                Some(
                    row.child(
                        div()
                            .text_color(p.amber)
                            .truncate()
                            .child(format!("Skipped · {}", skipped.reason)),
                    )
                    .child(
                        div().text_color(p.muted_fg).truncate().child(
                            skipped
                                .path
                                .display()
                                .to_string()
                                .escape_debug()
                                .to_string(),
                        ),
                    )
                    .into_any_element(),
                )
            }
        })
        .collect()
}
