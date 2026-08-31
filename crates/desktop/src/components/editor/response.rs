use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodySyntax {
    Plain,
    Json,
    Xml,
}

pub(crate) fn body_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    syntax: BodySyntax,
    variables: VariableContext,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let value = value.into();
    let ranges = variable_ranges(&value);
    let decorations = body_text_highlights(theme, &ranges);
    ProbeEditor {
        theme,
        id: id.into(),
        value,
        placeholder: SharedString::from("Body content"),
        decorations,
        language: match syntax {
            BodySyntax::Json => "json".into(),
            BodySyntax::Xml => "xml".into(),
            BodySyntax::Plain => SharedString::default(),
        },
        readonly: false,
        min_height: Some(120.0),
        padding: EditorInsets::standard(theme),
        soft_wrap: true,
        text_color: theme.colors.text.primary,
        scroll_to_range: None,
        search_matches: Vec::new(),
        on_change: Some(Rc::new(on_value_change)),
        on_mouse_down: None,
        on_visible_range: None,
        extra_context_menu_actions: Vec::new(),
        debug_selector: None,
        variables: Some(variables),
    }
    .into_any_element()
}

pub(crate) fn response_body_input(
    theme: Theme,
    id: impl Into<ElementId>,
    text: &str,
    options: ResponseBodyInputOptions<'_>,
) -> gpui::AnyElement {
    let inspection_reveal = options.inspection_reveal;
    let search_matches = response_highlights(
        options.matches,
        options.active_match,
        inspection_reveal.as_ref(),
    );
    let (decorations, scroll_to_range) = response_decorations(
        theme,
        options.matches,
        options.active_match,
        inspection_reveal,
    );
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: text.into(),
            decorations,
            language: options.language,
            soft_wrap: options.soft_wrap,
            text_color: theme.colors.syntax.plain,
            scroll_to_range,
            search_matches,
        },
        options.on_visible_range,
        Some(options.on_mouse_down),
        vec![TextContextMenuExtraAction {
            id: "inspect",
            label: "Inspect",
            requires_selection: false,
            is_enabled: options.inspect_enabled,
            on_click: options.on_inspect,
        }],
    )
}

pub(crate) fn response_headers_input(
    theme: Theme,
    id: impl Into<ElementId>,
    headers: &[probe_http::ResponseHeader],
    matches: &[SearchMatch],
    active_match: usize,
    on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
) -> gpui::AnyElement {
    let joined = join_header_lines(headers);
    let mut decorations = Vec::new();
    for (offset, name_len) in joined.line_offsets.iter().zip(&joined.name_lens) {
        decorations.push(text_decoration(
            *offset..*offset + name_len,
            Some(theme.colors.text.secondary.into()),
            None,
        ));
    }
    let (search_decorations, scroll_to_range) =
        response_decorations(theme, matches, active_match, None);
    decorations.extend(search_decorations);
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: joined.text.into(),
            decorations,
            language: SharedString::default(),
            soft_wrap: true,
            text_color: theme.colors.text.primary,
            scroll_to_range,
            search_matches: response_highlights(matches, active_match, None),
        },
        Rc::new(on_visible_range),
        None,
        Vec::new(),
    )
}

pub(crate) fn response_inspector_input(
    theme: Theme,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
) -> gpui::AnyElement {
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: text.into(),
            decorations: Vec::new(),
            language: SharedString::default(),
            soft_wrap: true,
            text_color: theme.colors.text.primary,
            scroll_to_range: None,
            search_matches: Vec::new(),
        },
        Rc::new(on_visible_range),
        None,
        Vec::new(),
    )
}

fn response_decorations(
    theme: Theme,
    matches: &[SearchMatch],
    active_match: usize,
    inspection_reveal: Option<(Range<usize>, bool)>,
) -> (Vec<TextDecoration>, Option<Range<usize>>) {
    let mut decorations =
        Vec::with_capacity(matches.len() + usize::from(inspection_reveal.is_some()));
    let mut scroll_to_range = None;
    for (index, found) in matches.iter().enumerate() {
        let active = index == active_match;
        if active {
            scroll_to_range = Some(found.range.start..found.range.start);
        }
        decorations.push(search_match_decoration(theme, found.range.clone(), active));
    }
    if let Some((range, should_scroll)) = inspection_reveal {
        if should_scroll {
            scroll_to_range = Some(range.clone());
        }
        decorations.push(search_match_decoration(theme, range, true));
    }
    (decorations, scroll_to_range)
}

fn response_highlights(
    matches: &[SearchMatch],
    active_match: usize,
    inspection_reveal: Option<&(Range<usize>, bool)>,
) -> Vec<(Range<usize>, bool)> {
    let mut highlights = matches
        .iter()
        .enumerate()
        .map(|(index, found)| (found.range.clone(), index == active_match))
        .collect::<Vec<_>>();
    if let Some((range, _)) = inspection_reveal {
        highlights.push((range.clone(), true));
    }
    highlights
}

struct ResponseEditorPresentation {
    value: SharedString,
    decorations: Vec<TextDecoration>,
    language: SharedString,
    soft_wrap: bool,
    text_color: gpui::Rgba,
    scroll_to_range: Option<Range<usize>>,
    search_matches: Vec<(Range<usize>, bool)>,
}

fn response_editor(
    theme: Theme,
    id: impl Into<ElementId>,
    presentation: ResponseEditorPresentation,
    on_visible_range: VisibleRangeHandler,
    on_mouse_down: Option<EditorMouseDownHandler>,
    extra_context_menu_actions: Vec<TextContextMenuExtraAction>,
) -> gpui::AnyElement {
    ProbeEditor {
        theme,
        id: id.into(),
        value: presentation.value,
        placeholder: SharedString::default(),
        decorations: presentation.decorations,
        language: presentation.language,
        readonly: true,
        min_height: None,
        padding: EditorInsets::response(theme),
        soft_wrap: presentation.soft_wrap,
        text_color: presentation.text_color,
        scroll_to_range: presentation.scroll_to_range,
        search_matches: presentation.search_matches,
        on_change: None,
        on_mouse_down,
        on_visible_range: Some(on_visible_range),
        extra_context_menu_actions,
        debug_selector: None,
        variables: None,
    }
    .into_any_element()
}
