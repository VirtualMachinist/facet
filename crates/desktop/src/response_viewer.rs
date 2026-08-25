//! Read-only response presentation: Pretty, Raw, Headers, Inspect, and Search.
//!
//! This module retains response text without altering it, searches the active
//! representation, and pretty-prints JSON off the UI thread. Syntax coloring is
//! applied by the gpui-base `Editor` highlighter.

use std::ops::Range;

use crate::response_inspector::{
    INSPECT_MAX_BYTES, InspectSelection, InspectionRange, ResponseInspection,
    first_inspection_selection, inspect_response_body, inspection_has_selection,
    inspection_selection_at_offset, inspection_value_ranges,
};
use probe_http::{HttpResponse, ResponseHeader};

/// JSON pretty-print larger than this runs on a background executor.
pub(crate) const SYNC_PRETTY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ResponseViewerTab {
    #[default]
    Pretty,
    Raw,
    Headers,
    Inspect,
}

impl ResponseViewerTab {
    pub(crate) const ALL: [Self; 4] = [Self::Pretty, Self::Raw, Self::Headers, Self::Inspect];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pretty => "Pretty",
            Self::Raw => "Raw",
            Self::Headers => "Headers",
            Self::Inspect => "Inspect",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SearchMatch {
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedDocument {
    pub generation: u64,
    pub raw_text: String,
    pub pretty_text: String,
    pub pretty_pending: bool,
    pub pretty_notice: Option<String>,
    pub binary: bool,
    pub truncated: bool,
    pub headers: Vec<ResponseHeader>,
    pub inspection: ResponseInspection,
    pub inspection_pending: bool,
    pub inspection_ranges: Vec<InspectionRange>,
    pub inspection_selection: Option<InspectSelection>,
}

#[derive(Debug, Default)]
pub(crate) struct ResponseViewerState {
    tab: ResponseViewerTab,
    search: String,
    active_match: usize,
    documents: std::collections::BTreeMap<probe_core::RequestKey, PreparedDocument>,
    next_generation: u64,
}

impl ResponseViewerState {
    pub(crate) fn tab(&self) -> ResponseViewerTab {
        self.tab
    }

    pub(crate) fn search(&self) -> &str {
        &self.search
    }

    pub(crate) fn active_match(&self) -> usize {
        self.active_match
    }

    pub(crate) fn document(&self, key: probe_core::RequestKey) -> Option<&PreparedDocument> {
        self.documents.get(&key)
    }

    pub(crate) fn inspection_selection(
        &self,
        key: probe_core::RequestKey,
    ) -> Option<InspectSelection> {
        let document = self.documents.get(&key)?;
        document
            .inspection_selection
            .filter(|selection| inspection_has_selection(&document.inspection, *selection))
            .or_else(|| first_inspection_selection(&document.inspection))
    }

    pub(crate) fn allocate_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        self.next_generation
    }

    pub(crate) fn insert(&mut self, key: probe_core::RequestKey, document: PreparedDocument) {
        self.documents.insert(key, document);
        self.active_match = 0;
    }

    pub(crate) fn remove(&mut self, key: probe_core::RequestKey) {
        self.documents.remove(&key);
        self.active_match = 0;
    }

    pub(crate) fn clear(&mut self) {
        self.documents.clear();
        self.search.clear();
        self.active_match = 0;
        self.tab = ResponseViewerTab::default();
    }

    pub(crate) fn remap_requests(
        &mut self,
        key_remaps: &std::collections::BTreeMap<probe_core::RequestKey, probe_core::RequestKey>,
    ) {
        self.documents = std::mem::take(&mut self.documents)
            .into_iter()
            .filter_map(|(key, document)| key_remaps.get(&key).map(|new| (*new, document)))
            .collect();
        self.active_match = 0;
    }

    pub(crate) fn set_tab(&mut self, tab: ResponseViewerTab) {
        if self.tab != tab {
            self.tab = tab;
            self.active_match = 0;
        }
    }

    pub(crate) fn set_search(&mut self, query: String) {
        if self.search != query {
            self.search = query;
            self.active_match = 0;
        }
    }

    pub(crate) fn apply_pretty(
        &mut self,
        key: probe_core::RequestKey,
        generation: u64,
        pretty: PrettyBody,
    ) {
        let Some(document) = self.documents.get_mut(&key) else {
            return;
        };
        if document.generation != generation || !document.pretty_pending {
            return;
        }
        document.pretty_text = pretty.text;
        document.pretty_notice = pretty.notice;
        document.pretty_pending = false;
        document.inspection_ranges =
            inspection_value_ranges(&document.pretty_text, &document.inspection);
        self.active_match = 0;
    }

    pub(crate) fn apply_inspection(
        &mut self,
        key: probe_core::RequestKey,
        generation: u64,
        inspection: ResponseInspection,
    ) {
        let Some(document) = self.documents.get_mut(&key) else {
            return;
        };
        if document.generation != generation || !document.inspection_pending {
            return;
        }
        document.inspection = inspection;
        document.inspection_pending = false;
        document.inspection_selection = first_inspection_selection(&document.inspection);
        document.inspection_ranges =
            inspection_value_ranges(&document.pretty_text, &document.inspection);
    }

    pub(crate) fn select_inspection_at_offset(
        &mut self,
        key: probe_core::RequestKey,
        offset: usize,
    ) -> Option<InspectSelection> {
        let document = self.documents.get_mut(&key)?;
        let selection = inspection_selection_at_offset(&document.inspection_ranges, offset)?;
        document.inspection_selection = Some(selection);
        self.tab = ResponseViewerTab::Inspect;
        Some(selection)
    }

    pub(crate) fn select_inspection(
        &mut self,
        key: probe_core::RequestKey,
        selection: InspectSelection,
    ) {
        if let Some(document) = self.documents.get_mut(&key) {
            document.inspection_selection = Some(selection);
        }
    }

    pub(crate) fn visible_text(&self, key: probe_core::RequestKey) -> &str {
        let Some(document) = self.documents.get(&key) else {
            return "";
        };
        match self.tab {
            ResponseViewerTab::Pretty => &document.pretty_text,
            ResponseViewerTab::Raw => &document.raw_text,
            ResponseViewerTab::Headers | ResponseViewerTab::Inspect => "",
        }
    }

    #[cfg(test)]
    pub(crate) fn visible_line_count(&self, key: probe_core::RequestKey) -> usize {
        let text = self.visible_text(key);
        if text.is_empty() {
            0
        } else {
            text.lines().count() + usize::from(text.ends_with('\n'))
        }
    }

    pub(crate) fn matches(&self, key: probe_core::RequestKey) -> Vec<SearchMatch> {
        let Some(document) = self.documents.get(&key) else {
            return Vec::new();
        };
        match self.tab {
            ResponseViewerTab::Headers => search_headers(&document.headers, &self.search),
            ResponseViewerTab::Pretty | ResponseViewerTab::Raw => {
                search_text(self.visible_text(key), &self.search)
            }
            ResponseViewerTab::Inspect => Vec::new(),
        }
    }

    pub(crate) fn step_match(
        &mut self,
        key: probe_core::RequestKey,
        delta: isize,
    ) -> Option<usize> {
        let count = self.matches(key).len();
        if count == 0 {
            self.active_match = 0;
            return None;
        }
        let next = (self.active_match as isize + delta).rem_euclid(count as isize) as usize;
        self.active_match = next;
        Some(next)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrettyBody {
    pub text: String,
    pub notice: Option<String>,
}

pub(crate) fn prepare_document(
    response: &HttpResponse,
    generation: u64,
) -> (PreparedDocument, bool, bool) {
    let truncated = !response.body_complete;
    if response.body.is_empty() {
        return (
            PreparedDocument {
                generation,
                raw_text: String::new(),
                pretty_text: String::new(),
                pretty_pending: false,
                pretty_notice: None,
                binary: false,
                truncated,
                headers: response.headers.clone(),
                inspection: ResponseInspection::default(),
                inspection_pending: false,
                inspection_ranges: Vec::new(),
                inspection_selection: None,
            },
            false,
            false,
        );
    }
    if std::str::from_utf8(&response.body).is_err() {
        return (
            PreparedDocument {
                generation,
                raw_text: String::new(),
                pretty_text: String::new(),
                pretty_pending: false,
                pretty_notice: Some(format!("Binary response body ({} bytes).", response.size)),
                binary: true,
                truncated,
                headers: response.headers.clone(),
                inspection: ResponseInspection::default(),
                inspection_pending: false,
                inspection_ranges: Vec::new(),
                inspection_selection: None,
            },
            false,
            false,
        );
    }

    let raw_text = String::from_utf8_lossy(&response.body).into_owned();
    let json_candidate = looks_like_json(response);
    let pretty_pending = json_candidate && response.body.len() > SYNC_PRETTY_BYTES;
    let inspection_pending = json_candidate && response.body.len() <= INSPECT_MAX_BYTES;
    let inspection = if !json_candidate || inspection_pending {
        ResponseInspection::default()
    } else {
        inspect_response_body(&response.body)
    };
    let (pretty_text, pretty_notice, pretty_pending) = if pretty_pending {
        (raw_text.clone(), Some("Formatting JSON…".to_owned()), true)
    } else if json_candidate {
        let pretty = pretty_json_body(&response.body);
        (pretty.text, pretty.notice, false)
    } else {
        (
            raw_text.clone(),
            Some("Pretty formatting is available for JSON responses.".to_owned()),
            false,
        )
    };
    let inspection_ranges = inspection_value_ranges(&pretty_text, &inspection);
    let inspection_selection = first_inspection_selection(&inspection);

    (
        PreparedDocument {
            generation,
            raw_text,
            pretty_text,
            pretty_pending,
            pretty_notice,
            binary: false,
            truncated,
            headers: response.headers.clone(),
            inspection,
            inspection_pending,
            inspection_ranges,
            inspection_selection,
        },
        pretty_pending,
        inspection_pending,
    )
}

pub(crate) fn pretty_json_body(body: &[u8]) -> PrettyBody {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(pretty) => PrettyBody {
                text: pretty,
                notice: None,
            },
            Err(_) => PrettyBody {
                text: String::from_utf8_lossy(body).into_owned(),
                notice: Some("Could not pretty-print this JSON response.".to_owned()),
            },
        },
        Err(_) => PrettyBody {
            text: String::from_utf8_lossy(body).into_owned(),
            notice: Some("Response is not valid JSON.".to_owned()),
        },
    }
}

pub(crate) fn looks_like_json(response: &HttpResponse) -> bool {
    if let Some(content_type) = content_type(response)
        && content_type.to_ascii_lowercase().contains("json")
    {
        return true;
    }
    let trimmed = trim_ascii_start(&response.body);
    matches!(trimmed.first(), Some(b'{' | b'['))
}

fn content_type(response: &HttpResponse) -> Option<&str> {
    response
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-type"))
        .map(|header| header.value.as_str())
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let index = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    &bytes[index..]
}

pub(crate) const HEADER_SEPARATOR: &str = ": ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JoinedHeaders {
    pub text: String,
    pub line_offsets: Vec<usize>,
    pub name_lens: Vec<usize>,
}

pub(crate) fn join_header_lines(headers: &[ResponseHeader]) -> JoinedHeaders {
    let mut text = String::new();
    let mut line_offsets = Vec::with_capacity(headers.len());
    let mut name_lens = Vec::with_capacity(headers.len());
    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        line_offsets.push(text.len());
        name_lens.push(header.name.len());
        text.push_str(&header.name);
        text.push_str(HEADER_SEPARATOR);
        text.push_str(&header.value);
    }
    JoinedHeaders {
        text,
        line_offsets,
        name_lens,
    }
}

pub(crate) fn search_text(text: &str, query: &str) -> Vec<SearchMatch> {
    find_ignore_case(text, query)
        .into_iter()
        .map(|range| SearchMatch { range })
        .collect()
}

pub(crate) fn search_headers(headers: &[ResponseHeader], query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    let mut line_start = 0;
    for header in headers {
        for range in find_ignore_case(&header.name, query) {
            matches.push(SearchMatch {
                range: line_start + range.start..line_start + range.end,
            });
        }
        let value_start = line_start + header.name.len() + HEADER_SEPARATOR.len();
        for range in find_ignore_case(&header.value, query) {
            matches.push(SearchMatch {
                range: value_start + range.start..value_start + range.end,
            });
        }
        line_start = value_start + header.value.len() + 1;
    }
    matches
}

fn find_ignore_case(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_chars: Vec<char> = needle.chars().collect();
    let haystack_chars: Vec<(usize, char)> = haystack.char_indices().collect();
    let mut matches = Vec::new();
    let mut index = 0;
    while index < haystack_chars.len() {
        if chars_eq_ignore_case(&haystack_chars, index, &needle_chars) {
            let start = haystack_chars[index].0;
            let end = haystack_chars
                .get(index + needle_chars.len())
                .map(|(next, _)| *next)
                .unwrap_or(haystack.len());
            matches.push(start..end);
            index += needle_chars.len();
        } else {
            index += 1;
        }
    }
    matches
}

fn chars_eq_ignore_case(haystack: &[(usize, char)], start: usize, needle: &[char]) -> bool {
    if start + needle.len() > haystack.len() {
        return false;
    }
    haystack[start..start + needle.len()]
        .iter()
        .zip(needle)
        .all(|((_, haystack_char), needle_char)| equal_ignore_case(*haystack_char, *needle_char))
}

fn equal_ignore_case(left: char, right: char) -> bool {
    if left.eq_ignore_ascii_case(&right) {
        return true;
    }
    let mut left_lower = left.to_lowercase();
    let mut right_lower = right.to_lowercase();
    loop {
        match (left_lower.next(), right_lower.next()) {
            (Some(left_ch), Some(right_ch)) if left_ch == right_ch => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use probe_http::{HttpResponse, ResponseHeader};

    use super::{
        PreparedDocument, ResponseViewerTab, join_header_lines, looks_like_json, prepare_document,
        pretty_json_body, search_headers, search_text,
    };

    fn response(body: &[u8], content_type: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            reason: "OK".to_owned(),
            url: String::new(),
            duration: Duration::ZERO,
            size: body.len(),
            headers: vec![ResponseHeader {
                name: "content-type".to_owned(),
                value: content_type.to_owned(),
            }],
            body: body.to_vec(),
            body_complete: true,
        }
    }

    #[test]
    fn pretty_json_indents_object_fields() {
        let pretty = pretty_json_body(br#"{"ok":true,"n":1}"#);
        assert!(pretty.notice.is_none());
        let text = pretty.text;
        assert!(text.contains('\n'));
        assert!(text.contains("\"ok\""));
    }

    #[test]
    fn long_lines_are_preserved_for_the_virtualized_editor() {
        let line = "x".repeat(1_000);
        let response = response(line.as_bytes(), "text/plain");
        let (document, pending, inspection_pending) = prepare_document(&response, 1);
        assert!(!pending);
        assert!(!inspection_pending);
        assert_eq!(document.raw_text, line);
    }

    #[test]
    fn search_is_case_insensitive_and_records_byte_ranges() {
        let text = "Alpha\nbeta ALPHA";
        let matches = search_text(text, "alpha");
        assert_eq!(matches.len(), 2);
        assert_eq!(&text[matches[0].range.clone()], "Alpha");
        assert_eq!(&text[matches[1].range.clone()], "ALPHA");
    }

    #[test]
    fn header_search_covers_names_and_values() {
        let headers = [
            ResponseHeader {
                name: "content-type".to_owned(),
                value: "application/json".to_owned(),
            },
            ResponseHeader {
                name: "x-request-id".to_owned(),
                value: "abc".to_owned(),
            },
        ];
        let matches = search_headers(&headers, "json");
        assert_eq!(matches.len(), 1);
        let joined = join_header_lines(&headers);
        assert_eq!(&joined.text[matches[0].range.clone()], "json");
    }

    #[test]
    fn join_header_lines_keeps_name_and_value_offsets() {
        let headers = [
            ResponseHeader {
                name: "content-type".to_owned(),
                value: "application/json".to_owned(),
            },
            ResponseHeader {
                name: "x-request-id".to_owned(),
                value: "abc".to_owned(),
            },
        ];
        let joined = join_header_lines(&headers);
        assert_eq!(
            joined.text,
            "content-type: application/json\nx-request-id: abc"
        );
        assert_eq!(
            &joined.text[joined.line_offsets[0]..joined.line_offsets[0] + joined.name_lens[0]],
            "content-type"
        );
        let value_start = joined.line_offsets[1] + joined.name_lens[1] + 2;
        assert_eq!(&joined.text[value_start..], "abc");
    }

    #[test]
    fn binary_and_json_sniffing_prepare_the_expected_document() {
        let json = response(br#"{"ok":true}"#, "application/json");
        assert!(looks_like_json(&json));
        let (document, pending, inspection_pending) = prepare_document(&json, 1);
        assert!(!pending);
        assert!(inspection_pending);
        assert!(!document.binary);
        assert!(document.pretty_notice.is_none());
        assert!(document.pretty_text.contains('\n'));

        let binary = response(&[0, 159, 146, 150], "application/octet-stream");
        let (document, pending, inspection_pending) = prepare_document(&binary, 2);
        assert!(!pending);
        assert!(!inspection_pending);
        assert!(document.binary);
        assert!(document.raw_text.is_empty());
    }

    #[test]
    fn invalid_json_keeps_raw_text_and_explains_pretty_failure() {
        let response = response(b"{not json", "application/json");
        let (PreparedDocument { pretty_notice, .. }, pending, inspection_pending) =
            prepare_document(&response, 1);
        assert!(!pending);
        assert!(inspection_pending);
        assert_eq!(
            pretty_notice.as_deref(),
            Some("Response is not valid JSON.")
        );
    }

    #[test]
    fn raw_text_does_not_insert_line_breaks() {
        let source = r#"{"value":"abcdefghij"}"#;
        let response = response(source.as_bytes(), "application/json");
        let (document, pending, inspection_pending) = prepare_document(&response, 1);
        assert!(!pending);
        assert!(inspection_pending);
        assert_eq!(document.raw_text, source);
    }

    #[test]
    fn viewer_tabs_are_stable() {
        assert_eq!(ResponseViewerTab::ALL.len(), 4);
        assert_eq!(ResponseViewerTab::Pretty.label(), "Pretty");
        assert_eq!(ResponseViewerTab::Inspect.label(), "Inspect");
    }
}
