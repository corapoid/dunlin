//! Exercise the actual editor backend, not just extension recognition.
use dunlin_ui::component::highlighter::highlight_capture_names;
use dunlin_workspace::{preview, syntax};

fn captures(path: &str, source: &str) -> Vec<(String, String)> {
    assert_eq!(preview::kind(path), preview::PreviewKind::Text, "{path}");
    let language = syntax::language(path).expect("recognized file language");
    let runs = highlight_capture_names(language, source);
    assert!(!runs.is_empty(), "no highlighting for {path}: {language}");
    let mut end = 0;
    for (range, _) in &runs {
        assert!(
            range.start >= end && range.start < range.end,
            "{path}: {runs:?}"
        );
        assert!(
            source.get(range.clone()).is_some(),
            "invalid UTF-8 range in {path}"
        );
        end = range.end;
    }
    runs.into_iter()
        .map(|(range, name)| (name.to_string(), source[range].to_owned()))
        .collect()
}

fn assert_capture(runs: &[(String, String)], name: &str, token: &str) {
    assert!(
        runs.iter()
            .any(|(n, text)| n == name && text.contains(token)),
        "missing {name} capture for {token:?}: {runs:?}"
    );
}

#[test]
fn programming_languages_produce_real_captures() {
    for (path, source, style, token) in [
        (
            "main.zig",
            "const count: u32 = 42; // zażółć 🦀\n",
            "number",
            "42",
        ),
        (
            "build.zig.zon",
            ".{ .name = \"demo\", .version = \"0.1.0\" }\n",
            "string",
            "demo",
        ),
        (
            "main.dart",
            "void main() { print(\"hello\"); }\n",
            "string",
            "hello",
        ),
        (
            "App.cs",
            "public class App { int count = 42; }\n",
            "number",
            "42",
        ),
        ("script.csx", "var text = \"hello\";\n", "string", "hello"),
        (
            "App.jsx",
            "const App = () => <div title=\"hello\">{42}</div>;\n",
            "tag",
            "div",
        ),
        (
            "Fragment.jsx",
            "const App = () => <>hello <Button {...props} /></>;\n",
            "type",
            "Button",
        ),
        (
            "module.psm1",
            "function Get-Value { return 42 }\n",
            "function",
            "Get-Value",
        ),
        (
            "module.psd1",
            "@{ ModuleVersion = '1.0.0' }\n",
            "string",
            "1.0.0",
        ),
    ] {
        assert_capture(&captures(path, source), style, token);
    }
}

#[test]
fn powershell_highlights_interpolation_comments_types_and_here_strings() {
    let source = "# zażółć 🦀\n<# block\ncomment #>\n[int]$count = 42\nif ($count -gt 0) { Write-Host \"hello $count\" }\n$text = @'\nliteral $count\n'@\nWrite-Host 'done'\n";
    let runs = captures("script.ps1", source);
    for (name, token) in [
        ("comment", "zażółć 🦀"),
        ("comment", "comment #>"),
        ("type", "int"),
        ("variable", "count"),
        ("number", "42"),
        ("keyword", "if"),
        ("function", "Write-Host"),
        ("string", "hello"),
        ("string", "literal $count"),
        ("string", "done"),
    ] {
        assert_capture(&runs, name, token);
    }
}

#[test]
fn jsx_fragments_keep_components_and_expressions_out_of_regex_scopes() {
    let runs = captures(
        "Fragment.jsx",
        "const App = () => <>zażółć 🦀 <Button {...props} />{42}</>;\n",
    );
    for (name, token) in [
        ("keyword", "const"),
        ("type", "Button"),
        ("variable", "props"),
        ("number", "42"),
    ] {
        assert_capture(&runs, name, token);
    }
    assert!(!runs.iter().any(|(name, _)| name == "string.regex"));
}

#[test]
fn infrastructure_and_tool_files_produce_real_captures() {
    for (path, source, style, token) in [
        (
            "main.tf",
            "resource \"aws_instance\" \"web\" { count = 42 }\n",
            "string",
            "aws_instance",
        ),
        (
            "terraform.tfvars",
            "region = \"eu-central-1\"\n",
            "string",
            "eu-central-1",
        ),
        (
            ".terraform.lock.hcl",
            "provider \"registry.opentofu.org/hashicorp/aws\" { version = \"5.0.0\" }\n",
            "string",
            "5.0.0",
        ),
        (
            "main.tf.json",
            "{\"resource\": {\"count\": 42}}\n",
            "number",
            "42",
        ),
        (
            "schema.graphql",
            "type Query { user(id: ID!): String }\n",
            "keyword",
            "type",
        ),
        (
            "message.proto",
            "syntax = \"proto3\";\nmessage User { string name = 1; }\n",
            "string",
            "proto3",
        ),
        (
            "flake.nix",
            "{ description = \"demo\"; }\n",
            "string",
            "demo",
        ),
        (
            "CMakeLists.txt",
            "cmake_minimum_required(VERSION 3.20)\nproject(\"demo\")\n",
            "string",
            "demo",
        ),
        (
            "Dockerfile.dev",
            "FROM alpine:3.20\nRUN echo \"hello\"\n",
            "keyword",
            "FROM",
        ),
        ("nginx.conf", "server { listen 8080; }\n", "string", "8080"),
        (
            "ansible.cfg",
            "[defaults]\ninventory = ./hosts\n",
            "string",
            "./hosts",
        ),
        (
            "hosts",
            "[web]\nserver ansible_host=192.0.2.1\n",
            "string",
            "192.0.2.1",
        ),
    ] {
        assert_capture(&captures(path, source), style, token);
    }
}

#[test]
fn ansible_templates_keep_yaml_and_highlight_jinja_inside_strings() {
    let source = "# playbook 🦀\n- hosts: all\n  vars:\n    message: \"hello {{ user | default('world') }}!\"\n    count: 42\n    script: |\n      {% if enabled %}\n      echo {{ user }}\n      {% endif %}\n    note: '{# template comment #}'\n";
    for path in ["playbook.yml", "templates/config.yaml.j2"] {
        let runs = captures(path, source);
        for (style, token) in [
            ("comment", "playbook 🦀"),
            ("string", "hello"),
            ("string", "world"),
            ("number", "42"),
            ("keyword", "if"),
            ("keyword", "endif"),
            ("comment", "template comment"),
        ] {
            assert_capture(&runs, style, token);
        }
        assert!(
            runs.iter()
                .any(|(name, text)| name == "string" && text.ends_with("!\"")),
            "YAML suffix lost: {runs:?}"
        );
    }
    let runs = captures(
        "template.j2",
        "{% if enabled %}{{ value | default('demo') }}{% endif %}\n",
    );
    assert_capture(&runs, "keyword", "if");
    assert_capture(&runs, "string", "demo");
    assert_capture(
        &captures("page.html.j2", "<div>{{ name }}</div>\n"),
        "tag",
        "div",
    );
}

#[test]
fn compose_ci_and_plain_yaml_retain_their_styles() {
    for path in ["compose.yaml", ".github/workflows/ci.yml", ".gitlab-ci.yml"] {
        let runs = captures(path, "# comment\nname: \"demo\"\ncount: 42\n");
        assert_capture(&runs, "comment", "comment");
        assert_capture(&runs, "string", "demo");
        assert_capture(&runs, "number", "42");
    }
    for language in ["plaintext", "no-such-language"] {
        assert!(highlight_capture_names(language, "some text").is_empty());
    }
}

#[test]
fn large_yaml_templates_keep_ordered_unicode_safe_captures() {
    let source = (0..2000)
        .map(|index| format!("entry_{index}: \"zażółć 🦀 {{{{ host }}}}!\"\n"))
        .collect::<String>();
    assert!(source.len() > 64 * 1024);
    let runs = captures("large.yaml.j2", &source);
    assert_eq!(
        runs.iter()
            .filter(|(name, text)| name == "variable" && text == "host")
            .count(),
        2000
    );
    assert_eq!(
        runs.iter()
            .filter(|(name, text)| name == "string" && text.ends_with("!\""))
            .count(),
        2000
    );
}

#[test]
fn existing_editor_languages_still_resolve_after_the_backend_change() {
    for (path, source) in [
        ("main.rs", "fn main() { let n = 42; }\n"),
        ("Cargo.toml", "[package]\nname = \"demo\"\n"),
        ("data.jsonc", "{\"name\": \"demo\"}\n"),
        ("README.md", "# Hello\n**world**\n"),
        ("main.js", "const text = \"demo\";\n"),
        ("main.ts", "const count: number = 42;\n"),
        ("App.tsx", "const App = () => <div>hello</div>;\n"),
        ("main.py", "print(\"demo\") # comment\n"),
        ("main.go", "package main\nfunc main() { println(42) }\n"),
        ("main.c", "int main() { return 42; }\n"),
        ("main.cpp", "int main() { return 42; }\n"),
        ("style.css", "div { color: red; }\n"),
        ("style.scss", "$color: red; div { color: $color; }\n"),
        ("style.sass", "$color: red\ndiv\n  color: $color\n"),
        ("index.html", "<div>hello</div>\n"),
        ("script.sh", "echo \"demo\"\n"),
        ("query.sql", "SELECT 42;\n"),
        ("main.swift", "let count = 42\n"),
        ("Main.java", "class Main { int count = 42; }\n"),
        ("main.kt", "val count = 42\n"),
        ("main.rb", "puts \"demo\"\n"),
        ("index.php", "<?php echo \"demo\"; ?>\n"),
        ("data.xml", "<root>hello</root>\n"),
        ("settings.ini", "[main]\nkey=value\n"),
        (
            "App.svelte",
            "<script>let count = 42;</script>\n<h1>Hello</h1>\n",
        ),
        ("App.vue", "<template><h1>Hello</h1></template>\n"),
        ("main.lua", "local count = 42\n"),
        ("main.ex", "defmodule Demo do\n  def count, do: 42\nend\n"),
        ("main.r", "count <- 42\n"),
        ("main.pl", "my $count = 42;\n"),
        (
            "change.patch",
            "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n",
        ),
        ("Makefile", "all:\n\techo hello\n"),
        ("script.bat", "@echo off\nREM comment\necho hello\n"),
        (
            "CMakeCache.txt",
            "# Cache entry\nCMAKE_BUILD_TYPE:STRING=Release\n",
        ),
    ] {
        captures(path, source);
    }
}
