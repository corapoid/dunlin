use crate::model::{KnowledgeEdge, KnowledgeKind, KnowledgeObject};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedLink {
    pub relation: String,
    pub target: String,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedDocument {
    pub has_frontmatter: bool,
    pub frontmatter_raw: String,
    pub frontmatter: BTreeMap<String, Value>,
    pub links: Vec<ParsedLink>,
    pub body: String,
}

/// Split YAML-like frontmatter without a YAML dependency. Scalars, inline
/// arrays, indented string lists and the `links:` list are supported. Unknown
/// shapes stay as raw strings rather than being guessed.
pub fn parse_frontmatter(source: &str) -> ParsedDocument {
    let mut document = ParsedDocument {
        body: source.to_string(),
        ..ParsedDocument::default()
    };
    let Some((open_end, close_start, close_end)) = frontmatter_bounds(source) else {
        return document;
    };
    document.has_frontmatter = true;
    document.frontmatter_raw = source[open_end..close_start].to_string();
    document.body = source[close_end..].to_string();
    let lines: Vec<&str> = document.frontmatter_raw.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // A stray indented line outside a known block is preserved as prose.
            index += 1;
            continue;
        }
        let Some((key, rest)) = trimmed.split_once(':') else {
            index += 1;
            continue;
        };
        let key = key.trim();
        let rest = rest.trim();
        if key.is_empty() {
            index += 1;
            continue;
        }
        if key == "links" && rest.is_empty() {
            index += 1;
            parse_links(&lines, &mut index, &mut document.links);
            continue;
        }
        if rest.is_empty() {
            index += 1;
            let values = parse_indented_list(&lines, &mut index);
            if !values.is_empty() {
                document
                    .frontmatter
                    .insert(key.to_string(), Value::Array(values));
            }
            continue;
        }
        document
            .frontmatter
            .insert(key.to_string(), parse_scalar(rest));
        index += 1;
    }
    document
}

fn frontmatter_bounds(source: &str) -> Option<(usize, usize, usize)> {
    let first_end = source.find('\n')?;
    if source[..first_end].trim_end_matches('\r') != "---" {
        return None;
    }
    let open_end = first_end + 1;
    let mut offset = open_end;
    for line in source[open_end..].split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            return Some((open_end, offset, offset + line.len()));
        }
        offset += line.len();
    }
    // A file that starts with frontmatter but is not closed is not treated as
    // frontmatter; the whole file remains visible as prose.
    None
}

fn parse_links(lines: &[&str], index: &mut usize, output: &mut Vec<ParsedLink>) {
    let mut current: Option<ParsedLink> = None;
    while *index < lines.len() {
        let line = lines[*index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            *index += 1;
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix('-') {
            if let Some(link) = current.take()
                && !link.relation.is_empty()
                && !link.target.is_empty()
            {
                output.push(link);
            }
            current = Some(ParsedLink::default());
            let rest = rest.trim();
            if let Some((key, value)) = rest.split_once(':') {
                assign_link(
                    current.as_mut().expect("current link"),
                    key.trim(),
                    value.trim(),
                );
            }
            *index += 1;
            continue;
        }
        if let Some((key, value)) = trimmed.split_once(':')
            && let Some(link) = current.as_mut()
        {
            assign_link(link, key.trim(), value.trim());
        }
        *index += 1;
    }
    if let Some(link) = current
        && !link.relation.is_empty()
        && !link.target.is_empty()
    {
        output.push(link);
    }
}

fn assign_link(link: &mut ParsedLink, key: &str, value: &str) {
    let value = value.trim_matches(|c| c == '"' || c == '\'');
    match key {
        "relation" => link.relation = value.to_string(),
        "target" => link.target = value.to_string(),
        _ => {}
    }
}

fn parse_indented_list(lines: &[&str], index: &mut usize) -> Vec<Value> {
    let mut values = Vec::new();
    while *index < lines.len() {
        let line = lines[*index];
        if line.trim().is_empty() {
            *index += 1;
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        let trimmed = line.trim();
        let Some(item) = trimmed.strip_prefix('-') else {
            break;
        };
        values.push(parse_scalar(item.trim()));
        *index += 1;
    }
    values
}

fn parse_scalar(value: &str) -> Value {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        return Value::String(value[1..value.len() - 1].to_string());
    }
    if value.starts_with('[') && value.ends_with(']') {
        let items = value[1..value.len() - 1]
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(parse_scalar)
            .collect();
        return Value::Array(items);
    }
    match value {
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        "null" | "Null" | "NULL" | "~" => Value::Null,
        _ => {
            if let Ok(number) = value.parse::<i64>() {
                Value::Number(number.into())
            } else if let Ok(number) = value.parse::<f64>()
                && number.is_finite()
            {
                serde_json::Number::from_f64(number)
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::String(value.to_string()))
            } else {
                Value::String(value.to_string())
            }
        }
    }
}

/// Build one object from Markdown. The caller owns validation against a
/// schema; parsing must never rewrite the source file.
pub fn parse_document(
    kind: &KnowledgeKind,
    project_id: &str,
    relative: &str,
    source: &str,
    plugin_id: Option<&str>,
) -> KnowledgeObject {
    let document = parse_frontmatter(source);
    let id = document
        .frontmatter
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| derived_id(kind, relative));
    let title = document
        .frontmatter
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|title| !title.trim().is_empty())
        .or_else(|| first_heading(&document.body))
        .unwrap_or_else(|| title_from_path(relative));
    let status = document
        .frontmatter
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|status| !status.trim().is_empty())
        .unwrap_or_else(|| kind.default_status.clone());
    let mut fields = BTreeMap::new();
    for (key, value) in &document.frontmatter {
        if matches!(
            key.as_str(),
            "dunlin" | "kind" | "id" | "title" | "status" | "links"
        ) {
            continue;
        }
        fields.insert(key.clone(), value.clone());
    }
    let revision = hex::encode(Sha256::digest(source.as_bytes()));
    KnowledgeObject {
        id,
        project_id: project_id.to_string(),
        kind: kind.id.clone(),
        title,
        status,
        path: Some(relative.to_string()),
        revision,
        updated_ms: now_ms(),
        fields,
        body: document.body,
        plugin_id: plugin_id.map(str::to_string),
        schema_version: crate::model::API_VERSION,
    }
}

pub fn edges_for(object: &KnowledgeObject, source: &str) -> Vec<KnowledgeEdge> {
    parse_frontmatter(source)
        .links
        .into_iter()
        .filter(|link| !link.relation.is_empty() && !link.target.is_empty())
        .map(|link| KnowledgeEdge {
            from_id: object.id.clone(),
            to_id: link.target,
            relation: link.relation,
            metadata: BTreeMap::new(),
        })
        .collect()
}

pub fn derived_id(kind: &KnowledgeKind, relative: &str) -> String {
    let prefix: String = match kind.id.as_str() {
        "intent" => "INT".into(),
        "concept" => "CON".into(),
        "decision" => "DEC".into(),
        "spec" => "SPEC".into(),
        "task" => "TASK".into(),
        other => {
            let upper: String = other
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(4)
                .collect::<String>()
                .to_ascii_uppercase();
            if upper.is_empty() {
                "OBJ".into()
            } else {
                upper
            }
        }
    };
    let digest = hex::encode(Sha256::digest(relative.as_bytes()));
    format!("{prefix}-{}", &digest[..8])
}

fn first_heading(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
    })
}

fn title_from_path(path: &str) -> String {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md");
    stem.replace(['-', '_'], " ")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FieldDef, FieldKind};

    fn concept_kind() -> KnowledgeKind {
        KnowledgeKind {
            id: "concept".into(),
            label: "Concept".into(),
            directory: "concepts".into(),
            statuses: vec!["proposed".into(), "accepted".into()],
            default_status: "proposed".into(),
            fields: vec![FieldDef {
                id: "problem".into(),
                label: "Problem".into(),
                kind: FieldKind::Markdown,
                required: true,
                values: Vec::new(),
            }],
            relations: Vec::new(),
        }
    }

    #[test]
    fn frontmatter_scalars_lists_and_links_are_read() {
        let source = "---\ndunlin: 1\nkind: concept\nid: CON-001\ntitle: Workspace\nstatus: accepted\nowners: [alice, bob]\nlinks:\n  - relation: realizes\n    target: INT-001\n  - relation: constrains\n    target: CON-002\n---\n\n# Workspace\n\nBody\n";
        let parsed = parse_frontmatter(source);
        assert!(parsed.has_frontmatter);
        assert_eq!(
            parsed.frontmatter.get("id"),
            Some(&Value::String("CON-001".into()))
        );
        assert_eq!(
            parsed.frontmatter.get("owners"),
            Some(&Value::Array(vec![
                Value::String("alice".into()),
                Value::String("bob".into())
            ]))
        );
        assert_eq!(parsed.links.len(), 2);
        assert_eq!(parsed.links[0].relation, "realizes");
        assert_eq!(parsed.links[1].target, "CON-002");
        assert!(parsed.body.contains("# Workspace"));
    }

    #[test]
    fn document_uses_stable_frontmatter_id_and_derives_legacy_ids() {
        let kind = concept_kind();
        let source = "---\nid: CON-001\ntitle: Workspace\nstatus: accepted\nproblem: Agents lack shared vocabulary.\n---\n\nBody\n";
        let object = parse_document(
            &kind,
            "project",
            "concepts/workspace.md",
            source,
            Some("governance"),
        );
        assert_eq!(object.id, "CON-001");
        assert_eq!(object.title, "Workspace");
        assert_eq!(object.status, "accepted");
        assert_eq!(
            object.fields.get("problem").unwrap(),
            "Agents lack shared vocabulary."
        );
        assert_eq!(object.plugin_id.as_deref(), Some("governance"));

        let legacy = parse_document(
            &kind,
            "project",
            "concepts/workspace.md",
            "# Legacy\n",
            None,
        );
        assert!(legacy.id.starts_with("CON-"));
        assert_eq!(legacy.status, "proposed");
        assert_eq!(legacy.title, "Legacy");
    }

    #[test]
    fn horizontal_rules_do_not_start_frontmatter() {
        let parsed = parse_frontmatter("# Heading\n\n---\ntitle: not frontmatter\n---\n");
        assert!(!parsed.has_frontmatter);
        assert!(parsed.body.contains("title: not frontmatter"));
    }

    #[test]
    fn malformed_or_unclosed_frontmatter_stays_as_body() {
        let parsed = parse_frontmatter("---\ntitle: Broken\n");
        assert!(!parsed.has_frontmatter);
        assert!(parsed.body.contains("title: Broken"));
    }
}
