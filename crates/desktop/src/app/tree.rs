use gpui::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_base::Button;
use probe_core::{Workspace, WorkspaceItemRef};
use probe_opencollection::ItemKind;

use crate::{components, shell::ShellState, theme::Theme, tree_search::TreeSearchMatches};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TreeRow {
    pub(crate) item: WorkspaceItemRef,
    pub(crate) depth: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct TreeDrag {
    pub(crate) item: WorkspaceItemRef,
    pub(crate) kind: ItemKind,
    pub(crate) label: String,
    pub(crate) method: Option<String>,
}

pub(crate) struct TreeRowSpec {
    pub(crate) item: WorkspaceItemRef,
    pub(crate) kind: ItemKind,
    pub(crate) selector: String,
    pub(crate) label: String,
    pub(crate) method: Option<String>,
    pub(crate) depth: usize,
    pub(crate) selected: bool,
}

impl Render for TreeDrag {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_window_appearance(window.appearance());
        let mut preview = div()
            .px(px(theme.metrics.spacing_2))
            .py(px(theme.metrics.spacing_1))
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_small))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .text_size(px(theme.typography.caption_size));
        if let Some(method) = &self.method {
            preview = preview.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.method_color(method))
                    .child(method.clone()),
            );
        } else if self.kind == ItemKind::Folder {
            preview = preview.child(components::tree_folder_icon(theme, false, false));
        }
        preview.child(self.label.clone())
    }
}

pub(crate) fn tree_row_button(
    theme: Theme,
    id: impl Into<gpui::ElementId>,
    depth: usize,
    selected: bool,
) -> Button {
    Button::new(id)
        .focusable(true)
        .tab_stop(true)
        .key_context("RequestTree")
        .w_full()
        .h(px(theme.metrics.tree_row_height))
        .pl(px(tree_level_indent(theme, depth)))
        .pr(px(theme.metrics.spacing_1))
        .flex()
        .items_center()
        .gap(px(theme.metrics.spacing_1))
        .overflow_hidden()
        .rounded(px(theme.metrics.radius_small))
        .when(selected, |row| {
            row.bg(theme.colors.selection.active_background)
                .text_color(theme.colors.selection.active_foreground)
        })
        .when(!selected, |row| {
            row.hover(move |row| row.bg(theme.colors.surfaces.window))
        })
        .cursor_pointer()
}

pub(crate) fn tree_level_indent(theme: Theme, depth: usize) -> f32 {
    theme.metrics.spacing_2 + depth as f32 * theme.metrics.icon_standard
}

pub(crate) fn tree_method_font_size(theme: Theme, method: &str) -> f32 {
    if method.len() > 3 {
        theme.typography.caption_size - 2.0
    } else {
        theme.typography.caption_size - 1.0
    }
}

pub(crate) fn tree_method_label(method: &str) -> &str {
    match method {
        "DELETE" => "DEL",
        "OPTION" | "OPTIONS" => "OPT",
        "PATCH" => "PAT",
        "CONNECT" => "CON",
        method if method.len() <= 4 => method,
        _ => "HTTP",
    }
}

pub(crate) fn tree_hierarchy_guides(theme: Theme, depth: usize, selected: bool) -> gpui::Div {
    let mut guides = div().absolute().top(px(0.0)).bottom(px(0.0)).left(px(0.0));
    let color = if selected {
        theme.colors.selection.active_foreground.opacity(0.22)
    } else {
        theme.colors.borders.standard
    };
    for level in 0..depth {
        guides = guides.child(
            div()
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .left(px(
                    tree_level_indent(theme, level) + theme.metrics.icon_standard / 2.0
                ))
                .w(px(1.0))
                .bg(color),
        );
    }
    guides
}

pub(crate) fn flatten_visible_tree_rows(
    workspace: &Workspace,
    items: &[WorkspaceItemRef],
    depth: usize,
    shell: &ShellState,
    filter: Option<&TreeSearchMatches>,
    rows: &mut Vec<TreeRow>,
) {
    for item in items {
        if filter.is_some_and(|hits| !hits.contains(*item)) {
            continue;
        }
        rows.push(TreeRow { item: *item, depth });
        if let WorkspaceItemRef::Folder(key) = item
            && shell.folder_is_expanded(*key)
            && let Some(folder) = workspace.folder(*key)
        {
            flatten_visible_tree_rows(workspace, &folder.children, depth + 1, shell, filter, rows);
        }
    }
}
