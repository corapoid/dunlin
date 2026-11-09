//! One-time converter for checksum-pinned, trusted upstream asset files ONLY.
//! Run in an isolated non-workspace Cargo project against the original syntect
//! 5.3.0/two-face 0.5.2+bat-0.26.1. Its legacy reader is NOT linked into Dunlin or run in CI.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use syntect::{
    dumps,
    highlighting::{Theme, ThemeSet},
    parsing::{
        SyntaxSet,
        syntax_definition::{Context, ContextId},
    },
};

#[derive(Serialize, Deserialize)]
struct LazyContexts {
    context_ids: BTreeMap<String, ContextId>,
    contexts: Vec<Context>,
}

fn pack<T: Serialize>(value: &T, compressed: bool) -> Vec<u8> {
    let mut data = b"DLSYNP01".to_vec();
    rmp_serde::encode::write_named(&mut data, value).unwrap();
    if !compressed {
        return data;
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&data).unwrap();
    encoder.finish().unwrap()
}

fn bytes(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|n| u8::try_from(n.as_u64().unwrap()).unwrap())
        .collect()
}

fn syntax(input: &Path, output: &Path) {
    let set: SyntaxSet = dumps::from_uncompressed_dump_file(input).unwrap();
    let mut value = serde_json::to_value(&set).unwrap();
    for reference in value["syntaxes"].as_array_mut().unwrap() {
        let legacy = bytes(&reference["serialized_lazy_contexts"]);
        let contexts: LazyContexts = dumps::from_reader(&legacy[..]).unwrap();
        reference["serialized_lazy_contexts"] =
            serde_json::to_value(pack(&contexts, true)).unwrap();
    }
    let converted: SyntaxSet = serde_json::from_value(value).unwrap();
    fs::write(output, pack(&converted, false)).unwrap();
    println!("{}: {} syntaxes", output.display(), set.syntaxes().len());
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    let registry = PathBuf::from(&args[1]);
    let root = PathBuf::from(&args[2]);
    let trusted: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../vendor/asset-sources.json")).unwrap();
    for (path, expected) in trusted {
        let actual = hex::encode(Sha256::digest(fs::read(registry.join(&path)).unwrap()));
        assert_eq!(actual, expected, "Untrusted historical asset: {path}");
    }
    let legacy = registry.join("syntect-5.3.0/assets");
    let target = root.join("vendor/syntect-5.3.0/assets");
    for name in ["default_newlines.packdump", "default_nonewlines.packdump"] {
        syntax(&legacy.join(name), &target.join(name));
    }
    let themes: ThemeSet = dumps::from_dump_file(legacy.join("default.themedump")).unwrap();
    fs::write(target.join("default.themedump"), pack(&themes, true)).unwrap();
    let metadata: syntect::parsing::Metadata =
        dumps::from_dump_file(legacy.join("default_metadata.packdump")).unwrap();
    fs::write(
        target.join("default_metadata.packdump"),
        pack(&metadata, true),
    )
    .unwrap();

    let legacy = registry.join("two-face-0.5.2+bat-0.26.1/generated");
    let target = root.join("vendor/two-face-0.5.2+bat-0.26.1/generated");
    for name in [
        "syntaxes-fancy-newlines.bin",
        "syntaxes-fancy-no-newlines.bin",
        "syntaxes-onig-newlines.bin",
        "syntaxes-onig-no-newlines.bin",
    ] {
        syntax(&legacy.join(name), &target.join(name));
    }
    let themes: two_face::theme::LazyThemeSet =
        dumps::from_uncompressed_dump_file(legacy.join("themes.bin")).unwrap();
    let mut value = serde_json::to_value(themes).unwrap();
    for theme in value["themes"].as_object_mut().unwrap().values_mut() {
        let legacy = bytes(&theme["serialized"]);
        let decoded: Theme = dumps::from_reader(&legacy[..]).unwrap();
        theme["serialized"] = serde_json::to_value(pack(&decoded, true)).unwrap();
    }
    let themes: two_face::theme::LazyThemeSet = serde_json::from_value(value).unwrap();
    fs::write(target.join("themes.bin"), pack(&themes, false)).unwrap();
    let acknowledgements: two_face::acknowledgement::Acknowledgements =
        dumps::from_dump_file(legacy.join("acknowledgements_full.bin")).unwrap();
    println!("Original license Markdown SHA256: {}", hex::encode(Sha256::digest(acknowledgements.to_md().as_bytes())));
    fs::write(
        target.join("acknowledgements_full.bin"),
        pack(&acknowledgements, true),
    )
    .unwrap();
}
