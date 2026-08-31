use super::*;

impl ProbeApp {
    pub(super) fn render_tabs(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let tab_count = self.shell.tabs().len();
        let mut active_tab_background: Hsla = theme.colors.actions.accent.into();
        active_tab_background.a = 0.12;
        let mut active_tab_close_hover: Hsla = theme.colors.actions.accent.into();
        active_tab_close_hover.a = 0.18;
        let request_tab_bar_height = theme.metrics.tab_bar_height + 2.0;
        let mut tab_strip = Tabs::new("request-tabs-scroll")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px(px(theme.metrics.spacing_1))
            .flex()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&self.tab_bar_scroll);
        let Some(loaded) = &self.loaded_workspace else {
            return div()
                .id("request-tabs")
                .h(px(request_tab_bar_height))
                .w_full()
                .bg(theme.colors.surfaces.raised)
                .border_b_1()
                .border_color(theme.colors.borders.subtle);
        };
        for key in self.shell.tabs() {
            let Some(request) = loaded.workspace().request(*key) else {
                continue;
            };
            let active = self.shell.active_tab() == Some(*key);
            let dirty = self.persistence.is_dirty(*key, request);
            let label = request
                .metadata
                .name
                .as_deref()
                .unwrap_or("Untitled request");
            let select_view = cx.weak_entity();
            let close_view = cx.weak_entity();
            let context_menu_view = cx.weak_entity();
            let tooltip_hover_view = cx.weak_entity();
            let tooltip_move_view = cx.weak_entity();
            let tooltip_leave_view = cx.weak_entity();
            let middle_close_view = close_view.clone();
            let close_hover = if active {
                active_tab_close_hover
            } else {
                theme.colors.actions.disabled.into()
            };
            let tab_key = *key;
            let tab_index = self
                .shell
                .tabs()
                .iter()
                .position(|open| *open == *key)
                .unwrap_or(0);
            tab_strip = tab_strip.child(
                Tab::new(("request-tab", key.slot()))
                    .selected(active)
                    .set_position(tab_index + 1, tab_count)
                    .h(px(request_tab_bar_height))
                    .min_w(px(80.0))
                    .max_w(px(176.0))
                    .pl(px(theme.metrics.spacing_3))
                    .pr(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_1))
                    .overflow_hidden()
                    .rounded_tl(px(theme.metrics.radius_medium))
                    .rounded_tr(px(theme.metrics.radius_medium))
                    .when(active, |tab| {
                        tab.bg(active_tab_background)
                            .border_b_1()
                            .border_color(theme.colors.actions.accent)
                            .text_color(theme.colors.actions.accent)
                    })
                    .when(!active, |tab| {
                        tab.text_color(theme.colors.text.secondary)
                            .hover(move |tab| tab.bg(theme.colors.surfaces.sidebar))
                    })
                    .cursor_pointer()
                    .on_mouse_move(move |event, _, cx| {
                        let _ = tooltip_move_view.update(cx, |view, cx| {
                            view.update_request_tab_tooltip_position(tab_key, event.position, cx);
                        });
                    })
                    .on_hover(move |hovered, window, cx| {
                        let _ = if *hovered {
                            tooltip_hover_view.update(cx, |view, cx| {
                                view.open_request_tab_tooltip(tab_key, window.mouse_position(), cx);
                            })
                        } else {
                            tooltip_leave_view.update(cx, |view, cx| {
                                view.close_request_tab_tooltip(tab_key, cx);
                            })
                        };
                    })
                    .on_click(move |_, _, cx| {
                        let _ = select_view.update(cx, |view, cx| view.select_request(tab_key, cx));
                    })
                    .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let _ = context_menu_view.update(cx, |view, cx| {
                            view.open_tab_context_menu(tab_key, event.position, cx);
                        });
                    })
                    .on_aux_click(move |event, window, cx| {
                        if event.is_middle_click() {
                            cx.stop_propagation();
                            let _ = middle_close_view
                                .update(cx, |view, cx| view.request_close_tab(tab_key, window, cx));
                        }
                    })
                    .child(
                        components::truncated_label(label.to_owned())
                            .flex_1()
                            .when(active, |label| {
                                label.debug_selector(|| "request-tab-label".into())
                            }),
                    )
                    .when(dirty, |tab| {
                        tab.child(
                            div()
                                .id(("request-dirty", key.slot()))
                                .flex_none()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(theme.colors.actions.accent),
                        )
                    })
                    .child(
                        Button::new(("close-tab", key.slot()))
                            .focusable(false)
                            .tab_stop(false)
                            .flex_none()
                            .w(px(theme.metrics.icon_standard + 4.0))
                            .h(px(theme.metrics.icon_standard + 4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(theme.metrics.radius_small))
                            .hover(move |close| close.bg(close_hover))
                            .child(components::close_icon(theme))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                let _ = close_view.update(cx, |view, cx| {
                                    view.request_close_tab(tab_key, window, cx)
                                });
                            }),
                    ),
            );
        }

        let mut tabs = div()
            .id("request-tabs")
            .h(px(request_tab_bar_height))
            .w_full()
            .flex()
            .items_center()
            .bg(theme.colors.surfaces.raised)
            .border_b_1()
            .border_color(theme.colors.borders.subtle)
            .child(tab_strip);
        let selected = self.shell.selected_environment().unwrap_or("").to_owned();
        let mut options = vec![(String::new(), "No environment".to_owned())];
        options.extend(
            loaded
                .workspace()
                .environments()
                .iter()
                .map(|environment| (environment.name.clone(), environment.name.clone())),
        );
        let environment_view = cx.weak_entity();
        let create_environment_view = cx.weak_entity();
        let manage_environment_view = cx.weak_entity();
        tabs = tabs.child(
            div().flex_none().px(px(theme.metrics.spacing_2)).child(
                components::dropdown(
                    theme,
                    "request-environment",
                    "Request environment",
                    Some(selected),
                    options,
                    190.0,
                    move |value, _, cx| {
                        let value = value.cloned().unwrap_or_default();
                        let _ = environment_view.update(cx, |view, cx| {
                            view.select_environment((!value.is_empty()).then_some(value), cx);
                        });
                    },
                )
                .with_action("Create environment…", move |window, cx| {
                    let _ = create_environment_view.update(cx, |view, cx| {
                        view.open_create_environment_dialog(window, cx);
                    });
                })
                .with_action("Manage environments…", move |window, cx| {
                    let _ = manage_environment_view.update(cx, |view, cx| {
                        view.open_environment_manager_dialog(window, cx);
                    });
                }),
            ),
        );
        tabs
    }

    pub(in crate::app) fn edit_request(
        &mut self,
        key: RequestKey,
        edit: impl FnOnce(&mut HttpRequest),
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        edit(request);
        self.persistence.edited(key);
        cx.notify();
    }

    pub(in crate::app) fn change_body_kind(
        &mut self,
        key: RequestKey,
        kind: BodyEditorKind,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        self.request_editor.switch_body_kind(key, request, kind);
        self.persistence.edited(key);
        cx.notify();
    }
}
