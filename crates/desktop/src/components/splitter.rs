use super::*;
use gpui::{Axis, CursorStyle, MouseDownEvent};

const LINE_SIZE: f32 = 1.0;
const HIT_TARGET_SIZE: f32 = 5.0;
const HIT_INSET: f32 = HIT_TARGET_SIZE / 2.0;
const LINE_INSET: f32 = (HIT_TARGET_SIZE - LINE_SIZE) / 2.0;

type MouseDownHandler = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

/// 5px overlay resize handle for a `relative` parent.
///
/// Attach the handle to the later pane. The 5px hit target is centered on the
/// shared edge, so 2.5px hangs into the previous pane and 2.5px stays on this
/// pane. The idle 1px line sits in the middle of that band. Hover fills the
/// full band. [`Self::show_line`] controls the idle stroke.
#[derive(IntoElement)]
pub(crate) struct PaneSplitter {
    theme: Theme,
    id: ElementId,
    axis: Axis,
    show_line: bool,
    trailing: bool,
    on_mouse_down: Option<MouseDownHandler>,
    debug_selector: Option<String>,
}

pub(crate) fn pane_splitter(theme: Theme, id: impl Into<ElementId>, axis: Axis) -> PaneSplitter {
    PaneSplitter {
        theme,
        id: id.into(),
        axis,
        show_line: true,
        trailing: false,
        on_mouse_down: None,
        debug_selector: None,
    }
}

impl PaneSplitter {
    /// When `false`, the 1px line stays hidden until hover, which then shows
    /// only the hit target. The default is `true`.
    pub(crate) fn show_line(mut self, show_line: bool) -> Self {
        self.show_line = show_line;
        self
    }

    /// Pin the handle to the trailing edge (right for a horizontal axis, bottom
    /// for a vertical axis). The default is the leading edge.
    #[allow(dead_code)]
    pub(crate) fn trailing(mut self) -> Self {
        self.trailing = true;
        self
    }

    pub(crate) fn debug_selector(mut self, selector: impl Into<String>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    pub(crate) fn on_mouse_down(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_mouse_down = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for PaneSplitter {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let along_x = self.axis == Axis::Horizontal;
        let line_color = self.theme.colors.borders.subtle;
        let debug_selector = self.debug_selector;
        let trailing = self.trailing;

        let mut handle = div()
            .id(self.id)
            .absolute()
            .occlude()
            .cursor(if along_x {
                CursorStyle::ResizeLeftRight
            } else {
                CursorStyle::ResizeUpDown
            })
            .hover(move |handle| handle.bg(line_color))
            .when_some(self.on_mouse_down, |handle, handler| {
                handle.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    handler(event, window, cx);
                })
            })
            .when_some(debug_selector, |handle, selector| {
                handle.debug_selector(move || selector.clone())
            });
        if along_x {
            handle = handle.w(px(HIT_TARGET_SIZE)).top(px(0.0)).bottom(px(0.0));
            handle = if trailing {
                handle.right(px(-HIT_INSET))
            } else {
                handle.left(px(-HIT_INSET))
            };
        } else {
            handle = handle.h(px(HIT_TARGET_SIZE)).left(px(0.0)).right(px(0.0));
            handle = if trailing {
                handle.bottom(px(-HIT_INSET))
            } else {
                handle.top(px(-HIT_INSET))
            };
        }

        handle.when(self.show_line, |handle| {
            handle.child(
                div()
                    .absolute()
                    .bg(line_color)
                    .when(along_x, |line| {
                        line.w(px(LINE_SIZE))
                            .top(px(0.0))
                            .bottom(px(0.0))
                            .left(px(LINE_INSET))
                    })
                    .when(!along_x, |line| {
                        line.h(px(LINE_SIZE))
                            .left(px(0.0))
                            .right(px(0.0))
                            .top(px(LINE_INSET))
                    }),
            )
        })
    }
}
