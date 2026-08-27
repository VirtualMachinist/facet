//! JSON and XML response inspection for JWTs, Unix timestamps, and Pretty-tab jumps.

use std::{fmt, fs::File, io::BufReader, ops::Range, path::Path};

use chrono::{DateTime, Local, TimeDelta, TimeZone, Utc};
use quick_xml::{
    Reader, XmlVersion,
    events::{BytesStart, Event},
};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};

pub(crate) const INSPECT_MAX_BYTES: usize = 512 * 1024;
const INSPECT_MAX_VALUES: usize = 10_000;
const INSPECTION_LIMIT_REACHED: &str = "probe inspection value limit reached";
const JWT_STANDARD_CLAIMS: &[&str] = &["exp", "iat", "nbf", "iss", "sub"];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResponseInspection {
    pub jwts: Vec<JwtFinding>,
    pub timestamps: Vec<TimestampFinding>,
    pub skipped: Option<String>,
}

impl ResponseInspection {
    pub(crate) fn count(&self) -> usize {
        self.jwts.len() + self.timestamps.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JwtFinding {
    pub path: String,
    pub search: String,
    pub source_range: Option<Range<usize>>,
    pub header_json: String,
    pub payload_json: String,
    pub claims: Vec<JwtClaim>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JwtClaim {
    pub name: String,
    pub value: String,
    pub timestamp: Option<TimestampDisplay>,
    pub relative: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimestampFinding {
    pub path: String,
    pub search: String,
    pub source_range: Option<Range<usize>>,
    pub raw: String,
    pub timestamp: TimestampDisplay,
    pub confidence: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimestampDisplay {
    pub epoch_millis: i64,
    pub millisecond_precision: bool,
}

impl TimestampDisplay {
    pub(crate) fn local(&self) -> String {
        format_millis_local(self.epoch_millis, self.millisecond_precision)
    }

    pub(crate) fn utc(&self) -> String {
        format_millis_utc(self.epoch_millis, self.millisecond_precision)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InspectSelection {
    Jwt(usize),
    Timestamp(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InspectionRange {
    pub range: Range<usize>,
    pub selection: InspectSelection,
}

pub(crate) fn inspect_response_body(body: &[u8]) -> ResponseInspection {
    if body.len() > INSPECT_MAX_BYTES {
        return ResponseInspection {
            skipped: Some("Response is too large for automatic inspection.".to_owned()),
            ..ResponseInspection::default()
        };
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        let mut inspector = JsonInspector::default();
        inspector.visit(&value, &mut Vec::new(), None);
        if inspector.visited >= INSPECT_MAX_VALUES {
            inspector.inspection.skipped =
                Some("Inspection stopped after the first 10000 response values.".to_owned());
        }
        return inspector.inspection;
    }
    let Ok(source) = std::str::from_utf8(body) else {
        return ResponseInspection::default();
    };
    inspect_xml_response(source)
}

/// Inspects a complete JSON response without retaining its document tree.
pub(crate) fn inspect_json_file(path: &Path) -> ResponseInspection {
    let Ok(file) = File::open(path) else {
        return ResponseInspection {
            skipped: Some("Could not read the retained response body.".to_owned()),
            ..ResponseInspection::default()
        };
    };
    let mut inspector = StreamingJsonInspector::default();
    let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(file));
    let result = StreamingJsonSeed {
        inspector: &mut inspector,
        path: Vec::new(),
        key: None,
    }
    .deserialize(&mut deserializer);
    if inspector.limit_reached {
        inspector.inspection.skipped =
            Some("Inspection stopped after the first 10000 response values.".to_owned());
    } else if result.is_err() || deserializer.end().is_err() {
        return ResponseInspection {
            skipped: Some("Response is not valid JSON.".to_owned()),
            ..ResponseInspection::default()
        };
    }
    inspector.inspection
}

/// Inspects a complete XML response using a bounded event buffer.
pub(crate) fn inspect_xml_file(path: &Path) -> ResponseInspection {
    let Ok(file) = File::open(path) else {
        return ResponseInspection {
            skipped: Some("Could not read the retained response body.".to_owned()),
            ..ResponseInspection::default()
        };
    };
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut path = Vec::<String>::new();
    let mut inspection = ResponseInspection::default();
    let mut visited = 0_usize;
    let mut limit_reached = false;
    let mut invalid = false;
    let mut root_seen = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                if path.is_empty() {
                    if root_seen {
                        invalid = true;
                    }
                    root_seen = true;
                }
                let name = element.name().as_ref().to_owned();
                path.push(name);
                match inspect_streaming_xml_attributes(
                    &element,
                    &path,
                    &mut inspection,
                    &mut visited,
                ) {
                    StreamingXmlStatus::Continue => {}
                    StreamingXmlStatus::LimitReached => limit_reached = true,
                    StreamingXmlStatus::Invalid => invalid = true,
                }
            }
            Ok(Event::Empty(element)) => {
                if path.is_empty() {
                    if root_seen {
                        invalid = true;
                    }
                    root_seen = true;
                }
                let name = element.name().as_ref().to_owned();
                path.push(name);
                match inspect_streaming_xml_attributes(
                    &element,
                    &path,
                    &mut inspection,
                    &mut visited,
                ) {
                    StreamingXmlStatus::Continue => {}
                    StreamingXmlStatus::LimitReached => limit_reached = true,
                    StreamingXmlStatus::Invalid => invalid = true,
                }
                path.pop();
            }
            Ok(Event::Text(text)) => {
                if path.is_empty() {
                    invalid = !text.xml10_content().trim().is_empty();
                } else {
                    match quick_xml::escape::unescape(&text.xml10_content()) {
                        Ok(value) => {
                            limit_reached = inspect_streaming_xml_value(
                                value.as_ref(),
                                &format!("/{}", path.join("/")),
                                path.last().map(String::as_str),
                                &mut inspection,
                                &mut visited,
                            );
                        }
                        Err(_) => invalid = true,
                    }
                }
            }
            Ok(Event::CData(text)) => {
                if path.is_empty() {
                    invalid = true;
                } else {
                    limit_reached = inspect_streaming_xml_value(
                        &text.xml10_content(),
                        &format!("/{}", path.join("/")),
                        path.last().map(String::as_str),
                        &mut inspection,
                        &mut visited,
                    );
                }
            }
            Ok(Event::End(_)) => invalid |= path.pop().is_none(),
            Ok(Event::Eof) => {
                invalid |= !root_seen || !path.is_empty();
                break;
            }
            Ok(Event::GeneralRef(_)) if path.is_empty() => invalid = true,
            Err(_) => {
                invalid = true;
                break;
            }
            _ => {}
        }
        if invalid || limit_reached {
            break;
        }
        buffer.clear();
    }
    if invalid {
        return ResponseInspection {
            skipped: Some("Response is not valid XML.".to_owned()),
            ..ResponseInspection::default()
        };
    }
    if limit_reached {
        inspection.skipped =
            Some("Inspection stopped after the first 10000 response values.".to_owned());
    }
    inspection
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamingXmlStatus {
    Continue,
    LimitReached,
    Invalid,
}

fn inspect_streaming_xml_attributes(
    element: &BytesStart<'_>,
    path: &[String],
    inspection: &mut ResponseInspection,
    visited: &mut usize,
) -> StreamingXmlStatus {
    for attribute in element.attributes() {
        let Ok(attribute) = attribute else {
            return StreamingXmlStatus::Invalid;
        };
        let name = attribute.key.as_ref().to_owned();
        let Ok(value) = attribute.normalized_value(XmlVersion::Implicit1_0) else {
            return StreamingXmlStatus::Invalid;
        };
        if inspect_streaming_xml_value(
            value.as_ref(),
            &format!("/{}/@{name}", path.join("/")),
            Some(&name),
            inspection,
            visited,
        ) {
            return StreamingXmlStatus::LimitReached;
        }
    }
    StreamingXmlStatus::Continue
}

fn inspect_streaming_xml_value(
    value: &str,
    path: &str,
    key: Option<&str>,
    inspection: &mut ResponseInspection,
    visited: &mut usize,
) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if *visited >= INSPECT_MAX_VALUES {
        return true;
    }
    *visited += 1;
    if let Some(finding) = inspect_jwt_text(value, path) {
        inspection.jwts.push(finding);
    } else if let Some(finding) = inspect_timestamp_text(value, path, key, false) {
        inspection.timestamps.push(finding);
    }
    false
}

pub(crate) fn inspection_text(inspection: &ResponseInspection) -> String {
    if inspection.is_empty() {
        return inspection
            .skipped
            .clone()
            .unwrap_or_else(|| "JWTs and Unix timestamps are detected automatically.".to_owned());
    }

    let mut text = String::new();
    if !inspection.jwts.is_empty() {
        text.push_str(&format!("JWT [{}]\n", inspection.jwts.len()));
        for jwt in &inspection.jwts {
            text.push_str(&format!("\n{}\n", jwt.path));
            text.push_str("Decoded locally. Signature not verified.\n");
            if !jwt.claims.is_empty() {
                text.push_str("Claims\n");
                for claim in &jwt.claims {
                    text.push_str("  ");
                    text.push_str(&claim.name);
                    text.push_str(": ");
                    text.push_str(&claim.value);
                    if let Some(timestamp) = &claim.timestamp {
                        text.push_str("  Local: ");
                        text.push_str(&timestamp.local());
                        text.push_str("  UTC: ");
                        text.push_str(&timestamp.utc());
                    }
                    if let Some(relative) = &claim.relative {
                        text.push_str("  ");
                        text.push_str(relative);
                    }
                    text.push('\n');
                }
            }
            text.push_str("Header\n");
            text.push_str(&jwt.header_json);
            text.push('\n');
            text.push_str("Payload\n");
            text.push_str(&jwt.payload_json);
            text.push('\n');
        }
    }

    if !inspection.timestamps.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&format!("Timestamps [{}]\n", inspection.timestamps.len()));
        for timestamp in &inspection.timestamps {
            text.push_str(&format!("\n{}\n", timestamp.path));
            text.push_str("  Raw: ");
            text.push_str(&timestamp.raw);
            text.push('\n');
            text.push_str("  Local: ");
            text.push_str(&timestamp.timestamp.local());
            text.push('\n');
            text.push_str("  UTC: ");
            text.push_str(&timestamp.timestamp.utc());
            text.push('\n');
        }
    }

    if let Some(skipped) = &inspection.skipped {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(skipped);
        text.push('\n');
    }
    text
}

pub(crate) fn inspection_detail_text(
    inspection: &ResponseInspection,
    selection: Option<InspectSelection>,
) -> String {
    let Some(selection) = selection else {
        return inspection_text(inspection);
    };
    match selection {
        InspectSelection::Jwt(index) => inspection
            .jwts
            .get(index)
            .map(jwt_detail_text)
            .unwrap_or_else(|| inspection_text(inspection)),
        InspectSelection::Timestamp(index) => inspection
            .timestamps
            .get(index)
            .map(timestamp_detail_text)
            .unwrap_or_else(|| inspection_text(inspection)),
    }
}

pub(crate) fn first_inspection_selection(
    inspection: &ResponseInspection,
) -> Option<InspectSelection> {
    if !inspection.jwts.is_empty() {
        Some(InspectSelection::Jwt(0))
    } else if !inspection.timestamps.is_empty() {
        Some(InspectSelection::Timestamp(0))
    } else {
        None
    }
}

pub(crate) fn inspection_has_selection(
    inspection: &ResponseInspection,
    selection: InspectSelection,
) -> bool {
    match selection {
        InspectSelection::Jwt(index) => index < inspection.jwts.len(),
        InspectSelection::Timestamp(index) => index < inspection.timestamps.len(),
    }
}

pub(crate) fn inspection_selection_at_offset(
    ranges: &[InspectionRange],
    offset: usize,
) -> Option<InspectSelection> {
    ranges
        .iter()
        .find(|entry| entry.range.contains(&offset))
        .map(|entry| entry.selection)
}

pub(crate) fn inspection_value_ranges(
    pretty_text: &str,
    inspection: &ResponseInspection,
) -> Vec<InspectionRange> {
    if inspection.is_empty() {
        return Vec::new();
    }
    let source_ranges = inspection_source_ranges(inspection);
    if !source_ranges.is_empty() {
        return source_ranges;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(pretty_text) else {
        return Vec::new();
    };
    let targets = InspectionTargets::new(inspection);
    let mut cursor = 0;
    let mut ranges = Vec::with_capacity(inspection.count());
    collect_value_ranges(
        &value,
        pretty_text,
        &targets,
        &mut Vec::new(),
        &mut cursor,
        &mut ranges,
    );
    ranges
}

fn inspection_source_ranges(inspection: &ResponseInspection) -> Vec<InspectionRange> {
    let mut ranges = Vec::with_capacity(inspection.count());
    ranges.extend(
        inspection
            .jwts
            .iter()
            .enumerate()
            .filter_map(|(index, finding)| {
                finding.source_range.clone().map(|range| InspectionRange {
                    range,
                    selection: InspectSelection::Jwt(index),
                })
            }),
    );
    ranges.extend(
        inspection
            .timestamps
            .iter()
            .enumerate()
            .filter_map(|(index, finding)| {
                finding.source_range.clone().map(|range| InspectionRange {
                    range,
                    selection: InspectSelection::Timestamp(index),
                })
            }),
    );
    ranges.sort_by_key(|entry| entry.range.start);
    ranges
}

fn collect_value_ranges(
    value: &serde_json::Value,
    pretty_text: &str,
    targets: &InspectionTargets,
    path: &mut Vec<PathSegment>,
    cursor: &mut usize,
    ranges: &mut Vec<InspectionRange>,
) {
    match value {
        serde_json::Value::Object(object) => {
            for (name, child) in object {
                path.push(PathSegment::Key(name.clone()));
                collect_value_ranges(child, pretty_text, targets, path, cursor, ranges);
                path.pop();
            }
        }
        serde_json::Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                path.push(PathSegment::Index(index));
                collect_value_ranges(child, pretty_text, targets, path, cursor, ranges);
                path.pop();
            }
        }
        _ => {
            let path_text = json_path(path);
            let Ok(rendered) = serde_json::to_string_pretty(value) else {
                return;
            };
            let Some(relative_start) = pretty_text[*cursor..].find(&rendered) else {
                return;
            };
            let start = *cursor + relative_start;
            let end = start + rendered.len();
            *cursor = end;
            if let Some(selection) = targets.selection_for_path(&path_text) {
                ranges.push(InspectionRange {
                    range: start..end,
                    selection,
                });
            }
        }
    }
}

struct InspectionTargets {
    entries: Vec<(String, InspectSelection)>,
}

impl InspectionTargets {
    fn new(inspection: &ResponseInspection) -> Self {
        let mut entries = Vec::with_capacity(inspection.count());
        entries.extend(
            inspection
                .jwts
                .iter()
                .enumerate()
                .map(|(index, finding)| (finding.path.clone(), InspectSelection::Jwt(index))),
        );
        entries.extend(
            inspection
                .timestamps
                .iter()
                .enumerate()
                .map(|(index, finding)| (finding.path.clone(), InspectSelection::Timestamp(index))),
        );
        Self { entries }
    }

    fn selection_for_path(&self, path: &str) -> Option<InspectSelection> {
        self.entries
            .iter()
            .find(|(entry_path, _)| entry_path == path)
            .map(|(_, selection)| *selection)
    }
}

fn jwt_detail_text(jwt: &JwtFinding) -> String {
    let mut text = String::new();
    text.push_str("JWT\n\n");
    text.push_str(&jwt.path);
    text.push('\n');
    text.push_str("Decoded locally. Signature not verified.\n");
    if !jwt.claims.is_empty() {
        text.push_str("\nClaims\n");
        for claim in &jwt.claims {
            text.push_str("  ");
            text.push_str(&claim.name);
            text.push_str(": ");
            text.push_str(&claim.value);
            if let Some(timestamp) = &claim.timestamp {
                text.push_str("  Local: ");
                text.push_str(&timestamp.local());
                text.push_str("  UTC: ");
                text.push_str(&timestamp.utc());
            }
            if let Some(relative) = &claim.relative {
                text.push_str("  ");
                text.push_str(relative);
            }
            text.push('\n');
        }
    }
    text.push_str("\nHeader\n");
    text.push_str(&jwt.header_json);
    text.push_str("\n\nPayload\n");
    text.push_str(&jwt.payload_json);
    text
}

fn timestamp_detail_text(timestamp: &TimestampFinding) -> String {
    let mut text = String::new();
    text.push_str("Timestamp\n\n");
    text.push_str(&timestamp.path);
    text.push('\n');
    text.push_str("Raw: ");
    text.push_str(&timestamp.raw);
    text.push('\n');
    text.push_str("Local: ");
    text.push_str(&timestamp.timestamp.local());
    text.push('\n');
    text.push_str("UTC: ");
    text.push_str(&timestamp.timestamp.utc());
    text.push('\n');
    text
}

fn inspect_xml_response(source: &str) -> ResponseInspection {
    let Ok(document) = roxmltree::Document::parse(source) else {
        return ResponseInspection::default();
    };
    let mut inspector = XmlInspector {
        source,
        ..XmlInspector::default()
    };
    inspector.visit_element(document.root_element(), String::new());
    if inspector.visited >= INSPECT_MAX_VALUES {
        inspector.inspection.skipped =
            Some("Inspection stopped after the first 10000 response values.".to_owned());
    }
    inspector.inspection
}

#[derive(Default)]
struct XmlInspector<'a> {
    source: &'a str,
    inspection: ResponseInspection,
    visited: usize,
}

impl XmlInspector<'_> {
    fn visit_element(&mut self, element: roxmltree::Node<'_, '_>, parent_path: String) {
        if self.visited >= INSPECT_MAX_VALUES {
            return;
        }
        let name = xml_element_name(element);
        let segment = xml_element_segment(element, &name);
        let path = if parent_path.is_empty() {
            format!("/{segment}")
        } else {
            format!("{parent_path}/{segment}")
        };

        for attribute in element.attributes() {
            if self.visited >= INSPECT_MAX_VALUES {
                return;
            }
            let attribute_name = xml_attribute_name(element, attribute);
            self.inspect_scalar(
                attribute.value(),
                format!("{path}/@{attribute_name}"),
                Some(attribute.name()),
                attribute.range_value(),
            );
        }

        let text_count = element.children().filter(|child| child.is_text()).count();
        let mut text_index = 0;
        for child in element.children() {
            if self.visited >= INSPECT_MAX_VALUES {
                return;
            }
            if child.is_element() {
                self.visit_element(child, path.clone());
            } else if child.is_text() {
                text_index += 1;
                let text_path = if text_count == 1 {
                    path.clone()
                } else {
                    format!("{path}/text()[{text_index}]")
                };
                self.inspect_scalar(
                    child.text().unwrap_or_default(),
                    text_path,
                    Some(element.tag_name().name()),
                    child.range(),
                );
            }
        }
    }

    fn inspect_scalar(
        &mut self,
        value: &str,
        path: String,
        key: Option<&str>,
        source_range: Range<usize>,
    ) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        self.visited += 1;
        let source_range = xml_scalar_source_range(self.source, source_range, value);
        if let Some(mut finding) = inspect_jwt_text(value, &path) {
            finding.source_range = Some(source_range);
            self.inspection.jwts.push(finding);
        } else if let Some(mut finding) = inspect_timestamp_text(value, &path, key, false) {
            finding.source_range = Some(source_range);
            self.inspection.timestamps.push(finding);
        }
    }
}

fn xml_element_name(element: roxmltree::Node<'_, '_>) -> String {
    let tag = element.tag_name();
    tag.namespace()
        .and_then(|namespace| element.lookup_prefix(namespace))
        .map(|prefix| format!("{prefix}:{}", tag.name()))
        .unwrap_or_else(|| tag.name().to_owned())
}

fn xml_attribute_name(
    element: roxmltree::Node<'_, '_>,
    attribute: roxmltree::Attribute<'_, '_>,
) -> String {
    attribute
        .namespace()
        .and_then(|namespace| element.lookup_prefix(namespace))
        .map(|prefix| format!("{prefix}:{}", attribute.name()))
        .unwrap_or_else(|| attribute.name().to_owned())
}

fn xml_element_segment(element: roxmltree::Node<'_, '_>, name: &str) -> String {
    let Some(parent) = element.parent().filter(|parent| parent.is_element()) else {
        return name.to_owned();
    };
    let tag = element.tag_name();
    let same_name = |sibling: &roxmltree::Node<'_, '_>| {
        sibling.is_element()
            && sibling.tag_name().name() == tag.name()
            && sibling.tag_name().namespace() == tag.namespace()
    };
    let count = parent.children().filter(same_name).count();
    if count <= 1 {
        return name.to_owned();
    }
    let index = parent
        .children()
        .take_while(|sibling| *sibling != element)
        .filter(same_name)
        .count()
        + 1;
    format!("{name}[{index}]")
}

fn xml_scalar_source_range(source: &str, range: Range<usize>, value: &str) -> Range<usize> {
    source
        .get(range.clone())
        .and_then(|raw| raw.find(value))
        .map(|offset| range.start + offset..range.start + offset + value.len())
        .unwrap_or(range)
}

#[derive(Default)]
struct JsonInspector {
    inspection: ResponseInspection,
    visited: usize,
}

impl JsonInspector {
    fn visit(&mut self, value: &serde_json::Value, path: &mut Vec<PathSegment>, key: Option<&str>) {
        if self.visited >= INSPECT_MAX_VALUES {
            return;
        }
        self.visited += 1;

        let path_text = json_path(path);
        if let Some(finding) = inspect_jwt(value, &path_text) {
            self.inspection.jwts.push(finding);
        } else if let Some(finding) = inspect_timestamp(value, &path_text, key, false) {
            self.inspection.timestamps.push(finding);
        }

        match value {
            serde_json::Value::Object(object) => {
                for (name, child) in object {
                    path.push(PathSegment::Key(name.clone()));
                    self.visit(child, path, Some(name));
                    path.pop();
                }
            }
            serde_json::Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    path.push(PathSegment::Index(index));
                    self.visit(child, path, key);
                    path.pop();
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Default)]
struct StreamingJsonInspector {
    inspection: ResponseInspection,
    visited: usize,
    limit_reached: bool,
}

impl StreamingJsonInspector {
    fn string(&mut self, value: &str, path: &[PathSegment], key: Option<&str>) {
        if self.visited >= INSPECT_MAX_VALUES {
            return;
        }
        self.visited += 1;
        let path = json_path(path);
        if let Some(finding) = inspect_jwt_text(value, &path) {
            self.inspection.jwts.push(finding);
        } else if let Some(finding) = inspect_timestamp_text(value, &path, key, false) {
            self.inspection.timestamps.push(finding);
        }
    }

    fn scalar(&mut self, value: serde_json::Value, path: &[PathSegment], key: Option<&str>) {
        if self.visited >= INSPECT_MAX_VALUES {
            return;
        }
        self.visited += 1;
        let path = json_path(path);
        if let Some(finding) = inspect_jwt(&value, &path) {
            self.inspection.jwts.push(finding);
        } else if let Some(finding) = inspect_timestamp(&value, &path, key, false) {
            self.inspection.timestamps.push(finding);
        }
    }
}

struct StreamingJsonSeed<'a> {
    inspector: &'a mut StreamingJsonInspector,
    path: Vec<PathSegment>,
    key: Option<String>,
}

impl<'de> DeserializeSeed<'de> for StreamingJsonSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if self.inspector.visited >= INSPECT_MAX_VALUES {
            self.inspector.limit_reached = true;
            return Err(D::Error::custom(INSPECTION_LIMIT_REACHED));
        }
        deserializer.deserialize_any(StreamingJsonVisitor(self))
    }
}

struct StreamingJsonVisitor<'a>(StreamingJsonSeed<'a>);

impl<'de> Visitor<'de> for StreamingJsonVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        self.0.inspector.visited = self.0.inspector.visited.saturating_add(1);
        while let Some(key) = map.next_key::<String>()? {
            let mut path = self.0.path.clone();
            path.push(PathSegment::Key(key.clone()));
            map.next_value_seed(StreamingJsonSeed {
                inspector: self.0.inspector,
                path,
                key: Some(key),
            })?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        self.0.inspector.visited = self.0.inspector.visited.saturating_add(1);
        let mut index = 0;
        loop {
            let mut path = self.0.path.clone();
            path.push(PathSegment::Index(index));
            if sequence
                .next_element_seed(StreamingJsonSeed {
                    inspector: self.0.inspector,
                    path,
                    key: self.0.key.clone(),
                })?
                .is_none()
            {
                break;
            }
            index += 1;
        }
        Ok(())
    }

    fn visit_str<E>(self, value: &str) -> Result<(), E> {
        self.0
            .inspector
            .string(value, &self.0.path, self.0.key.as_deref());
        Ok(())
    }

    fn visit_string<E>(self, value: String) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&value)
    }

    fn visit_i64<E>(self, value: i64) -> Result<(), E> {
        self.0.inspector.scalar(
            serde_json::Value::Number(value.into()),
            &self.0.path,
            self.0.key.as_deref(),
        );
        Ok(())
    }

    fn visit_u64<E>(self, value: u64) -> Result<(), E> {
        self.0.inspector.scalar(
            serde_json::Value::Number(value.into()),
            &self.0.path,
            self.0.key.as_deref(),
        );
        Ok(())
    }

    fn visit_f64<E>(self, value: f64) -> Result<(), E> {
        if let Some(number) = serde_json::Number::from_f64(value) {
            self.0.inspector.scalar(
                serde_json::Value::Number(number),
                &self.0.path,
                self.0.key.as_deref(),
            );
        }
        Ok(())
    }

    fn visit_bool<E>(self, value: bool) -> Result<(), E> {
        self.0.inspector.scalar(
            serde_json::Value::Bool(value),
            &self.0.path,
            self.0.key.as_deref(),
        );
        Ok(())
    }

    fn visit_none<E>(self) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        self.visit_unit()
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        self.0
            .inspector
            .scalar(serde_json::Value::Null, &self.0.path, self.0.key.as_deref());
        Ok(())
    }
}

fn json_path(path: &[PathSegment]) -> String {
    let mut text = String::new();
    for segment in path {
        match segment {
            PathSegment::Key(key) => {
                if text.is_empty() {
                    text.push_str(key);
                } else {
                    text.push('.');
                    text.push_str(key);
                }
            }
            PathSegment::Index(index) => {
                text.push('[');
                text.push_str(&index.to_string());
                text.push(']');
            }
        }
    }
    if text.is_empty() {
        "$".to_owned()
    } else {
        text
    }
}

fn inspect_jwt(value: &serde_json::Value, path: &str) -> Option<JwtFinding> {
    inspect_jwt_text(value.as_str()?, path)
}

fn inspect_jwt_text(token: &str, path: &str) -> Option<JwtFinding> {
    if token.matches('.').count() != 2 {
        return None;
    }
    let mut parts = token.split('.');
    let header = decode_base64url_json_object(parts.next()?)?;
    let payload = decode_base64url_json_object(parts.next()?)?;
    let signature = parts.next()?;
    if !base64url_candidate(signature) {
        return None;
    }

    let mut confidence = 0;
    if header.get("alg").is_some() {
        confidence += 2;
    }
    if header.get("typ").and_then(serde_json::Value::as_str) == Some("JWT") {
        confidence += 1;
    }
    confidence += JWT_STANDARD_CLAIMS
        .iter()
        .filter(|claim| payload.get(**claim).is_some())
        .count()
        .min(3);
    if confidence < 2 {
        return None;
    }

    let claims = JWT_STANDARD_CLAIMS
        .iter()
        .filter_map(|claim| jwt_claim(&payload, claim))
        .collect();
    Some(JwtFinding {
        path: path.to_owned(),
        search: token.to_owned(),
        source_range: None,
        header_json: serde_json::to_string_pretty(&header).ok()?,
        payload_json: serde_json::to_string_pretty(&payload).ok()?,
        claims,
    })
}

fn jwt_claim(payload: &serde_json::Map<String, serde_json::Value>, name: &str) -> Option<JwtClaim> {
    let value = payload.get(name)?;
    let timestamp = matches!(name, "exp" | "iat" | "nbf")
        .then(|| inspect_timestamp(value, name, Some(name), true))
        .flatten()
        .map(|finding| finding.timestamp);
    let relative = if name == "exp" {
        timestamp
            .as_ref()
            .map(|timestamp| expiration_relative(timestamp.epoch_millis))
    } else {
        None
    };
    Some(JwtClaim {
        name: name.to_owned(),
        value: value_to_compact_string(value),
        timestamp,
        relative,
    })
}

fn decode_base64url_json_object(
    segment: &str,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    if !base64url_candidate(segment) {
        return None;
    }
    let bytes = decode_base64url(segment)?;
    match serde_json::from_slice::<serde_json::Value>(&bytes).ok()? {
        serde_json::Value::Object(object) => Some(object),
        _ => None,
    }
}

fn base64url_candidate(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn decode_base64url(segment: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut bit_count = 0u8;
    let mut output = Vec::with_capacity(segment.len() * 3 / 4);
    for byte in segment.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        bits = (bits << 6) | value;
        bit_count += 6;
        while bit_count >= 8 {
            bit_count -= 8;
            output.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    Some(output)
}

fn inspect_timestamp(
    value: &serde_json::Value,
    path: &str,
    key: Option<&str>,
    explicit: bool,
) -> Option<TimestampFinding> {
    let (raw, number) = timestamp_number(value)?;
    inspect_timestamp_number(raw, number, path, key, explicit)
}

fn inspect_timestamp_text(
    raw: &str,
    path: &str,
    key: Option<&str>,
    explicit: bool,
) -> Option<TimestampFinding> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = raw.parse::<i64>().ok()?;
    inspect_timestamp_number(raw.to_owned(), number, path, key, explicit)
}

fn inspect_timestamp_number(
    raw: String,
    number: i64,
    path: &str,
    key: Option<&str>,
    explicit: bool,
) -> Option<TimestampFinding> {
    let candidate = classify_unix_timestamp(number)?;
    let mut confidence = if explicit {
        8
    } else {
        candidate.base_confidence
    };
    if let Some(key) = key {
        confidence += timestamp_key_score(key);
    }
    if confidence < 5 {
        return None;
    }
    Some(TimestampFinding {
        path: path.to_owned(),
        search: raw.clone(),
        source_range: None,
        raw,
        timestamp: TimestampDisplay {
            epoch_millis: candidate.epoch_millis,
            millisecond_precision: candidate.millisecond_precision,
        },
        confidence,
    })
}

struct TimestampCandidate {
    epoch_millis: i64,
    millisecond_precision: bool,
    base_confidence: u8,
}

fn timestamp_number(value: &serde_json::Value) -> Option<(String, i64)> {
    match value {
        serde_json::Value::Number(number) => {
            number.as_i64().map(|value| (value.to_string(), value))
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.len() == text.len()
                && !trimmed.is_empty()
                && trimmed.bytes().all(|byte| byte.is_ascii_digit())
            {
                trimmed
                    .parse::<i64>()
                    .ok()
                    .map(|value| (text.clone(), value))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn classify_unix_timestamp(number: i64) -> Option<TimestampCandidate> {
    let digits = number.unsigned_abs().to_string().len();
    let (epoch_millis, millisecond_precision, base_confidence) = match digits {
        10 => (number.checked_mul(1000)?, false, 3),
        13 => (number, true, 3),
        _ => return None,
    };
    if !plausible_epoch_millis(epoch_millis) {
        return None;
    }
    Some(TimestampCandidate {
        epoch_millis,
        millisecond_precision,
        base_confidence,
    })
}

fn plausible_epoch_millis(epoch_millis: i64) -> bool {
    let start = Utc
        .with_ymd_and_hms(2000, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_millis();
    let end = Utc
        .with_ymd_and_hms(2100, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_millis();
    (start..end).contains(&epoch_millis)
}

fn timestamp_key_score(key: &str) -> u8 {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if matches!(
        normalized.as_str(),
        "id" | "userid" | "orderid" | "count" | "code" | "statuscode" | "zip" | "postalcode"
    ) || normalized.ends_with("id")
    {
        return 0;
    }
    if matches!(
        normalized.as_str(),
        "timestamp"
            | "createdat"
            | "updatedat"
            | "expiresat"
            | "issuedat"
            | "created"
            | "updated"
            | "expires"
            | "lastlogin"
            | "date"
            | "time"
    ) || normalized.ends_with("date")
        || normalized.ends_with("time")
        || normalized.contains("timestamp")
    {
        return 3;
    }
    1
}

fn value_to_compact_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn format_millis_local(epoch_millis: i64, precise: bool) -> String {
    let Some(datetime) = DateTime::from_timestamp_millis(epoch_millis) else {
        return "Invalid timestamp".to_owned();
    };
    format_datetime(datetime.with_timezone(&Local), precise)
}

fn format_millis_utc(epoch_millis: i64, precise: bool) -> String {
    let Some(datetime) = DateTime::from_timestamp_millis(epoch_millis) else {
        return "Invalid timestamp".to_owned();
    };
    format_datetime(datetime, precise)
}

fn format_datetime<Tz: TimeZone>(datetime: DateTime<Tz>, precise: bool) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if precise {
        datetime.format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string()
    } else {
        datetime.format("%Y-%m-%d %H:%M:%S %:z").to_string()
    }
}

fn expiration_relative(epoch_millis: i64) -> String {
    let now = Utc::now().timestamp_millis();
    let delta = epoch_millis - now;
    let duration = TimeDelta::try_milliseconds(delta.abs()).unwrap_or(TimeDelta::MAX);
    let text = if duration.num_days() > 0 {
        format!("{}d", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{}h", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{}m", duration.num_minutes())
    } else {
        format!("{}s", duration.num_seconds())
    };
    if delta >= 0 {
        format!("Expires in {text}")
    } else {
        format!("Expired {text} ago")
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        InspectSelection, ResponseInspection, inspect_json_file, inspect_response_body,
        inspect_xml_file, inspection_detail_text, inspection_selection_at_offset, inspection_text,
        inspection_value_ranges,
    };

    fn inspect_temp_source(
        suffix: &str,
        source: impl AsRef<[u8]>,
        inspect: fn(&Path) -> ResponseInspection,
    ) -> ResponseInspection {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "probe-streaming-inspection-{}-{unique}.{suffix}",
            std::process::id()
        ));
        std::fs::write(&path, source).unwrap();
        let inspection = inspect(&path);
        std::fs::remove_file(path).unwrap();
        inspection
    }

    #[test]
    fn inspection_detects_structurally_valid_jwts() {
        let token = concat!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
            "eyJzdWIiOiIxMjMiLCJpYXQiOjE3ODc0ODI4MDAsImV4cCI6MTc4NzQ4NjQwMH0.",
            "signature"
        );
        let source = format!(r#"{{"authResponse":{{"accessToken":"{token}"}}}}"#);
        let inspection = inspect_response_body(source.as_bytes());

        assert_eq!(inspection.jwts.len(), 1);
        assert_eq!(inspection.jwts[0].path, "authResponse.accessToken");
        assert!(inspection.jwts[0].header_json.contains("\"alg\""));
        assert!(
            inspection.jwts[0]
                .claims
                .iter()
                .any(|claim| claim.name == "exp" && claim.timestamp.is_some())
        );
    }

    #[test]
    fn streaming_inspection_reaches_findings_after_the_memory_preview() {
        let padding = "x".repeat(1024 * 1024);
        let json = inspect_temp_source(
            "json",
            format!(r#"{{"padding":"{padding}","createdAt":1787482800}}"#),
            inspect_json_file,
        );
        assert_eq!(json.timestamps.len(), 1);
        assert_eq!(json.timestamps[0].path, "createdAt");

        let xml = inspect_temp_source(
            "xml",
            format!(r#"<root><padding>{padding}</padding><item createdAt="1787482800"/></root>"#),
            inspect_xml_file,
        );
        assert_eq!(xml.timestamps.len(), 1);
        assert_eq!(xml.timestamps[0].path, "/root/item/@createdAt");
    }

    #[test]
    fn streaming_inspection_rejects_invalid_input_without_partial_findings() {
        let json = inspect_temp_source(
            "json",
            r#"{"createdAt":1787482800} trailing"#,
            inspect_json_file,
        );
        assert!(json.timestamps.is_empty());
        assert_eq!(json.skipped.as_deref(), Some("Response is not valid JSON."));

        let xml = inspect_temp_source(
            "xml",
            r#"<root createdAt="1787482800"><broken></root>"#,
            inspect_xml_file,
        );
        assert!(xml.timestamps.is_empty());
        assert_eq!(xml.skipped.as_deref(), Some("Response is not valid XML."));
    }

    #[test]
    fn streaming_json_inspection_stops_at_the_value_limit() {
        let source = format!(
            "[{},{{\"createdAt\":1787482800}}]",
            std::iter::repeat_n("0", 10_000)
                .collect::<Vec<_>>()
                .join(",")
        );
        let inspection = inspect_temp_source("json", source, inspect_json_file);

        assert!(inspection.timestamps.is_empty());
        assert_eq!(
            inspection.skipped.as_deref(),
            Some("Inspection stopped after the first 10000 response values.")
        );
    }

    #[test]
    fn streaming_xml_inspection_reads_cdata_values() {
        let inspection = inspect_temp_source(
            "xml",
            r#"<root><createdAt><![CDATA[1787482800]]></createdAt></root>"#,
            inspect_xml_file,
        );

        assert_eq!(inspection.timestamps.len(), 1);
        assert_eq!(inspection.timestamps[0].path, "/root/createdAt");
    }

    #[test]
    fn xml_inspection_detects_attribute_and_element_values() {
        let token = concat!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
            "eyJzdWIiOiIxMjMiLCJpYXQiOjE3ODc0ODI4MDAsImV4cCI6MTc4NzQ4NjQwMH0.",
            "signature"
        );
        let source = format!(
            r#"<response createdAt="1787482800"><accessToken>{token}</accessToken><item updated_at="1787482800123"/></response>"#
        );
        let inspection = inspect_response_body(source.as_bytes());

        assert_eq!(inspection.jwts.len(), 1);
        assert_eq!(inspection.jwts[0].path, "/response/accessToken");
        assert_eq!(inspection.timestamps.len(), 2);
        assert!(
            inspection
                .timestamps
                .iter()
                .any(|finding| finding.path == "/response/@createdAt")
        );
        assert!(
            inspection
                .timestamps
                .iter()
                .any(|finding| finding.path == "/response/item/@updated_at")
        );

        let ranges = inspection_value_ranges(&source, &inspection);
        assert_eq!(ranges.len(), 3);
        for range in ranges {
            let selected = &source[range.range];
            assert!(selected == token || selected.starts_with("1787482800"));
        }
    }

    #[test]
    fn xml_inspection_paths_include_namespaces_and_repeated_sibling_indexes() {
        let source = r#"<n:response xmlns:n="urn:test"><n:item createdAt="1787482800"/><n:item createdAt="1787486400"/></n:response>"#;
        let inspection = inspect_response_body(source.as_bytes());
        let paths = inspection
            .timestamps
            .iter()
            .map(|finding| finding.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            paths,
            vec![
                "/n:response/n:item[1]/@createdAt",
                "/n:response/n:item[2]/@createdAt"
            ]
        );
    }

    #[test]
    fn inspection_rejects_jwt_shaped_strings_that_do_not_decode() {
        let inspection = inspect_response_body(br#"{"token":"abc.def.ghi"}"#);
        assert!(inspection.jwts.is_empty());
    }

    #[test]
    fn timestamp_inspection_uses_field_semantics() {
        let inspection = inspect_response_body(
            br#"{"createdAt":1787482800,"userId":1787482800,"updated_at":1787482800123}"#,
        );

        let paths: Vec<_> = inspection
            .timestamps
            .iter()
            .map(|timestamp| timestamp.path.as_str())
            .collect();
        assert!(paths.contains(&"createdAt"));
        assert!(paths.contains(&"updated_at"));
        assert!(!paths.contains(&"userId"));
        assert!(
            inspection
                .timestamps
                .iter()
                .any(|timestamp| timestamp.path == "updated_at"
                    && timestamp.timestamp.millisecond_precision)
        );
    }

    #[test]
    fn inspection_skips_very_large_bodies() {
        let body = format!(
            r#"{{"createdAt":{},"padding":"{}"}}"#,
            1_787_482_800,
            "x".repeat(600 * 1024)
        );
        let inspection = inspect_response_body(body.as_bytes());

        assert!(inspection.timestamps.is_empty());
        assert!(inspection.skipped.is_some());
    }

    #[test]
    fn inspection_text_keeps_jwt_times_close_to_claim_values() {
        let token = concat!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
            "eyJzdWIiOiIxMjMiLCJpYXQiOjE3ODc0ODI4MDAsImV4cCI6MTc4NzQ4NjQwMH0.",
            "signature"
        );
        let source = format!(r#"{{"accessToken":"{token}"}}"#);
        let report = inspection_text(&inspect_response_body(source.as_bytes()));

        assert!(report.contains("JWT [1]"));
        assert!(report.contains("exp: 1787486400  Local:"));
        assert!(report.contains("Signature not verified"));
    }

    #[test]
    fn inspection_detail_text_renders_one_selected_finding() {
        let inspection =
            inspect_response_body(br#"{"createdAt":1787482800,"updatedAt":1787486400}"#);
        let detail = inspection_detail_text(&inspection, Some(InspectSelection::Timestamp(1)));

        assert!(detail.starts_with("Timestamp"));
        assert!(detail.contains("updatedAt"));
        assert!(detail.contains("Raw: 1787486400"));
        assert!(!detail.contains("createdAt"));
    }

    #[test]
    fn inspection_ranges_follow_paths_not_duplicate_values() {
        let inspection =
            inspect_response_body(br#"{"createdAt":1787482800,"updatedAt":1787482800}"#);
        let pretty = serde_json::to_string_pretty(
            &serde_json::json!({"createdAt": 1787482800, "updatedAt": 1787482800}),
        )
        .unwrap();
        let ranges = inspection_value_ranges(&pretty, &inspection);

        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].selection, InspectSelection::Timestamp(0));
        assert_eq!(ranges[1].selection, InspectSelection::Timestamp(1));
        assert_eq!(
            inspection_selection_at_offset(&ranges, ranges[1].range.start + 2),
            Some(InspectSelection::Timestamp(1))
        );
    }

    #[test]
    fn inspection_ranges_do_not_match_substrings_in_other_values() {
        let inspection = inspect_response_body(br#"{"createdAt":1787482800,"label":"1787482800"}"#);
        let pretty = serde_json::to_string_pretty(
            &serde_json::json!({"createdAt": 1787482800, "label": "1787482800"}),
        )
        .unwrap();
        let ranges = inspection_value_ranges(&pretty, &inspection);

        assert_eq!(ranges.len(), 1);
        let label_offset = pretty.find("\"1787482800\"").unwrap() + 1;
        assert_eq!(inspection_selection_at_offset(&ranges, label_offset), None);
    }

    #[test]
    fn inspection_ranges_skip_duplicate_non_targets_before_targets() {
        let inspection = inspect_response_body(br#"{"label":"1787482800","createdAt":1787482800}"#);
        let pretty = serde_json::to_string_pretty(
            &serde_json::json!({"label": "1787482800", "createdAt": 1787482800}),
        )
        .unwrap();
        let ranges = inspection_value_ranges(&pretty, &inspection);
        let label_offset = pretty.find("\"1787482800\"").unwrap() + 1;
        let timestamp_offset = pretty.rfind("1787482800").unwrap() + 1;

        assert_eq!(ranges.len(), 1);
        assert_eq!(inspection_selection_at_offset(&ranges, label_offset), None);
        assert_eq!(
            inspection_selection_at_offset(&ranges, timestamp_offset),
            Some(InspectSelection::Timestamp(0))
        );
    }
}
