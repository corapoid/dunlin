use super::*;

pub(super) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    dunlin_ui::settings::page()
        .child(section_label("Export and import", &p))
        .child(crate::backup::settings(this, window, cx))
        .child(section_label("Local storage · this Mac", &p))
        .child(settings_card(&p)
            .child(setting_row("Data directory", "Database, session events, search index and local preferences.",
                div().flex().items_center().gap(px(8.))
                    .child(mono_value(&crate::app::data_dir_display(), &p))
                    .child(settings_button("copy-data-directory", "Copy path", false, &p)
                        .on_click(|_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(crate::app::data_dir_display()))))
                    .into_any_element(), &p))
            .child(divider(&p))
            .child(setting_row("Local storage", "Database, write-ahead logs and cached projections. Remote storage is not included.",
                mono_value(&crate::app::format_bytes(crate::app::storage_size() as usize), &p), &p)))
        .child(section_label("Recovery · destructive actions", &p))
        .child(dunlin_ui::settings::description("Portable export is not a full history backup. Read what is included above before clearing data. Both actions below require confirmation.", p))
        .child(settings_card(&p).border_color(p.red.opacity(0.35))
            .child(setting_row("Clear conversation history", "Delete local sessions and transcripts. Projects, repository files and settings stay.",
                settings_text_button("clear-history", "Clear…", p.red, &p)
                    .on_click(cx.listener(|this, _, _, cx| this.request_clear_history(cx)))
                    .into_any_element(), &p))
            .child(divider(&p))
            .child(setting_row("Reset Dunlin to factory defaults",
                "Remove local settings, projects, groups, history and saved credentials, then restart Dunlin. Repository files and installed agents stay.",
                settings_text_button("factory-reset", "Reset…", p.red, &p)
                    .on_click(cx.listener(|this, _, window, cx| this.request_factory_reset(window, cx)))
                    .into_any_element(), &p)))
        .into_any_element()
}
