//! gpui-base Editor highlighter adapter.
//!
//! The Editor paints syntax from [`InputHighlighter`] plus a
//! [`HighlightStyleResolver`]. gpui-base does not ship a parser or colors;
//! Probe supplies Syntect (same adapter as the gpui-base gallery) and maps
//! semantic names onto [`crate::theme::SyntaxColors`].

use std::{collections::HashMap, ops::Range, rc::Rc, sync::LazyLock};

use gpui::{Context, HighlightStyle, SharedString, Window};
use gpui_base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use syntect::{
    parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

use crate::theme::{SyntaxColors, Theme};

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
/// Match the in-memory response page so retained bodies can still be colored.
const MAX_HIGHLIGHT_BYTES: usize = 16 * 1024 * 1024;

fn should_highlight(text: &Rope) -> bool {
    within_highlight_budget(text.len())
}

fn within_highlight_budget(len: usize) -> bool {
    len <= MAX_HIGHLIGHT_BYTES
}

pub(crate) fn factory() -> InputHighlighterFactory {
    Rc::new(|language| {
        SyntectHighlighter::new(language).map(|highlighter| Box::new(highlighter) as Box<_>)
    })
}

pub(crate) struct ProbeHighlightStyles {
    colors: SyntaxColors,
}

impl ProbeHighlightStyles {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            colors: theme.colors.syntax,
        }
    }
}

impl HighlightStyleResolver for ProbeHighlightStyles {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        let color = match name {
            "property" => self.colors.property,
            "string" | "string.escape" => self.colors.string,
            "number" => self.colors.number,
            "boolean" | "keyword" | "constant" => self.colors.boolean,
            "null" | "comment" => self.colors.null,
            "punctuation" | "operator" => self.colors.punctuation,
            _ => return None,
        };
        Some(HighlightStyle {
            color: Some(color.into()),
            ..Default::default()
        })
    }
}

struct SyntectHighlighter {
    language: SharedString,
    highlights: Vec<(Range<usize>, &'static str)>,
    semantic_names: HashMap<Scope, Option<&'static str>>,
    json_meta: HashMap<Scope, JsonMeta>,
}

impl SyntectHighlighter {
    fn new(language: &str) -> Option<Self> {
        if language.is_empty() {
            return None;
        }
        find_syntax(language)?;
        Some(Self {
            language: language.to_owned().into(),
            highlights: Vec::new(),
            semantic_names: HashMap::new(),
            json_meta: HashMap::new(),
        })
    }

    fn reparse(&mut self, text: &str) {
        let syntax = find_syntax(self.language.as_ref())
            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
        let mut parser = ParseState::new(syntax);
        let mut scopes = ScopeStack::new();
        let mut offset = 0;
        self.highlights.clear();

        for line in LinesWithEndings::from(text) {
            if let Ok(operations) = parser.parse_line(line, &SYNTAX_SET) {
                let mut cursor = 0;
                for (index, operation) in operations {
                    self.push_highlight(offset + cursor..offset + index, &scopes);
                    let _ = scopes.apply(&operation);
                    cursor = index;
                }
                self.push_highlight(offset + cursor..offset + line.len(), &scopes);
            }
            offset += line.len();
        }
    }

    fn push_highlight(&mut self, range: Range<usize>, scopes: &ScopeStack) {
        if range.is_empty() {
            return;
        }

        let name = scopes.scopes.iter().rev().find_map(|scope| {
            *self
                .semantic_names
                .entry(*scope)
                .or_insert_with(|| semantic_name(*scope))
        });
        let name = match name {
            Some("string") | Some("string.escape") if self.json_object_key(scopes) => {
                Some("property")
            }
            other => other,
        };
        if let Some(name) = name {
            self.highlights.push((range, name));
        }
    }

    fn json_object_key(&mut self, scopes: &ScopeStack) -> bool {
        let mut in_dict = false;
        let mut in_value = false;
        for scope in &scopes.scopes {
            match self
                .json_meta
                .entry(*scope)
                .or_insert_with(|| json_meta(*scope))
            {
                JsonMeta::DictValue => in_value = true,
                JsonMeta::Dict => in_dict = true,
                JsonMeta::Other => {}
            }
        }
        in_dict && !in_value
    }
}

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    SYNTAX_SET
        .find_syntax_by_token(language)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(language))
}

impl InputHighlighter for SyntectHighlighter {
    fn language(&self) -> SharedString {
        self.language.clone()
    }

    fn update(
        &mut self,
        _edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        if !should_highlight(text) {
            self.highlights.clear();
            return;
        }
        self.reparse(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        resolve_styles(&self.highlights, range, resolver)
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

fn resolve_styles(
    highlights: &[(Range<usize>, &'static str)],
    range: &Range<usize>,
    resolver: &dyn HighlightStyleResolver,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let first = highlights.partition_point(|(highlight, _)| highlight.end <= range.start);
    let mut runs = Vec::new();
    let mut cursor = range.start;

    for (highlight_range, name) in &highlights[first..] {
        if highlight_range.start >= range.end {
            break;
        }

        let start = highlight_range.start.max(range.start);
        let end = highlight_range.end.min(range.end);
        if start >= end || end <= cursor {
            continue;
        }
        if cursor < start {
            runs.push((cursor..start, HighlightStyle::default()));
        }
        runs.push((start..end, resolver.style(name).unwrap_or_default()));
        cursor = end;
    }

    if cursor < range.end {
        runs.push((cursor..range.end, HighlightStyle::default()));
    }
    runs
}

fn semantic_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    if scope.starts_with("comment") {
        Some("comment")
    } else if scope.starts_with("constant.character.escape") {
        Some("string.escape")
    } else if scope.contains("property-name")
        || scope.starts_with("entity.other.attribute-name")
        || scope.starts_with("entity.name.tag")
    {
        Some("property")
    } else if scope.starts_with("string") {
        Some("string")
    } else if scope.starts_with("constant.numeric") {
        Some("number")
    } else if scope.starts_with("constant.language.null") {
        Some("null")
    } else if scope.starts_with("constant.language.boolean")
        || scope.starts_with("constant.language")
    {
        Some("boolean")
    } else if scope.starts_with("keyword.operator") {
        Some("operator")
    } else if scope.starts_with("keyword") || scope.starts_with("storage") {
        Some("keyword")
    } else if scope.starts_with("constant") {
        Some("constant")
    } else if scope.starts_with("punctuation") {
        Some("punctuation")
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum JsonMeta {
    Dict,
    DictValue,
    Other,
}

fn json_meta(scope: Scope) -> JsonMeta {
    let name = scope.build_string();
    if name.starts_with("meta.structure.dictionary.value") {
        JsonMeta::DictValue
    } else if name.starts_with("meta.structure.dictionary") {
        JsonMeta::Dict
    } else {
        JsonMeta::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight(language: &str, text: &str) -> Vec<(Range<usize>, &'static str)> {
        let mut highlighter = SyntectHighlighter::new(language).expect("known language");
        highlighter.reparse(text);
        highlighter.highlights
    }

    fn lexemes<'a>(
        source: &'a str,
        highlights: &[(Range<usize>, &'static str)],
        name: &str,
    ) -> Vec<&'a str> {
        highlights
            .iter()
            .filter(|(_, role)| *role == name)
            .map(|(range, _)| &source[range.clone()])
            .collect()
    }

    #[test]
    fn factory_ignores_empty_and_unknown_languages() {
        assert!(factory()("").is_none());
        assert!(factory()("not-a-real-language").is_none());
        assert!(factory()("json").is_some());
        assert!(factory()("xml").is_some());
    }

    #[test]
    fn highlighting_is_bounded_for_large_editor_buffers() {
        assert!(within_highlight_budget(MAX_HIGHLIGHT_BYTES));
        assert!(!within_highlight_budget(MAX_HIGHLIGHT_BYTES + 1));
        assert!(should_highlight(&Rope::from_str("{\"ok\":true}")));
    }

    #[test]
    #[ignore = "manual highlighter performance probe"]
    fn highlight_scan_timings() {
        let _ = &*SYNTAX_SET;
        highlight("json", "{}");

        for &(label, bytes) in &[
            ("64 KiB", 64 * 1024),
            ("1 MiB", 1024 * 1024),
            ("16 MiB", MAX_HIGHLIGHT_BYTES),
        ] {
            let pretty = json_document(bytes, true);
            let minified = json_document(bytes, false);
            let pretty_time = time_reparse("json", &pretty);
            let minified_time = time_reparse("json", &minified);
            eprintln!(
                "{label}: pretty {} bytes in {pretty_time:?}, minified {} bytes in {minified_time:?}",
                pretty.len(),
                minified.len()
            );
        }
    }

    fn json_document(target_bytes: usize, pretty: bool) -> String {
        let line = if pretty {
            "  \"key\": \"value\",\n"
        } else {
            "\"key\":\"value\","
        };
        let prefix = if pretty { "{\n" } else { "{" };
        let suffix = if pretty {
            "  \"end\": 1\n}"
        } else {
            "\"end\":1}"
        };
        let n = target_bytes.saturating_sub(prefix.len() + suffix.len()) / line.len();
        let mut document = String::with_capacity(target_bytes);
        document.push_str(prefix);
        for _ in 0..n {
            document.push_str(line);
        }
        document.push_str(suffix);
        document
    }

    fn time_reparse(language: &str, text: &str) -> std::time::Duration {
        let started = std::time::Instant::now();
        let highlights = highlight(language, text);
        let elapsed = started.elapsed();
        assert!(!highlights.is_empty(), "expected highlight spans");
        elapsed
    }

    #[test]
    fn json_keys_are_properties_and_values_keep_their_roles() {
        let source =
            "{\n  \"name\": \"Ada\",\n  \"ok\": true,\n  \"n\": 1,\n  \"missing\": null\n}";
        let highlights = highlight("json", source);
        let properties = lexemes(source, &highlights, "property");
        assert!(
            properties.iter().any(|lexeme| lexeme.contains("name")),
            "keys: {properties:?}"
        );
        assert!(
            properties.iter().any(|lexeme| lexeme.contains("ok")),
            "keys: {properties:?}"
        );
        let strings = lexemes(source, &highlights, "string");
        assert!(
            strings.iter().any(|lexeme| lexeme.contains("Ada")),
            "strings: {strings:?}"
        );
        assert!(
            !strings.iter().any(|lexeme| lexeme.contains("name")),
            "object keys should not use the string role: {strings:?}"
        );
        assert!(
            lexemes(source, &highlights, "boolean")
                .iter()
                .any(|lexeme| lexeme.contains("true"))
        );
        assert!(
            lexemes(source, &highlights, "number").contains(&"1"),
            "numbers: {:?}",
            lexemes(source, &highlights, "number")
        );
        let nulls = lexemes(source, &highlights, "null");
        let booleans = lexemes(source, &highlights, "boolean");
        assert!(
            nulls.iter().any(|lexeme| lexeme.contains("null"))
                || booleans.iter().any(|lexeme| lexeme.contains("null")),
            "null should be a constant; got nulls={nulls:?} booleans={booleans:?}"
        );
    }

    #[test]
    fn xml_tags_attributes_and_comments_map_to_probe_roles() {
        let source = r#"<?xml version="1.0"?><root id="1"><!-- n --><item/></root>"#;
        let highlights = highlight("xml", source);
        let properties = lexemes(source, &highlights, "property");
        assert!(
            properties
                .iter()
                .any(|lexeme| *lexeme == "root" || lexeme.contains("root")),
            "tags: {properties:?}"
        );
        assert!(
            properties
                .iter()
                .any(|lexeme| *lexeme == "id" || lexeme.contains("id")),
            "attributes: {properties:?}"
        );
        let strings = lexemes(source, &highlights, "string");
        assert!(
            strings.iter().any(|lexeme| lexeme.contains('1')),
            "strings: {strings:?}"
        );
        let comments = lexemes(source, &highlights, "comment");
        assert!(
            comments.iter().any(|lexeme| lexeme.contains("n")),
            "comments: {comments:?}"
        );
    }

    #[test]
    fn resolved_styles_cover_the_requested_range_without_gaps() {
        let theme = ProbeHighlightStyles::new(Theme::light());
        let runs = resolve_styles(&[(2..4, "keyword"), (6..8, "string")], &(0..10), &theme);
        let ranges: Vec<_> = runs.into_iter().map(|(range, _)| range).collect();
        assert_eq!(ranges, vec![0..2, 2..4, 4..6, 6..8, 8..10]);
    }

    #[test]
    fn unknown_highlight_roles_have_no_style() {
        let styles = ProbeHighlightStyles::new(Theme::light());
        assert!(styles.style("unknown").is_none());
    }
}
