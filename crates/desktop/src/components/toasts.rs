use super::*;
use crate::toast::{TOAST_MOTION_DURATION, ToastId, ToastIntent, ToastMessage};
use gpui_base::{Toast, ToastTransitionStatus};

pub(crate) const TOAST_STACK_WIDTH: f32 = 360.0;
const TOAST_ACCENT_STRIP_WIDTH: f32 = 4.0;
const TOAST_BORDER_WIDTH: f32 = 1.0;

fn intent_color(theme: Theme, intent: ToastIntent) -> gpui::Rgba {
    match intent {
        ToastIntent::Success => theme.colors.status.success,
        ToastIntent::Info => theme.colors.status.informational,
        ToastIntent::Warning => theme.colors.status.warning,
        ToastIntent::Error => theme.colors.status.error,
    }
}

fn accent_strip(color: gpui::Rgba, radius: f32) -> gpui::Div {
    // GPUI clips overflow to a rectangle, so a 4px child would poke out of the
    // toast's rounded corners. Paint a matching rounded fill and clip it to the
    // accent column so it stays inside the card.
    div()
        .relative()
        .w(px(TOAST_ACCENT_STRIP_WIDTH))
        .flex_none()
        .self_stretch()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left(px(-TOAST_BORDER_WIDTH))
                .top(px(-TOAST_BORDER_WIDTH))
                .bottom(px(-TOAST_BORDER_WIDTH))
                .w(px(radius * 2.0))
                .rounded_tl(px(radius))
                .rounded_bl(px(radius))
                .bg(color),
        )
}

pub(crate) fn toast(
    theme: Theme,
    id: ToastId,
    notification: &ToastMessage,
    status: ToastTransitionStatus,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let intent = notification.intent;
    let intent_label = intent.label();
    let color = intent_color(theme, intent);
    let close_id = ElementId::Name(format!("toast-close-{id}").into());
    let toast_id = ElementId::Name(format!("toast-{id}").into());
    let radius = theme.metrics.radius_medium;

    let toast = Toast::new(toast_id)
        .transition_status(status)
        .debug_selector(move || format!("toast-{id}"))
        .aria_label(format!("{intent_label}: {}", notification.message))
        .occlude()
        .relative()
        .w_full()
        .min_h(px(theme.metrics.control_height + theme.metrics.spacing_3))
        .overflow_hidden()
        .rounded(px(radius))
        .bg(theme.colors.surfaces.overlay)
        .border_1()
        .border_color(theme.colors.borders.standard)
        .shadow(super::temporary_surface_shadow(theme, 4.0))
        .flex()
        .child(accent_strip(color, radius))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .items_start()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .px(px(theme.metrics.spacing_3))
                        .py(px(theme.metrics.spacing_2))
                        .flex()
                        .flex_col()
                        .gap(px(theme.metrics.spacing_1))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color)
                                .child(intent_label),
                        )
                        .child(
                            div()
                                .text_color(theme.colors.text.primary)
                                .child(notification.message.clone()),
                        ),
                )
                .child(
                    Button::new(close_id)
                        .debug_selector(move || format!("toast-close-{id}"))
                        .accessibility_label(format!("Dismiss {intent_label} notification"))
                        .flex_none()
                        .mt(px(theme.metrics.spacing_1))
                        .mr(px(theme.metrics.spacing_1))
                        .w(px(theme.metrics.control_height))
                        .h(px(theme.metrics.control_height))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(theme.metrics.radius_small))
                        .text_color(theme.colors.text.secondary)
                        .hover(move |button| button.bg(theme.colors.selection.inactive_background))
                        .focus(move |button| {
                            button.shadow(super::buttons::focus_ring_shadow(
                                theme.colors.borders.focused.into(),
                                theme.colors.surfaces.overlay.into(),
                            ))
                        })
                        .on_click(on_dismiss)
                        .child(close_icon(theme).text_color(theme.colors.text.secondary)),
                ),
        );

    match status {
        ToastTransitionStatus::Starting => toast
            .with_animation(
                ("toast-enter", id),
                Animation::new(TOAST_MOTION_DURATION),
                |toast, progress| {
                    toast
                        .left(px(48.0 * (1.0 - progress)))
                        .opacity(0.2 + 0.8 * progress)
                },
            )
            .into_any_element(),
        ToastTransitionStatus::Ending => toast
            .with_animation(
                ("toast-exit", id),
                Animation::new(TOAST_MOTION_DURATION),
                |toast, progress| toast.opacity(1.0 - progress),
            )
            .into_any_element(),
        ToastTransitionStatus::Present => toast.into_any_element(),
    }
}
