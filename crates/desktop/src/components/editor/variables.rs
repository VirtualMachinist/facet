use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::components) fn variable_input_overlay(
    theme: Theme,
    state: Entity<InputState>,
    tooltip_id: ElementId,
    input: impl IntoElement,
    value: SharedString,
    variables: VariableContext,
    highlight_path_variables: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let ranges = input_variable_ranges(&value, highlight_path_variables);
    // Input paints first so it keeps native caret, selection, and scroll.
    // The overlay sits on top, recolors supported variable spans, and covers
    // the native caret while blink is off.
    let mut wrapper = div()
        .id(tooltip_id.clone())
        .relative()
        .debug_selector(|| "variable-input-tooltip-trigger".into())
        .w_full()
        .child(input)
        .child(variable_highlight_layer(
            theme,
            state.clone(),
            ranges.is_empty(),
            theme.typography.monospace_family,
            theme.typography.body_size,
            highlight_path_variables,
        ));
    let tooltip_ranges = variable_ranges(&value);
    if tooltip_ranges.is_empty() {
        return wrapper.into_any_element();
    }

    let hover = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(tooltip_id), SharedString::from("hover")),
        cx,
        VariableHoverState::new,
    );
    let visible_width = hover.read(cx).visible_width;
    let current_scroll = state.read(cx).scroll_offset().x;
    let cursor = state.read(cx).cursor();
    let spans = variable_span_layout(
        window,
        &value,
        &tooltip_ranges,
        theme.typography.monospace_family,
        theme.typography.body_size,
        current_scroll,
        cursor,
        visible_width,
    );
    let mut hits = div().relative().w_full().h_full().on_prepaint({
        let hover = hover.clone();
        move |bounds, window, cx| {
            let width = bounds.size.width;
            let changed = hover.update(cx, |state, _| {
                let changed = state.visible_width != Some(width);
                state.visible_width = Some(width);
                changed
            });
            if changed {
                window.request_animation_frame();
            }
        }
    });
    for (index, (name, left, width)) in spans.into_iter().enumerate() {
        hits = hits.child(
            variable_hover_hit(
                ("variable-hover", index),
                index,
                name,
                hover.clone(),
                left,
                px(0.0),
                width,
                None,
                if index == 0 {
                    "variable-hover-trigger".into()
                } else {
                    format!("variable-hover-trigger-{index}")
                },
            )
            .top(px(0.0))
            .bottom(px(0.0)),
        );
    }
    wrapper = wrapper.child(
        div()
            .absolute()
            .top(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .border_1()
            .border_color(transparent_black())
            .px(px(theme.metrics.spacing_2))
            .overflow_hidden()
            .flex()
            .items_center()
            .child(hits),
    );

    with_variable_tooltip(wrapper, theme, hover, variables, cx)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn variable_editor_overlay(
    theme: Theme,
    state: Entity<EditorState>,
    tooltip_id: ElementId,
    editor: impl IntoElement,
    value: SharedString,
    variables: VariableContext,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let ranges = variable_ranges(&value);
    let mut wrapper = div()
        .id(tooltip_id.clone())
        .relative()
        .size_full()
        .min_h(px(0.0))
        .child(editor);
    if ranges.is_empty() {
        return wrapper.into_any_element();
    }

    let hover = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(tooltip_id), SharedString::from("hover")),
        cx,
        VariableHoverState::new,
    );
    let overlay_origin = hover.read(cx).overlay_origin;
    let mut hits = div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .overflow_hidden()
        .on_prepaint({
            let hover = hover.clone();
            move |bounds, window, cx| {
                let origin = bounds.origin;
                let changed = hover.update(cx, |state, _| {
                    let changed = state.overlay_origin != Some(origin);
                    state.overlay_origin = Some(origin);
                    changed
                });
                if changed {
                    window.request_animation_frame();
                }
            }
        });
    if let Some(origin) = overlay_origin {
        let editor = state.read(cx);
        if editor.visible_row_range().is_none() {
            window.request_animation_frame();
        }
        for (index, (range, name)) in ranges.into_iter().enumerate() {
            let Some(bounds) = editor.range_to_bounds(&range) else {
                continue;
            };
            hits = hits.child(variable_hover_hit(
                ("body-variable-hover", index),
                index,
                name,
                hover.clone(),
                bounds.origin.x - origin.x,
                bounds.origin.y - origin.y,
                bounds.size.width.max(px(1.0)),
                Some(bounds.size.height.max(px(1.0))),
                if index == 0 {
                    "body-variable-hover-trigger".into()
                } else {
                    format!("body-variable-hover-trigger-{index}")
                },
            ));
        }
    } else {
        window.request_animation_frame();
    }
    wrapper = wrapper.child(hits);
    with_variable_tooltip(wrapper, theme, hover, variables, cx)
}

#[allow(clippy::too_many_arguments)]
fn variable_hover_hit(
    id: impl Into<ElementId>,
    index: usize,
    name: String,
    hover: Entity<VariableHoverState>,
    left: Pixels,
    top: Pixels,
    width: Pixels,
    height: Option<Pixels>,
    debug_selector: String,
) -> gpui::Stateful<gpui::Div> {
    let hover_trigger = hover.clone();
    div()
        .id(id)
        .absolute()
        .left(left)
        .top(top)
        .w(width)
        .when_some(height, |hit, height| hit.h(height))
        .debug_selector(move || debug_selector.clone())
        .on_hover({
            let hover = hover_trigger.clone();
            move |hovered, _, cx| {
                hover.update(cx, |state, cx| {
                    state.on_trigger_hover(index, name.clone(), *hovered, cx);
                });
            }
        })
        .on_prepaint({
            let hover = hover_trigger;
            move |bounds, window, cx| {
                let changed = hover.update(cx, |state, _| {
                    if state
                        .active
                        .as_ref()
                        .is_none_or(|(active, _)| *active != index)
                    {
                        return false;
                    }
                    let changed = state.trigger_bounds != bounds;
                    state.trigger_bounds = bounds;
                    changed
                });
                if changed {
                    window.request_animation_frame();
                }
            }
        })
}

fn with_variable_tooltip(
    wrapper: gpui::Stateful<gpui::Div>,
    theme: Theme,
    hover: Entity<VariableHoverState>,
    variables: VariableContext,
    cx: &App,
) -> gpui::AnyElement {
    let (open, active, bounds) = {
        let state = hover.read(cx);
        (state.open, state.active.clone(), state.trigger_bounds)
    };
    if !open {
        return wrapper.into_any_element();
    }
    let Some((_, name)) = active else {
        return wrapper.into_any_element();
    };
    if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
        return wrapper.into_any_element();
    }
    let presentation = variable_tooltip_presentation(&name, &variables);
    let value_input = hover.read(cx).value_input.clone();
    *hover.read(cx).on_value_change.borrow_mut() = variables.on_change.clone();
    wrapper
        .child(
            deferred(
                Positioner::side(bounds)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.0))
                    .margin(px(4.0))
                    .child(variable_tooltip_popup(
                        theme,
                        name,
                        presentation,
                        hover,
                        value_input,
                        variables.on_manage_environments,
                    )),
            )
            .with_priority(POPUP_PRIORITY + 1),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(in crate::components) fn variable_span_layout(
    window: &mut Window,
    value: &str,
    ranges: &[(Range<usize>, String)],
    font_family: &'static str,
    font_size: f32,
    current_scroll_x: Pixels,
    cursor: usize,
    visible_width: Option<Pixels>,
) -> Vec<(String, Pixels, Pixels)> {
    let run = TextRun {
        len: value.len(),
        font: font(font_family),
        color: transparent_black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line =
        window
            .text_system()
            .shape_line(SharedString::from(value), px(font_size), &[run], None);
    let scroll_x = visible_width.map_or(current_scroll_x, |width| {
        input_text_scroll_offset(
            line.x_for_index(cursor),
            line.width,
            width,
            current_scroll_x,
        )
    });
    ranges
        .iter()
        .map(|(range, name)| {
            let start = line.x_for_index(range.start) + scroll_x;
            let end = line.x_for_index(range.end) + scroll_x;
            (name.clone(), start, (end - start).max(px(1.0)))
        })
        .collect()
}

fn variable_highlight_layer(
    theme: Theme,
    state: Entity<InputState>,
    ranges_empty: bool,
    font_family: &'static str,
    text_size: f32,
    highlight_path_variables: bool,
) -> impl IntoElement {
    let highlight_color = theme.colors.syntax.string.into();
    let base_color = if ranges_empty {
        transparent_black()
    } else {
        theme.colors.text.primary.into()
    };
    div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        // Match the input's border box so highlight rects share its origin
        // and visible width.
        .border_1()
        .border_color(transparent_black())
        .px(px(theme.metrics.spacing_2))
        .items_center()
        .flex()
        .overflow_hidden()
        .font_family(font_family)
        .text_size(px(text_size))
        .when(!ranges_empty, |layer| {
            layer.debug_selector(|| "variable-highlight-overlay".into())
        })
        .child(VariableHighlightElement {
            state,
            base_color,
            highlight_color,
            highlight_path_variables,
        })
}

pub(in crate::components) struct VariableHighlightElement {
    pub(in crate::components) state: Entity<InputState>,
    pub(in crate::components) base_color: Hsla,
    pub(in crate::components) highlight_color: Hsla,
    pub(in crate::components) highlight_path_variables: bool,
}

pub(in crate::components) struct VariableHighlightPrepaintState {
    line: Option<ShapedLine>,
    pub(in crate::components) scroll_offset: Pixels,
    cursor_x: Pixels,
}

impl IntoElement for VariableHighlightElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for VariableHighlightElement {
    type RequestLayoutState = ();
    type PrepaintState = VariableHighlightPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.state.clone();
        let value = single_line(state.read(cx).value());
        let ranges = input_variable_ranges(&value, self.highlight_path_variables)
            .into_iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        let cursor = state.read(cx).cursor();
        let style = window.text_style();
        let run = TextRun {
            len: value.len(),
            font: style.font(),
            color: self.base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = variable_highlight_runs(&value, &ranges, &run, self.highlight_color);
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(value, font_size, &runs, None);
        let cursor_x = line.x_for_index(cursor);
        VariableHighlightPrepaintState {
            line: Some(line),
            scroll_offset: px(0.0),
            cursor_x,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Input is the previous sibling and paints first. gpui-base commits
        // its finalized cursor-follow scroll offset during that paint, so read
        // it here instead of trying to predict it during prepaint. The fallback
        // keeps isolated element tests meaningful before an Input has laid out.
        let visible_width = bounds.right() - bounds.left();
        let current_scroll_offset = {
            let input = self.state.read(cx);
            input.scroll_offset().x
        };
        let line_width = prepaint
            .line
            .as_ref()
            .map(|line| line.width)
            .unwrap_or(px(0.0));
        let scroll_offset = input_text_scroll_offset(
            prepaint.cursor_x,
            line_width,
            visible_width,
            current_scroll_offset,
        );
        prepaint.scroll_offset = scroll_offset;
        let line = prepaint.line.take();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(line) = line {
                line.paint(
                    bounds.origin + point(scroll_offset, px(0.0)),
                    window.line_height(),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .expect("variable highlight text should paint");
            }
        });
    }
}

/// Matches Longbridge gpui-base single-line input: shift the painted line left
/// when the caret would otherwise sit past the visible width.
pub(in crate::components) fn variable_highlight_runs(
    value: &str,
    ranges: &[Range<usize>],
    base: &TextRun,
    highlight_color: Hsla,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut ix = 0;
    for range in ranges {
        let start = range.start.min(value.len());
        let end = range.end.min(value.len());
        if ix < start {
            runs.push(TextRun {
                len: start - ix,
                color: base.color,
                ..base.clone()
            });
        }
        if end > start {
            runs.push(TextRun {
                len: end - start,
                color: highlight_color,
                ..base.clone()
            });
        }
        ix = ix.max(end);
    }
    if ix < value.len() {
        runs.push(TextRun {
            len: value.len() - ix,
            color: base.color,
            ..base.clone()
        });
    }
    runs.retain(|run| run.len > 0);
    if runs.is_empty() {
        runs.push(base.clone());
    }
    runs
}

/// Horizontal scroll for single-line input highlights.
///
/// Matches gpui-base `InputBaseState::scroll_to` for left-aligned input without
/// line numbers (`RIGHT_MARGIN` = 10px). The overlay must reuse the input's
/// current scroll offset; recomputing from zero desyncs highlights from text
/// once the caret moves while the field is already scrolled.
pub(in crate::components) fn input_text_scroll_offset(
    cursor_x: Pixels,
    line_width: Pixels,
    visible_width: Pixels,
    current_scroll_x: Pixels,
) -> Pixels {
    const RIGHT_MARGIN: Pixels = px(10.0);
    let mut scroll_x = current_scroll_x;
    if cursor_x - RIGHT_MARGIN < -scroll_x {
        scroll_x = -cursor_x + RIGHT_MARGIN;
    } else if cursor_x + RIGHT_MARGIN > -scroll_x + visible_width {
        scroll_x = -(cursor_x - visible_width + RIGHT_MARGIN);
    }
    // gpui-base clamps the offset after shaping. This matters when deleting
    // from the end of a scrolled value: the valid left edge moves right on
    // every keystroke, before the updated offset is observable from state.
    let scroll_width = if line_width + RIGHT_MARGIN > visible_width {
        line_width + RIGHT_MARGIN
    } else {
        line_width
    };
    let minimum_scroll_x = (-scroll_width + visible_width).min(px(0.0));
    scroll_x.clamp(minimum_scroll_x, px(0.0))
}

pub(crate) fn single_line(value: impl Into<SharedString>) -> SharedString {
    let value = value.into();
    if value.find(['\n', '\r']).is_none() {
        value
    } else {
        SharedString::from(value.replace(['\n', '\r'], " "))
    }
}

pub(in crate::components) fn variable_tooltip_presentation(
    name: &str,
    variables: &VariableContext,
) -> VariableTooltipPresentation {
    if variables.secrets.contains(name) {
        return unavailable_variable_tooltip(&variables.unavailable_message);
    }
    if let Some(value) = variables.values.get(name) {
        return VariableTooltipPresentation {
            value: value.clone(),
            placeholder: "Variable value",
            editable: variables.on_change.is_some(),
            hint: None,
        };
    }
    if variables.on_change.is_some() {
        return VariableTooltipPresentation {
            value: String::new(),
            placeholder: "Enter a value to create",
            editable: true,
            hint: Some("Not defined in this environment"),
        };
    }
    unavailable_variable_tooltip(&variables.unavailable_message)
}

fn unavailable_variable_tooltip(message: &str) -> VariableTooltipPresentation {
    VariableTooltipPresentation {
        value: message.to_owned(),
        placeholder: "Variable value",
        editable: false,
        hint: None,
    }
}

pub(in crate::components) struct VariableTooltipPresentation {
    pub(in crate::components) value: String,
    pub(in crate::components) placeholder: &'static str,
    pub(in crate::components) editable: bool,
    pub(in crate::components) hint: Option<&'static str>,
}

pub(in crate::components) fn variable_ranges(value: &str) -> Vec<(Range<usize>, String)> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    let mut remaining = value;
    while let Some(start) = remaining.find("{{") {
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let name = after_start[..end].trim();
        if !name.is_empty() && !name.contains("{{") {
            let range_start = offset + start;
            let range_end = range_start + 2 + end + 2;
            ranges.push((range_start..range_end, name.to_owned()));
        }
        let consumed = start + 2 + end + 2;
        offset += consumed;
        remaining = &remaining[consumed..];
    }
    ranges
}

pub(in crate::components) fn input_variable_ranges(
    value: &str,
    highlight_path_variables: bool,
) -> Vec<(Range<usize>, String)> {
    let mut ranges = variable_ranges(value);
    if highlight_path_variables {
        ranges.extend(path_variable_ranges(value));
        ranges.sort_by_key(|(range, _)| range.start);
    }
    ranges
}
