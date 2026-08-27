//! Read-only response presentation: Pretty, Raw (Text/Base64), Headers, Inspect, and Search.
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

/// JSON pretty-print and Base64 encoding larger than this run on a background executor.
pub(crate) const SYNC_PRETTY_BYTES: usize = 64 * 1024;
pub(crate) const RESPONSE_PAGE_BYTES: usize = probe_http::MAX_IN_MEMORY_RESPONSE_BYTES;
const BASE64_LINE_LENGTH: usize = 76;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PageDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ResponseViewerTab {
    #[default]
    Pretty,
    Raw,
    Headers,
    Inspect,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ResponseBodySyntax {
    #[default]
    Plain,
    Json,
    Xml,
}

impl ResponseBodySyntax {
    pub(crate) const fn language(self) -> &'static str {
        match self {
            Self::Plain => "",
            Self::Json => "json",
            Self::Xml => "xml",
        }
    }
}

impl ResponseViewerTab {
    pub(crate) const ALL: [Self; 4] = [Self::Pretty, Self::Raw, Self::Headers, Self::Inspect];
    pub(crate) const TRUNCATED: [Self; 3] = [Self::Raw, Self::Headers, Self::Inspect];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pretty => "Pretty",
            Self::Raw => "Raw",
            Self::Headers => "Headers",
            Self::Inspect => "Inspect",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RawBodyView {
    #[default]
    Text,
    Base64,
}

impl RawBodyView {
    pub(crate) const ALL: [Self; 2] = [Self::Text, Self::Base64];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Base64 => "Base64",
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
    pub page_body: Vec<u8>,
    pub base64_text: String,
    pub base64_pending: bool,
    pub syntax: ResponseBodySyntax,
    pub binary: bool,
    pub file_backed: bool,
    pub truncated: bool,
    pub retention_notice: Option<String>,
    pub page_offset: usize,
    pub page_len: usize,
    pub total_size: usize,
    pub page_pending: bool,
    pub headers: Vec<ResponseHeader>,
    pub inspection: ResponseInspection,
    pub inspection_pending: bool,
    pub inspection_ranges: Vec<InspectionRange>,
    pub inspection_selection: Option<InspectSelection>,
}

impl PreparedDocument {
    pub(crate) fn can_load_previous_page(&self) -> bool {
        self.file_backed && self.page_offset > 0 && !self.page_pending
    }

    pub(crate) fn can_load_next_page(&self) -> bool {
        self.file_backed
            && !self.page_pending
            && self
                .page_offset
                .checked_add(self.page_len)
                .is_some_and(|end| end < self.total_size)
    }
}

#[derive(Debug, Default)]
pub(crate) struct ResponseViewerState {
    tab: ResponseViewerTab,
    raw_view: RawBodyView,
    documents: std::collections::BTreeMap<probe_core::RequestKey, PreparedDocument>,
    next_generation: u64,
}

impl ResponseViewerState {
    pub(crate) fn tab(&self) -> ResponseViewerTab {
        self.tab
    }

    pub(crate) fn raw_view(&self) -> RawBodyView {
        self.raw_view
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
    }

    pub(crate) fn ensure_available_tab(&mut self, key: probe_core::RequestKey) {
        let Some(document) = self.documents.get(&key) else {
            return;
        };
        if self.tab == ResponseViewerTab::Pretty && document.truncated {
            self.tab = ResponseViewerTab::Raw;
        }
        if document.binary && self.raw_view == RawBodyView::Text {
            self.raw_view = RawBodyView::Base64;
        }
    }

    pub(crate) fn remove(&mut self, key: probe_core::RequestKey) {
        self.documents.remove(&key);
    }

    pub(crate) fn clear(&mut self) {
        self.documents.clear();
        self.tab = ResponseViewerTab::default();
        self.raw_view = RawBodyView::default();
    }

    pub(crate) fn remap_requests(
        &mut self,
        key_remaps: &std::collections::BTreeMap<probe_core::RequestKey, probe_core::RequestKey>,
    ) {
        self.documents = std::mem::take(&mut self.documents)
            .into_iter()
            .filter_map(|(key, document)| key_remaps.get(&key).map(|new| (*new, document)))
            .collect();
    }

    pub(crate) fn set_tab(&mut self, tab: ResponseViewerTab) {
        self.tab = tab;
    }

    pub(crate) fn set_raw_view(&mut self, view: RawBodyView) {
        self.raw_view = view;
    }

    pub(crate) fn take_base64_job(
        &mut self,
        key: probe_core::RequestKey,
    ) -> Option<(u64, Vec<u8>)> {
        if self.tab != ResponseViewerTab::Raw || self.raw_view != RawBodyView::Base64 {
            return None;
        }
        let document = self.documents.get_mut(&key)?;
        if document.base64_pending || !document.base64_text.is_empty() {
            return None;
        }
        let bytes = page_bytes(document).to_vec();
        if bytes.is_empty() {
            return None;
        }
        if bytes.len() <= SYNC_PRETTY_BYTES {
            document.base64_text = encode_base64(&bytes);
            None
        } else {
            document.base64_pending = true;
            Some((document.generation, bytes))
        }
    }

    pub(crate) fn apply_base64(
        &mut self,
        key: probe_core::RequestKey,
        generation: u64,
        encoded: String,
    ) {
        let Some(document) = self.documents.get_mut(&key) else {
            return;
        };
        if document.generation != generation || !document.base64_pending {
            return;
        }
        document.base64_text = encoded;
        document.base64_pending = false;
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

    pub(crate) fn begin_page(
        &mut self,
        key: probe_core::RequestKey,
        direction: PageDirection,
    ) -> Option<(u64, usize)> {
        let document = self.documents.get_mut(&key)?;
        if !document.file_backed || document.page_pending {
            return None;
        }
        let offset = match direction {
            PageDirection::Previous if document.can_load_previous_page() => {
                document.page_offset.saturating_sub(RESPONSE_PAGE_BYTES)
            }
            PageDirection::Previous => return None,
            PageDirection::Next => document
                .can_load_next_page()
                .then_some(document.page_offset)
                .and_then(|offset| offset.checked_add(RESPONSE_PAGE_BYTES))
                .filter(|offset| *offset < document.total_size)?,
        };
        if offset == document.page_offset {
            return None;
        }
        document.page_pending = true;
        Some((document.generation, offset))
    }

    pub(crate) fn apply_page(
        &mut self,
        key: probe_core::RequestKey,
        generation: u64,
        offset: usize,
        body: Vec<u8>,
    ) {
        let Some(document) = self.documents.get_mut(&key) else {
            return;
        };
        if document.generation != generation || !document.page_pending {
            return;
        }
        document.page_offset = offset;
        document.page_len = body.len();
        if document.binary {
            document.page_body = body;
            document.raw_text.clear();
        } else {
            document.raw_text = String::from_utf8_lossy(&body).into_owned();
            document.page_body.clear();
        }
        document.base64_text.clear();
        document.base64_pending = false;
        document.pretty_text.clear();
        document.pretty_notice = None;
        document.inspection_ranges.clear();
        document.page_pending = false;
    }

    pub(crate) fn fail_page(
        &mut self,
        key: probe_core::RequestKey,
        generation: u64,
        message: String,
    ) {
        let Some(document) = self.documents.get_mut(&key) else {
            return;
        };
        if document.generation == generation && document.page_pending {
            document.page_pending = false;
            document.pretty_notice = Some(message);
        }
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

    pub(crate) fn inspection_range_for_selection(
        &self,
        key: probe_core::RequestKey,
        selection: InspectSelection,
    ) -> Option<Range<usize>> {
        let document = self.documents.get(&key)?;
        document
            .inspection_ranges
            .iter()
            .find(|range| range.selection == selection)
            .map(|range| range.range.clone())
    }

    pub(crate) fn reveal_inspection_in_pretty(
        &mut self,
        key: probe_core::RequestKey,
    ) -> Option<InspectSelection> {
        let selection = self.inspection_selection(key)?;
        self.inspection_range_for_selection(key, selection)?;
        if let Some(document) = self.documents.get_mut(&key) {
            document.inspection_selection = Some(selection);
        }
        self.tab = ResponseViewerTab::Pretty;
        Some(selection)
    }

    pub(crate) fn visible_text(&self, key: probe_core::RequestKey) -> &str {
        let Some(document) = self.documents.get(&key) else {
            return "";
        };
        match self.tab {
            ResponseViewerTab::Pretty => &document.pretty_text,
            ResponseViewerTab::Raw => match self.raw_view {
                RawBodyView::Text => &document.raw_text,
                RawBodyView::Base64 => &document.base64_text,
            },
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

    #[cfg(test)]
    fn show_raw_base64(&mut self, key: probe_core::RequestKey) {
        self.set_tab(ResponseViewerTab::Raw);
        self.set_raw_view(RawBodyView::Base64);
        if let Some((generation, bytes)) = self.take_base64_job(key) {
            self.apply_base64(key, generation, encode_base64(&bytes));
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrettyBody {
    pub text: String,
    pub notice: Option<String>,
}

fn document_from_response(response: &HttpResponse, generation: u64) -> PreparedDocument {
    PreparedDocument {
        generation,
        raw_text: String::new(),
        pretty_text: String::new(),
        pretty_pending: false,
        pretty_notice: None,
        page_body: Vec::new(),
        base64_text: String::new(),
        base64_pending: false,
        syntax: ResponseBodySyntax::Plain,
        binary: false,
        file_backed: response.body_file.is_some(),
        truncated: !response.body_complete,
        retention_notice: response.body_retention_error.clone(),
        page_offset: 0,
        page_len: response.body.len(),
        total_size: response.size,
        page_pending: false,
        headers: response.headers.clone(),
        inspection: ResponseInspection::default(),
        inspection_pending: false,
        inspection_ranges: Vec::new(),
        inspection_selection: None,
    }
}

pub(crate) fn prepare_document(
    response: &HttpResponse,
    generation: u64,
) -> (PreparedDocument, bool, bool) {
    if response.body.is_empty() {
        return (document_from_response(response, generation), false, false);
    }
    let file_backed = response.body_file.is_some();
    if body_is_binary(&response.body, file_backed) {
        let mut document = document_from_response(response, generation);
        document.pretty_notice = Some(format!("Binary response body ({} bytes).", response.size));
        document.page_body = response.body.clone();
        document.binary = true;
        return (document, false, false);
    }

    let raw_text = String::from_utf8_lossy(&response.body).into_owned();
    let syntax = response_body_syntax(response);
    let truncated = !response.body_complete;
    let json_candidate = syntax == ResponseBodySyntax::Json;
    let inspection_candidate = matches!(syntax, ResponseBodySyntax::Json | ResponseBodySyntax::Xml);
    let pretty_pending = !truncated && json_candidate && response.body.len() > SYNC_PRETTY_BYTES;
    let inspection_pending = (file_backed && inspection_candidate)
        || (!truncated && inspection_candidate && response.body.len() <= INSPECT_MAX_BYTES);
    let inspection = if truncated && !file_backed {
        ResponseInspection {
            skipped: Some(
                response
                    .body_retention_error
                    .clone()
                    .unwrap_or_else(|| "The complete response body was not retained.".to_owned()),
            ),
            ..ResponseInspection::default()
        }
    } else if !inspection_candidate || inspection_pending {
        ResponseInspection::default()
    } else {
        inspect_response_body(&response.body)
    };
    let (pretty_text, pretty_notice, pretty_pending) = if truncated {
        (String::new(), None, false)
    } else if pretty_pending {
        (raw_text.clone(), Some("Formatting JSON…".to_owned()), true)
    } else if json_candidate {
        let pretty = pretty_json_body(&response.body);
        (pretty.text, pretty.notice, false)
    } else if syntax == ResponseBodySyntax::Xml {
        (raw_text.clone(), None, false)
    } else {
        (
            raw_text.clone(),
            Some("Pretty formatting is available for JSON responses.".to_owned()),
            false,
        )
    };
    let inspection_ranges = inspection_value_ranges(&pretty_text, &inspection);
    let inspection_selection = first_inspection_selection(&inspection);

    let mut document = document_from_response(response, generation);
    document.raw_text = raw_text;
    document.pretty_text = pretty_text;
    document.pretty_pending = pretty_pending;
    document.pretty_notice = pretty_notice;
    document.syntax = syntax;
    document.inspection = inspection;
    document.inspection_pending = inspection_pending;
    document.inspection_ranges = inspection_ranges;
    document.inspection_selection = inspection_selection;
    (document, pretty_pending, inspection_pending)
}

fn body_is_binary(body: &[u8], file_backed: bool) -> bool {
    match std::str::from_utf8(body) {
        Ok(_) => false,
        // A bounded prefix can end partway through an otherwise valid UTF-8
        // scalar. Invalid bytes before the end still identify a binary body.
        Err(error) => !file_backed || error.error_len().is_some(),
    }
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

fn page_bytes(document: &PreparedDocument) -> &[u8] {
    if document.binary {
        &document.page_body
    } else {
        document.raw_text.as_bytes()
    }
}

pub(crate) fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let output_len = input.len().div_ceil(3).saturating_mul(4);
    let mut output = vec![0; output_len];
    let mut offset = 0;
    let mut index = 0;
    while index + 3 <= input.len() {
        let n = (u32::from(input[index]) << 16)
            | (u32::from(input[index + 1]) << 8)
            | u32::from(input[index + 2]);
        output[offset] = TABLE[((n >> 18) & 0x3F) as usize];
        output[offset + 1] = TABLE[((n >> 12) & 0x3F) as usize];
        output[offset + 2] = TABLE[((n >> 6) & 0x3F) as usize];
        output[offset + 3] = TABLE[(n & 0x3F) as usize];
        index += 3;
        offset += 4;
    }
    match input.len() - index {
        1 => {
            let n = u32::from(input[index]) << 16;
            output[offset] = TABLE[((n >> 18) & 0x3F) as usize];
            output[offset + 1] = TABLE[((n >> 12) & 0x3F) as usize];
            output[offset + 2] = b'=';
            output[offset + 3] = b'=';
        }
        2 => {
            let n = (u32::from(input[index]) << 16) | (u32::from(input[index + 1]) << 8);
            output[offset] = TABLE[((n >> 18) & 0x3F) as usize];
            output[offset + 1] = TABLE[((n >> 12) & 0x3F) as usize];
            output[offset + 2] = TABLE[((n >> 6) & 0x3F) as usize];
            output[offset + 3] = b'=';
        }
        _ => {}
    }
    wrap_base64(output)
}

fn wrap_base64(encoded: Vec<u8>) -> String {
    if encoded.len() <= BASE64_LINE_LENGTH {
        return String::from_utf8(encoded).expect("base64 alphabet is ASCII");
    }
    let extra_newlines = encoded.len().saturating_sub(1) / BASE64_LINE_LENGTH;
    let mut wrapped = String::with_capacity(encoded.len() + extra_newlines);
    for (index, chunk) in encoded.chunks(BASE64_LINE_LENGTH).enumerate() {
        if index > 0 {
            wrapped.push('\n');
        }
        wrapped.push_str(std::str::from_utf8(chunk).expect("base64 alphabet is ASCII"));
    }
    wrapped
}

pub(crate) fn response_body_syntax(response: &HttpResponse) -> ResponseBodySyntax {
    if let Some(content_type) = content_type(response)
        && content_type.to_ascii_lowercase().contains("json")
    {
        return ResponseBodySyntax::Json;
    }
    if let Some(content_type) = content_type(response)
        && content_type.to_ascii_lowercase().contains("xml")
    {
        return ResponseBodySyntax::Xml;
    }
    let trimmed = trim_ascii_start(&response.body);
    if matches!(trimmed.first(), Some(b'{' | b'[')) {
        ResponseBodySyntax::Json
    } else if trimmed.first() == Some(&b'<') {
        ResponseBodySyntax::Xml
    } else {
        ResponseBodySyntax::Plain
    }
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

#[cfg(test)]
pub(crate) fn search_text(text: &str, query: &str) -> Vec<SearchMatch> {
    find_ignore_case(text, query)
        .into_iter()
        .map(|range| SearchMatch { range })
        .collect()
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
fn chars_eq_ignore_case(haystack: &[(usize, char)], start: usize, needle: &[char]) -> bool {
    if start + needle.len() > haystack.len() {
        return false;
    }
    haystack[start..start + needle.len()]
        .iter()
        .zip(needle)
        .all(|((_, haystack_char), needle_char)| equal_ignore_case(*haystack_char, *needle_char))
}

#[cfg(test)]
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
        PageDirection, PreparedDocument, RESPONSE_PAGE_BYTES, RawBodyView, ResponseBodySyntax,
        ResponseViewerTab, body_is_binary, encode_base64, join_header_lines, prepare_document,
        pretty_json_body, response_body_syntax, search_headers, search_text,
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
            body_file: None,
            body_retention_error: None,
        }
    }

    fn request_key() -> probe_core::RequestKey {
        let workspace = probe_core::Workspace::from_collection(probe_core::Collection {
            items: vec![probe_core::CollectionItem::HttpRequest(
                probe_core::HttpRequest::default(),
            )],
            ..probe_core::Collection::default()
        });
        let probe_core::WorkspaceItemRef::Request(key) = workspace.root_items()[0] else {
            panic!("expected request key");
        };
        key
    }

    fn viewer_with(
        document: PreparedDocument,
    ) -> (probe_core::RequestKey, super::ResponseViewerState) {
        let key = request_key();
        let mut viewer = super::ResponseViewerState::default();
        viewer.insert(key, document);
        (key, viewer)
    }

    fn file_backed_document(body: &[u8], trailing: usize, content_type: &str) -> PreparedDocument {
        let mut large = response(body, content_type);
        large.size = body.len() + trailing;
        large.body_complete = false;
        let (mut document, _, _) = prepare_document(&large, 7);
        document.file_backed = true;
        document
    }

    const BINARY_BODY: &[u8] = &[0, 159, 146, 150];

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
        assert_eq!(response_body_syntax(&json), ResponseBodySyntax::Json);
        let (document, pending, inspection_pending) = prepare_document(&json, 1);
        assert!(!pending);
        assert!(inspection_pending);
        assert!(!document.binary);
        assert!(document.pretty_notice.is_none());
        assert!(document.pretty_text.contains('\n'));

        let binary = response(BINARY_BODY, "application/octet-stream");
        let (document, pending, inspection_pending) = prepare_document(&binary, 2);
        assert!(!pending);
        assert!(!inspection_pending);
        assert!(document.binary);
        assert!(document.raw_text.is_empty());
        assert_eq!(document.page_body, BINARY_BODY);
    }

    #[test]
    fn encode_base64_matches_rfc4648_and_wraps_at_76_columns() {
        let cases: &[(&[u8], &str)] = &[
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (BINARY_BODY, "AJ+Slg=="),
        ];
        for (input, expected) in cases {
            assert_eq!(encode_base64(input), *expected, "input={input:?}");
        }

        let wrapped = encode_base64(&[b'a'; 60]);
        let lines: Vec<&str> = wrapped.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 76);
        assert!(lines[1].len() < 76);
        assert!(!wrapped.ends_with('\n'));
    }

    #[test]
    fn raw_base64_view_encodes_the_response_body() {
        let json = br#"{"ok":true}"#;
        let (key, mut viewer) =
            viewer_with(prepare_document(&response(json, "application/json"), 1).0);
        viewer.show_raw_base64(key);
        assert_eq!(viewer.visible_text(key), encode_base64(json));

        viewer.insert(
            key,
            prepare_document(&response(BINARY_BODY, "application/octet-stream"), 2).0,
        );
        viewer.ensure_available_tab(key);
        assert_eq!(viewer.raw_view(), RawBodyView::Base64);
        viewer.show_raw_base64(key);
        assert_eq!(viewer.visible_text(key), encode_base64(BINARY_BODY));
    }

    #[test]
    fn paging_a_binary_body_replaces_bytes_and_invalidates_base64() {
        let first_page = vec![0xFF; RESPONSE_PAGE_BYTES];
        let (key, mut viewer) = viewer_with(file_backed_document(
            &first_page,
            4,
            "application/octet-stream",
        ));
        viewer.ensure_available_tab(key);
        assert_eq!(viewer.tab(), ResponseViewerTab::Raw);
        assert_eq!(viewer.raw_view(), RawBodyView::Base64);
        assert!(viewer.take_base64_job(key).is_some());
        assert!(viewer.document(key).unwrap().base64_pending);

        let (generation, offset) = viewer.begin_page(key, PageDirection::Next).unwrap();
        viewer.apply_page(key, generation, offset, vec![1, 2, 3, 4]);

        let document = viewer.document(key).unwrap();
        assert_eq!(document.page_offset, RESPONSE_PAGE_BYTES);
        assert_eq!(document.page_body, [1, 2, 3, 4]);
        assert!(document.base64_text.is_empty());
        assert!(!document.base64_pending);
        viewer.show_raw_base64(key);
        assert_eq!(viewer.visible_text(key), encode_base64(&[1, 2, 3, 4]));
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
    fn xml_responses_select_xml_highlighting_in_the_pretty_tab() {
        let source = r#"<?xml version="1.0"?><root id="1"><item/></root>"#;
        let xml = response(source.as_bytes(), "application/problem+xml; charset=utf-8");
        assert_eq!(response_body_syntax(&xml), ResponseBodySyntax::Xml);

        let (document, pending, inspection_pending) = prepare_document(&xml, 1);
        assert!(!pending);
        assert!(inspection_pending);
        assert_eq!(document.syntax.language(), "xml");
        assert_eq!(document.pretty_text, source);
        assert!(document.pretty_notice.is_none());
    }

    #[test]
    fn xml_is_sniffed_when_content_type_is_not_specific() {
        let xml = response(b" \n<root><item/></root>", "text/plain");
        assert_eq!(response_body_syntax(&xml), ResponseBodySyntax::Xml);
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
    fn file_backed_pages_replace_only_the_bounded_view() {
        let first_page = vec![b'x'; RESPONSE_PAGE_BYTES];
        let (key, mut viewer) = viewer_with(file_backed_document(&first_page, 4, "text/plain"));
        viewer.ensure_available_tab(key);
        assert_eq!(viewer.tab(), ResponseViewerTab::Raw);
        assert!(!viewer.document(key).unwrap().can_load_previous_page());
        assert!(viewer.document(key).unwrap().can_load_next_page());
        assert_eq!(
            ResponseViewerTab::TRUNCATED,
            [
                ResponseViewerTab::Raw,
                ResponseViewerTab::Headers,
                ResponseViewerTab::Inspect,
            ]
        );

        let (generation, offset) = viewer.begin_page(key, PageDirection::Next).unwrap();
        viewer.set_tab(ResponseViewerTab::Headers);
        viewer.apply_page(key, generation, offset, b"last".to_vec());

        let document = viewer.document(key).unwrap();
        assert_eq!(document.page_offset, RESPONSE_PAGE_BYTES);
        assert_eq!(document.raw_text, "last");
        assert!(document.pretty_text.is_empty());
        assert!(document.can_load_previous_page());
        assert!(!document.can_load_next_page());
        assert_eq!(viewer.tab(), ResponseViewerTab::Headers);
    }

    #[test]
    fn an_incomplete_utf8_scalar_at_a_file_preview_boundary_is_not_binary() {
        assert!(!body_is_binary(b"text\xE2\x82", true));
        assert!(body_is_binary(b"text\xE2\x82", false));
        assert!(body_is_binary(b"text\xFF", true));
    }

    #[test]
    fn an_unretained_large_response_exposes_only_the_raw_preview() {
        let mut large = response(br#"{"createdAt":1787482800}"#, "application/json");
        large.size = RESPONSE_PAGE_BYTES + 1;
        large.body_complete = false;
        large.body_retention_error = Some("Response cache quota reached.".to_owned());

        let (document, pretty_pending, inspection_pending) = prepare_document(&large, 9);

        assert!(document.truncated);
        assert!(!document.file_backed);
        assert!(document.pretty_text.is_empty());
        assert!(!pretty_pending);
        assert!(!inspection_pending);
        assert_eq!(
            document.inspection.skipped.as_deref(),
            Some("Response cache quota reached.")
        );
        assert!(!document.can_load_next_page());
    }
}
