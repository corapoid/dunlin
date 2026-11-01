use sha2::{Digest, Sha256};
use syntect::{
    dumps,
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::SyntaxSet,
};

#[test]
fn migrated_default_and_extra_syntaxes_decode_all_lazy_contexts() {
    for (index, set) in [
        SyntaxSet::load_defaults_newlines(),
        SyntaxSet::load_defaults_nonewlines(),
        two_face::syntax::extra_newlines(),
        two_face::syntax::extra_no_newlines(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(set.syntaxes().len(), if index < 2 { 75 } else { 199 });
        // Traverses every lazy context, not just the outer dump envelope.
        let _ = set.find_unlinked_contexts();
        for extension in ["rs", "js", "md", "py"] {
            let syntax = set.find_syntax_by_extension(extension).unwrap();
            let themes = ThemeSet::load_defaults();
            let mut highlighter =
                HighlightLines::new(syntax, themes.themes.values().next().unwrap());
            let source = "let value = \"zażółć 🦀\"; // source must survive\n";
            let regions = highlighter.highlight_line(source, &set).unwrap();
            assert_eq!(
                regions.iter().map(|(_, text)| *text).collect::<String>(),
                source
            );
        }
    }
}

#[test]
fn all_migrated_lazy_themes_and_license_records_decode() {
    let lazy = two_face::theme::extra();
    let themes: ThemeSet = (&lazy).into();
    assert!(themes.themes.len() > 10);
    for theme in themes.themes.values() {
        let bytes = dumps::dump_binary(theme);
        let roundtrip: Theme = dumps::from_reader(&bytes[..]).unwrap();
        assert_eq!(theme.name, roundtrip.name);
    }
    let acknowledgements = two_face::acknowledgement::listing().to_md();
    assert_eq!(
        format!("{:x}", Sha256::digest(acknowledgements.as_bytes())),
        "693d56ce18824f0c8db2218677c2cda58cee2c2b604c83c4c3ebea5705a4e450",
        "Migration must preserve the exact upstream license text"
    );
    assert!(acknowledgements.len() > 10000);
    assert!(acknowledgements.contains("Copyright") || acknowledgements.contains("copyright"));
}

#[test]
fn legacy_or_malformed_dumps_fail_closed() {
    assert!(dumps::from_uncompressed_data::<ThemeSet>(b"legacy bincode data").is_err());
    assert!(dumps::from_uncompressed_data::<ThemeSet>(b"DLSYNP01\xc1").is_err());
    assert!(dumps::from_reader::<ThemeSet, _>(&b"not a zlib stream"[..]).is_err());
}
