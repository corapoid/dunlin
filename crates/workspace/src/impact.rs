//! Bounded Rust AST dependency evidence. No project code, Cargo build scripts,
//! macros, LSP servers or package installation are executed. Edges are lexical
//! resolutions, NOT compiler/type/cfg proofs; uncertainty remains explicit.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::{Component, Path},
    time::{Duration, Instant},
};
use syn::{
    parse::Parser,
    visit::{self, Visit},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub test: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub file: String,
    pub line: u32,
    pub spelling: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Raw {
    symbol: Symbol,
    module: Vec<String>,
    paths: Vec<(Vec<String>, u32)>,
    locals: HashSet<String>,
    self_type: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Unit {
    raw: Vec<Raw>,
    imports: BTreeMap<Vec<String>, BTreeMap<String, Vec<String>>>,
    children: Vec<(String, Vec<String>)>,
    warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cached {
    digest: String,
    unit: Unit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub root: String,
    pub head: String,
    pub fingerprint: String,
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<String>,
    pub reused: usize,
    pub parsed: usize,
    files: BTreeMap<String, String>,
    inventory: BTreeSet<String>,
    cache: BTreeMap<String, Cached>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Impact {
    pub symbol: Symbol,
    pub references: Vec<Edge>,
    pub dependent_symbols: Vec<Symbol>,
    pub candidate_tests: Vec<Symbol>,
}
fn read(root: &str, args: &[&str]) -> Result<String, String> {
    crate::git::operations::read(root, args)
}
fn names(root: &str) -> Result<BTreeSet<String>, String> {
    let output = read(
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let names = output
        .split_terminator('\0')
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if names.len() > 30_000 {
        return Err("Impact inventory exceeds 30,000 paths".into());
    }
    Ok(names)
}
fn safe(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c,Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
}
fn source(root: &str, file: &str) -> Result<String, String> {
    if !safe(file) {
        return Err("Unsafe impact path".into());
    }
    let image = crate::task_review::image(root, file)?.ok_or("Impact source disappeared")?;
    let text = image.text().ok_or("Impact source is not UTF-8 text")?;
    if text.len() > 512 * 1024 {
        return Err("Impact source exceeds 512 KiB".into());
    }
    Ok(text)
}
fn hash(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}
fn key(module: &[String], name: &str) -> String {
    format!("{}::{name}", module.join("::"))
}
fn path_segments(path: &syn::Path) -> Vec<String> {
    path.segments.iter().map(|s| s.ident.to_string()).collect()
}
#[derive(Default)]
struct References {
    paths: Vec<(Vec<String>, u32)>,
    locals: HashSet<String>,
    dynamic: usize,
}
impl<'a> Visit<'a> for References {
    fn visit_path(&mut self, path: &'a syn::Path) {
        if path.leading_colon.is_some() {
            self.dynamic += 1;
            return;
        }
        self.paths.push((
            path_segments(path),
            path.segments
                .first()
                .map_or(1, |s| s.ident.span().start().line as u32),
        ));
        visit::visit_path(self, path);
    }
    fn visit_pat_ident(&mut self, pat: &'a syn::PatIdent) {
        self.locals.insert(pat.ident.to_string());
        visit::visit_pat_ident(self, pat);
    }
    fn visit_type_param(&mut self, param: &'a syn::TypeParam) {
        self.locals.insert(param.ident.to_string());
        visit::visit_type_param(self, param);
    }
    fn visit_const_param(&mut self, param: &'a syn::ConstParam) {
        self.locals.insert(param.ident.to_string());
        visit::visit_const_param(self, param);
    }
    fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
        self.dynamic += 1;
        visit::visit_expr_method_call(self, call);
    }
    fn visit_item_use(&mut self, _: &'a syn::ItemUse) {
        self.dynamic += 1;
    }
    fn visit_macro(&mut self, value: &'a syn::Macro) {
        // Parse expressions inside common builtin macros; never expand or run
        // user macros. Method dispatch/unknown macro expansion remain unknown.
        let name = value
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if [
            "assert",
            "assert_eq",
            "assert_ne",
            "debug_assert",
            "debug_assert_eq",
            "debug_assert_ne",
            "vec",
            "format",
            "println",
            "eprintln",
        ]
        .contains(&name.as_str())
        {
            let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
            if let Ok(exprs) = parser.parse2(value.tokens.clone()) {
                for expr in exprs {
                    self.visit_expr(&expr);
                }
                return;
            }
        }
        self.dynamic += 1;
    }
}
fn imports(
    tree: &syn::UseTree,
    prefix: Vec<String>,
    map: &mut BTreeMap<String, Vec<String>>,
    warnings: &mut Vec<String>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            let mut path = prefix;
            path.push(p.ident.to_string());
            imports(&p.tree, path, map, warnings);
        }
        syn::UseTree::Name(n) => {
            let mut path = prefix;
            let alias = if n.ident == "self" {
                path.last().cloned().unwrap_or_default()
            } else {
                path.push(n.ident.to_string());
                n.ident.to_string()
            };
            if map.insert(alias.clone(), path).is_some() {
                map.insert(alias, Vec::new());
                warnings
                    .push("Duplicate import aliases: lexical resolution may be ambiguous".into());
            }
        }
        syn::UseTree::Rename(n) => {
            let mut path = prefix;
            if n.ident != "self" {
                path.push(n.ident.to_string());
            }
            if map.insert(n.rename.to_string(), path).is_some() {
                map.insert(n.rename.to_string(), Vec::new());
                warnings.push("Duplicate import alias omitted".into());
            }
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                imports(item, prefix.clone(), map, warnings);
            }
        }
        syn::UseTree::Glob(_) => {
            warnings.push("Glob import omitted; references through it are unresolved".into())
        }
    }
}
fn parse(file: &str, crate_key: &str, module: &[String], text: &str) -> Result<Unit, String> {
    let parsed =
        syn::parse_file(text).map_err(|e| format!("{file}: Rust syntax unavailable: {e}"))?;
    let mut unit = Unit::default();
    let base = Path::new(file).parent().unwrap_or(Path::new(""));
    let stem = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let directory = if module.len() == 1 || ["lib", "main", "mod"].contains(&stem) {
        base.to_owned()
    } else {
        base.join(stem)
    };
    let _ = crate_key;
    items(&parsed.items, module, &directory, file, &mut unit);
    Ok(unit)
}
fn items(items: &[syn::Item], module: &[String], directory: &Path, file: &str, unit: &mut Unit) {
    for item in items {
        if let syn::Item::Use(value) = item {
            if value.leading_colon.is_some() {
                unit.warnings
                    .push(format!("{file}: absolute external import omitted"));
                continue;
            }
            imports(
                &value.tree,
                Vec::new(),
                unit.imports.entry(module.to_vec()).or_default(),
                &mut unit.warnings,
            );
        }
    }
    for item in items {
        if let syn::Item::Mod(value) = item {
            let name = value.ident.to_string();
            let mut child = module.to_vec();
            child.push(name.clone());
            if let Some((_, content)) = &value.content {
                self::items(content, &child, &directory.join(&name), file, unit);
            } else if value.attrs.iter().any(|a| a.path().is_ident("path")) {
                unit.warnings.push(format!(
                    "{file}: #[path] module omitted rather than guessing its scope"
                ));
            } else {
                unit.children.push((
                    directory
                        .join(format!("{name}.rs"))
                        .to_string_lossy()
                        .into_owned(),
                    child,
                ));
            }
            continue;
        }
        let mut push = |name: String,
                        line: usize,
                        kind: &str,
                        test: bool,
                        refs: References,
                        self_type: Option<String>| {
            if refs.dynamic > 0 {
                unit.warnings.push(format!("{file}:{line}: {} dynamic method/macro/local-import sites are not type-resolved",refs.dynamic));
            }
            unit.raw.push(Raw {
                symbol: Symbol {
                    id: key(module, &name),
                    name,
                    kind: kind.into(),
                    file: file.into(),
                    line: line as u32,
                    test,
                },
                module: module.to_vec(),
                paths: refs.paths,
                locals: refs.locals,
                self_type,
            });
        };
        match item {
            syn::Item::Fn(f) => {
                let mut refs = References::default();
                refs.visit_signature(&f.sig);
                refs.visit_block(&f.block);
                push(
                    f.sig.ident.to_string(),
                    f.sig.ident.span().start().line,
                    "function",
                    f.attrs
                        .iter()
                        .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test")),
                    refs,
                    None,
                );
            }
            syn::Item::Struct(s) => {
                let mut refs = References::default();
                refs.visit_fields(&s.fields);
                push(
                    s.ident.to_string(),
                    s.ident.span().start().line,
                    "struct",
                    false,
                    refs,
                    None,
                );
            }
            syn::Item::Enum(e) => {
                let mut refs = References::default();
                for v in &e.variants {
                    refs.visit_fields(&v.fields);
                }
                push(
                    e.ident.to_string(),
                    e.ident.span().start().line,
                    "enum",
                    false,
                    refs,
                    None,
                );
            }
            syn::Item::Type(t) => {
                let mut refs = References::default();
                refs.visit_type(&t.ty);
                push(
                    t.ident.to_string(),
                    t.ident.span().start().line,
                    "type",
                    false,
                    refs,
                    None,
                );
            }
            syn::Item::Const(c) => {
                let mut refs = References::default();
                refs.visit_expr(&c.expr);
                refs.visit_type(&c.ty);
                push(
                    c.ident.to_string(),
                    c.ident.span().start().line,
                    "const",
                    false,
                    refs,
                    None,
                );
            }
            syn::Item::Impl(i) => {
                if i.trait_.is_some() {
                    unit.warnings.push(format!(
                        "{file}: trait implementation dispatch is not resolved"
                    ));
                    continue;
                }
                let syn::Type::Path(ty) = i.self_ty.as_ref() else {
                    continue;
                };
                let Some(ty) = ty.path.segments.last().map(|s| s.ident.to_string()) else {
                    continue;
                };
                for method in &i.items {
                    if let syn::ImplItem::Fn(f) = method {
                        let mut refs = References::default();
                        refs.visit_signature(&f.sig);
                        refs.visit_block(&f.block);
                        push(
                            format!("{ty}::{}", f.sig.ident),
                            f.sig.ident.span().start().line,
                            "associated function",
                            false,
                            refs,
                            Some(ty.clone()),
                        );
                    }
                }
            }
            syn::Item::Macro(_) => unit
                .warnings
                .push(format!("{file}: item macro expansion is not indexed")),
            _ => {}
        }
    }
}
fn normalize(module: &[String], path: &[String], import: bool) -> Option<Vec<String>> {
    if path.is_empty() {
        return None;
    }
    let mut base = module.to_vec();
    let mut rest = path;
    match path[0].as_str() {
        "crate" => {
            base.truncate(1);
            rest = &path[1..];
        }
        "self" => rest = &path[1..],
        "super" => {
            while rest.first().is_some_and(|s| s == "super") {
                if base.len() <= 1 {
                    return None;
                }
                base.pop();
                rest = &rest[1..];
            }
        }
        _ => {
            if import {
                base.truncate(1);
            }
        }
    }
    base.extend_from_slice(rest);
    Some(base)
}
pub fn build(root: &str, previous: Option<&Index>) -> Result<Index, String> {
    let started = Instant::now();
    let root = crate::git::history::repository_root(root, None)?;
    let inventory = names(&root)?;
    let head = read(&root, &["rev-parse", "--verify", "HEAD"])?
        .trim()
        .to_string();
    let mut index=Index{root:root.clone(),head,fingerprint:String::new(),symbols:Vec::new(),edges:Vec::new(),warnings:vec!["Rust lexical AST evidence, NOT compiler/type/cfg/macro-complete references. Candidate tests are not proven coverage. Non-Rust and unresolved external crates are not indexed.".into()],reused:0,parsed:0,files:BTreeMap::new(),inventory:inventory.iter().filter(|f|f.ends_with(".rs")||Path::new(f).file_name().is_some_and(|n|n=="Cargo.toml")).cloned().collect(),cache:BTreeMap::new()};
    let mut queue = VecDeque::new();
    for manifest in inventory.iter().filter(|f| f.ends_with("Cargo.toml")) {
        if started.elapsed() > Duration::from_secs(10) {
            return Err("Impact manifest work budget exceeded".into());
        }
        let text = match source(&root, manifest) {
            Ok(v) => v,
            Err(e) => {
                index.warnings.push(e);
                continue;
            }
        };
        index.files.insert(manifest.clone(), hash(&text));
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            index.warnings.push(format!("Invalid manifest: {manifest}"));
            continue;
        };
        let Some(package) = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };
        let parent = Path::new(manifest).parent().unwrap_or(Path::new(""));
        let lib = value
            .get("lib")
            .and_then(|l| l.get("path"))
            .and_then(|p| p.as_str())
            .unwrap_or("src/lib.rs");
        let mut entries = vec![lib.to_string(), "src/main.rs".into()];
        if let Some(bins) = value.get("bin").and_then(|v| v.as_array()) {
            for bin in bins {
                if let Some(path) = bin.get("path").and_then(|p| p.as_str()) {
                    entries.push(path.into());
                }
            }
        }
        entries.sort();
        entries.dedup();
        for entry in entries {
            let file = parent.join(entry).to_string_lossy().into_owned();
            if safe(&file) && inventory.contains(&file) {
                let crate_key = format!("{}@{}", package.replace('-', "_"), file);
                queue.push_back((file, vec![crate_key]));
            }
        }
    }
    let mut visited = HashSet::new();
    let mut raw = Vec::new();
    let mut import_tables = BTreeMap::new();
    let mut total = 0;
    while let Some((file, module)) = queue.pop_front() {
        if !visited.insert((file.clone(), module.clone())) {
            continue;
        }
        if visited.len() > 500 || started.elapsed() > Duration::from_secs(10) {
            return Err("Impact index exceeds 500 module scopes / ten seconds; no complete graph is claimed".into());
        }
        let text = match source(&root, &file) {
            Ok(t) => t,
            Err(e) => {
                index.warnings.push(format!("{file}: {e}"));
                continue;
            }
        };
        total += text.len();
        if total > 20 * 1024 * 1024 {
            return Err("Impact source budget exceeds 20 MiB".into());
        }
        let digest = hash(&text);
        index.files.insert(file.clone(), digest.clone());
        let cache_key = format!("{}\0{}", module.join("::"), file);
        let cached = previous
            .filter(|p| p.root == root)
            .and_then(|p| p.cache.get(&cache_key))
            .filter(|c| c.digest == digest);
        let unit = if let Some(c) = cached {
            index.reused += 1;
            c.unit.clone()
        } else {
            index.parsed += 1;
            match parse(&file, &module[0], &module, &text) {
                Ok(unit) => unit,
                Err(e) => {
                    index.warnings.push(e);
                    continue;
                }
            }
        };
        for (child, scope) in &unit.children {
            let alternate = child
                .strip_suffix(".rs")
                .map(|s| format!("{s}/mod.rs"))
                .unwrap_or_default();
            if inventory.contains(child) && inventory.contains(&alternate) {
                index.warnings.push(format!(
                    "Ambiguous module files {child} and {alternate}; module omitted"
                ));
            } else if inventory.contains(child) {
                queue.push_back((child.clone(), scope.clone()));
            } else if inventory.contains(&alternate) {
                queue.push_back((alternate, scope.clone()));
            } else {
                index
                    .warnings
                    .push(format!("Module source unavailable: {child}"));
            }
        }
        raw.extend(unit.raw.clone());
        if raw.iter().map(|r| r.paths.len()).sum::<usize>() > 100_000 {
            return Err("Impact raw-reference budget reached".into());
        }
        import_tables.extend(unit.imports.clone());
        index.warnings.extend(unit.warnings.clone());
        index.cache.insert(cache_key, Cached { digest, unit });
        if raw.len() > 10_000 {
            return Err("Impact symbol limit reached".into());
        }
    }
    let mut definitions = HashMap::<String, usize>::new();
    for r in &raw {
        *definitions.entry(r.symbol.id.clone()).or_default() += 1;
    }
    let mut unresolved = 0;
    let mut edges = BTreeSet::new();
    for r in &raw {
        for (path, line) in &r.paths {
            if path.is_empty() || r.locals.contains(&path[0]) {
                continue;
            }
            let mut path = path.clone();
            if path[0] == "Self" {
                if let Some(ty) = &r.self_type {
                    path[0] = ty.clone();
                } else {
                    continue;
                }
            }
            let imports = import_tables.get(&r.module);
            let target = if let Some(alias) = imports.and_then(|m| m.get(&path[0])) {
                if alias.is_empty() {
                    unresolved += 1;
                    continue;
                }
                let mut alias = alias.clone();
                alias.extend_from_slice(&path[1..]);
                normalize(&r.module, &alias, true)
            } else {
                normalize(&r.module, &path, false)
            };
            let Some(target) = target else {
                unresolved += 1;
                continue;
            };
            let target = target.join("::");
            if definitions.get(&target) == Some(&1) && definitions.get(&r.symbol.id) == Some(&1) {
                if target != r.symbol.id {
                    edges.insert((
                        r.symbol.id.clone(),
                        target,
                        r.symbol.file.clone(),
                        *line,
                        path.join("::"),
                    ));
                }
            } else {
                unresolved += 1;
            }
            if edges.len() > 50_000 {
                return Err("Impact reference limit reached".into());
            }
        }
    }
    index.symbols = raw
        .into_iter()
        .filter(|r| definitions[&r.symbol.id] == 1)
        .map(|r| r.symbol)
        .collect();
    index.symbols.sort_by(|a, b| a.id.cmp(&b.id));
    index.edges = edges
        .into_iter()
        .map(|(from, to, file, line, spelling)| Edge {
            from,
            to,
            file,
            line,
            spelling,
        })
        .collect();
    index.warnings.push(format!("{unresolved} unresolved/ambiguous/external path sites omitted. Conditional compilation and receiver types are not evaluated."));
    index.warnings.sort();
    index.warnings.dedup();
    index.warnings.truncate(100);
    index.fingerprint = hash(&format!(
        "{}\n{}\n{:?}",
        index.root, index.head, index.files
    ));
    verify(&index)?;
    Ok(index)
}
pub fn verify(index: &Index) -> Result<(), String> {
    crate::git::history::repository_root(&index.root, Some(&index.root))?;
    let start = Instant::now();
    if read(&index.root, &["rev-parse", "--verify", "HEAD"])?.trim() != index.head {
        return Err("HEAD changed; refresh impact evidence".into());
    }
    let inventory = names(&index.root)?;
    let relevant = inventory
        .iter()
        .filter(|f| {
            f.ends_with(".rs") || Path::new(f).file_name().is_some_and(|n| n == "Cargo.toml")
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    if relevant != index.inventory {
        return Err("Rust source/manifest inventory changed; refresh impact evidence".into());
    }
    for (file, digest) in &index.files {
        if start.elapsed() > Duration::from_secs(10) {
            return Err("Impact verification work budget exceeded".into());
        }
        if !inventory.contains(file) || hash(&source(&index.root, file)?) != *digest {
            return Err("Source changed; refresh impact evidence before using it".into());
        }
    }
    Ok(())
}
pub fn affected(index: &Index, id: &str) -> Result<Impact, String> {
    let symbol = index
        .symbols
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or("Select an unambiguous indexed symbol")?;
    let mut reverse = HashMap::<&str, Vec<&str>>::new();
    for edge in &index.edges {
        reverse.entry(&edge.to).or_default().push(&edge.from);
    }
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([id]);
    while let Some(next) = queue.pop_front() {
        if !seen.insert(next) {
            continue;
        }
        if let Some(parents) = reverse.get(next) {
            queue.extend(parents.iter().copied());
        }
    }
    let references = index.edges.iter().filter(|e| e.to == id).cloned().collect();
    let dependent_symbols = index
        .symbols
        .iter()
        .filter(|s| s.id != id && seen.contains(s.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let candidate_tests = dependent_symbols
        .iter()
        .filter(|s| s.test)
        .cloned()
        .collect();
    Ok(Impact {
        symbol,
        references,
        dependent_symbols,
        candidate_tests,
    })
}
#[cfg(test)]
mod tests;
