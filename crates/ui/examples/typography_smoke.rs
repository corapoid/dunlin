//! Native, no-network typography check; opens no project or conversation.
use dunlin_ui::component::{ActiveTheme, ThemeMode};
use dunlin_ui::*;

fn main() {
    application().with_assets(assets::AllAssets).run(|cx| {
        dunlin_ui::init(cx);
        let installed = cx.text_system().all_font_names();
        assert!(installed.iter().any(|name| name == "Geist"));
        assert!(installed.iter().any(|name| name == "Geist Mono"));
        let alternate = ["Menlo", "Monaco", "Courier New"].into_iter()
            .find(|name| installed.iter().any(|font| font == name))
            .expect("install an alternate code font to run this optional check");
        let alternate_ui = ["Helvetica Neue", "Arial", "Times New Roman"].into_iter()
            .find(|name| installed.iter().any(|font| font == name))
            .expect("install an alternate interface font to run this optional check");
        dunlin_ui::theme::set_font_families(alternate_ui, alternate, cx);
        dunlin_ui::theme::set_code_font_size(17.0, cx);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            dunlin_ui::theme::set_mode(mode, None, cx);
            assert_eq!(dunlin_ui::theme::code_font().as_ref(), alternate);
            assert_eq!(cx.theme().mono_font_family.as_ref(), alternate);
            assert_eq!(cx.theme().mono_font_size, px(17.0));
            assert_eq!(cx.theme().font_family.as_ref(), alternate_ui);
            assert_eq!(dunlin_ui::theme::ui_font().as_ref(), alternate_ui);
        }
        let font = cx.text_system().resolve_font(&font(alternate));
        let small = cx.text_system().advance(font, px(10.0), 'W').unwrap();
        let large = cx.text_system().advance(font, px(20.0), 'W').unwrap();
        assert!(large.width > small.width);
        dunlin_ui::theme::set_font_families("Geist", "Geist Mono", cx);
        dunlin_ui::theme::set_code_font_size(13.0, cx);
        println!("PASS: bundled and installed fonts resolve; live code size and family survive light/dark switches.");
        cx.quit();
    });
}
