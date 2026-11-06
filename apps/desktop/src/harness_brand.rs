//! Bundled harness marks and dispatch-time response identity. No network at runtime.
use dunlin_ui::*;
use std::sync::{Arc, OnceLock};

const MARKS: [&str; 5] = [
    include_str!("../../../assets/harnesses/claude.svg"),
    include_str!("../../../assets/harnesses/openai.svg"),
    include_str!("../../../assets/harnesses/opencode.svg"),
    include_str!("../../../assets/harnesses/copilot.svg"),
    include_str!("../../../assets/harnesses/gemini.svg"),
];

fn mark(harness: &str) -> Option<usize> {
    match harness {
        "Claude Code" => Some(0),
        "Codex" | "OpenAI Codex" => Some(1),
        "OpenCode" => Some(2),
        "GitHub Copilot" => Some(3),
        "Gemini CLI" => Some(4),
        _ => None,
    }
}

pub(crate) fn logo(harness: &str, side: f32, color: Hsla) -> AnyElement {
    if harness == "Hermes Agent" {
        static HERMES: OnceLock<[Arc<Image>; 2]> = OnceLock::new();
        let images = HERMES.get_or_init(|| {
            [
                include_bytes!("../../../assets/harnesses/hermes-light.png").as_slice(),
                include_bytes!("../../../assets/harnesses/hermes-dark.png").as_slice(),
            ]
            .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec())))
        });
        return img(images[usize::from(color.l > 0.5)].clone())
            .size(px(side))
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }
    static IMAGES: OnceLock<Vec<[Arc<Image>; 2]>> = OnceLock::new();
    let images = IMAGES.get_or_init(|| {
        MARKS
            .iter()
            .map(|svg| {
                ["#000000", "#ffffff"].map(|fill| {
                    Arc::new(Image::from_bytes(
                        ImageFormat::Svg,
                        svg.replace("<path ", &format!("<path fill=\"{fill}\" "))
                            .into_bytes(),
                    ))
                })
            })
            .collect()
    });
    if let Some(index) = mark(harness) {
        img(images[index][usize::from(color.l > 0.5)].clone())
            .size(px(side))
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    } else {
        // shortcut: unknown/custom harnesses use initials, add a licensed mark when available.
        let initials: String = harness
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect();
        div()
            .text_size(px(10.))
            .text_color(color)
            .child(initials)
            .into_any_element()
    }
}

pub(crate) fn signature(harness: &str, model: Option<&str>) -> String {
    let model = model
        .filter(|value| !value.is_empty())
        .unwrap_or("Model not recorded");
    let model = model.strip_prefix("Claude ").unwrap_or(model);
    let normalized = model
        .strip_prefix("anthropic/")
        .unwrap_or(model)
        .strip_prefix("claude-");
    let model = if let Some(model) = normalized {
        let (family, version) = model.split_once('-').unwrap_or((model, ""));
        let name = match family {
            "opus" => "Opus",
            "sonnet" => "Sonnet",
            "haiku" => "Haiku",
            _ => family,
        };
        format!(
            "{name} {}",
            version
                .split('-')
                .take_while(|part| part.len() < 8)
                .collect::<Vec<_>>()
                .join(".")
        )
        .trim()
        .to_string()
    } else {
        model.to_string()
    };
    format!("{harness}: {model}")
}

#[test]
fn signatures_and_marks_belong_to_the_harness_not_the_application() {
    assert_eq!(
        signature("Claude Code", Some("claude-opus-5-5")),
        "Claude Code: Opus 5.5"
    );
    assert_eq!(signature("OpenCode", Some("gpt-6")), "OpenCode: gpt-6");
    assert_eq!(signature("Codex", None), "Codex: Model not recorded");
    assert!(mark("Custom ACP").is_none());
    assert_ne!(mark("Claude Code"), mark("OpenCode"));
    for svg in MARKS {
        assert!(resvg::usvg::Tree::from_str(svg, &Default::default()).is_ok());
    }
    for bytes in [
        include_bytes!("../../../assets/harnesses/hermes-light.png").as_slice(),
        include_bytes!("../../../assets/harnesses/hermes-dark.png").as_slice(),
    ] {
        let image = image::load_from_memory(bytes).unwrap();
        assert_eq!((image.width(), image.height()), (32, 32));
    }
}
