use super::*;

mod dialogs;
mod environments;
mod menus;

pub(super) fn environment_variable_text(variable: &Variable) -> (String, bool) {
    match variable.value.as_ref() {
        Some(VariableValueSet::Single(VariableValue::String(value))) => (value.clone(), true),
        Some(VariableValueSet::Single(VariableValue::Typed { data, .. })) => (data.clone(), true),
        Some(VariableValueSet::Variants(variants)) => (
            variants
                .iter()
                .find(|variant| variant.selected)
                .map(|variant| match &variant.value {
                    VariableValue::String(value) => value.clone(),
                    VariableValue::Typed { data, .. } => data.clone(),
                })
                .unwrap_or_default(),
            false,
        ),
        None => (String::new(), true),
    }
}

pub(super) fn set_environment_variable_text(variable: &mut Variable, value: String) {
    match variable.value.as_mut() {
        Some(VariableValueSet::Single(VariableValue::String(current))) => *current = value,
        Some(VariableValueSet::Single(VariableValue::Typed { data, .. })) => *data = value,
        Some(VariableValueSet::Variants(_)) => {}
        None => {
            variable.value = Some(VariableValueSet::Single(VariableValue::String(value)));
        }
    }
}

pub(super) fn environment_variant_value(
    theme: Theme,
    name: &str,
    row_index: usize,
    value: String,
    inherited: bool,
) -> gpui::AnyElement {
    let selector = if name.is_empty() {
        format!("environment-variable-variant-{row_index}")
    } else {
        format!("environment-variable-variant-{name}")
    };
    div()
        .id(selector.clone())
        .debug_selector(move || selector.clone())
        .flex_1()
        .min_w(px(120.0))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.0))
        .child(
            components::truncated_label(value)
                .font_family(theme.typography.monospace_family)
                .when(inherited, |label| label.text_color(theme.colors.text.muted)),
        )
        .child(
            div()
                .text_size(px(theme.typography.caption_size))
                .text_color(theme.colors.text.muted)
                .child("Multiple values"),
        )
        .into_any_element()
}
