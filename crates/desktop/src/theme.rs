//! Semantic desktop design tokens and complete built-in themes.
//!
//! Built-in appearances map Porcelain Honey (light, based on Probe's app icon)
//! and Graphite Honey (dark) onto this model. Components consume roles rather
//! than embedding color literals. Future user-authored themes can produce the
//! same model without changing components.

use std::borrow::Cow;

use gpui::{FontWeight, Hsla, Rgba, WindowAppearance, hsla, px, rgba};
use gpui_base::{
    ColorTokens, RadiusTokens, ScrollbarStyles, SemanticThemeTokens, ShadowTokens, SpacingTokens,
    TextStyleToken, TypographyTokens,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeAppearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub appearance: ThemeAppearance,
    pub colors: Colors,
    pub typography: Typography,
    pub metrics: Metrics,
    pub motion: Motion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colors {
    pub surfaces: SurfaceColors,
    pub text: TextColors,
    pub borders: BorderColors,
    pub actions: ActionColors,
    pub selection: SelectionColors,
    pub status: StatusColors,
    pub methods: MethodColors,
    pub responses: ResponseColors,
    pub syntax: SyntaxColors,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceColors {
    pub window: Rgba,
    pub sidebar: Rgba,
    pub editor: Rgba,
    pub raised: Rgba,
    pub overlay: Rgba,
    /// Semi-transparent dimming layer for modal overlays.
    pub scrim: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextColors {
    pub primary: Rgba,
    pub secondary: Rgba,
    pub muted: Rgba,
    pub placeholder: Rgba,
    pub inverse: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderColors {
    pub subtle: Rgba,
    pub standard: Rgba,
    pub strong: Rgba,
    pub focused: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionColors {
    pub accent: Rgba,
    pub hover: Rgba,
    pub pressed: Rgba,
    pub disabled: Rgba,
    pub disabled_foreground: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionColors {
    pub active_background: Rgba,
    pub active_foreground: Rgba,
    pub inactive_background: Rgba,
    pub inactive_foreground: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusColors {
    pub success: Rgba,
    pub warning: Rgba,
    pub error: Rgba,
    pub informational: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MethodColors {
    pub get: Rgba,
    pub post: Rgba,
    pub put: Rgba,
    pub patch: Rgba,
    pub delete: Rgba,
    pub other: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponseColors {
    pub informational: Rgba,
    pub success: Rgba,
    pub redirect: Rgba,
    pub client_error: Rgba,
    pub server_error: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxColors {
    pub plain: Rgba,
    pub property: Rgba,
    pub string: Rgba,
    pub number: Rgba,
    pub boolean: Rgba,
    pub null: Rgba,
    pub punctuation: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub interface_family: &'static str,
    pub monospace_family: &'static str,
    pub body_size: f32,
    pub caption_size: f32,
    pub title_size: f32,
    pub dialog_title_size: f32,
    pub body_line_height: f32,
    pub regular_weight: u16,
    pub medium_weight: u16,
    pub semibold_weight: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub spacing_1: f32,
    pub spacing_2: f32,
    pub spacing_3: f32,
    pub spacing_4: f32,
    pub radius_small: f32,
    pub radius_medium: f32,
    pub radius_large: f32,
    pub control_height: f32,
    pub tree_row_height: f32,
    pub tab_bar_height: f32,
    pub icon_small: f32,
    pub icon_standard: f32,
    pub elevation_raised: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub fast_ms: u16,
    pub standard_ms: u16,
    pub slow_ms: u16,
    pub easing: &'static str,
    pub reduced_duration_ms: u16,
}

impl Theme {
    #[must_use]
    pub fn for_window_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::dark(),
        }
    }

    /// Install gpui-base infrastructure and the library's default semantic tokens.
    pub fn init(cx: &mut gpui::App) {
        gpui_base::init(cx);
        load_bundled_fonts(cx);
        Self::sync_gpui_base(WindowAppearance::Light, cx);
    }

    /// Project this theme into gpui-base's global `Theme` so primitives and
    /// infrastructure (scrollbars, resize handles) share the same tokens.
    pub fn sync_gpui_base(appearance: WindowAppearance, cx: &mut gpui::App) {
        let theme = Self::for_window_appearance(appearance);
        let gpui_theme = gpui_base::Theme::global_mut(cx);
        gpui_theme.tokens = theme.semantic_tokens();
        let muted: Hsla = theme.colors.text.muted.into();
        let border: Hsla = theme.colors.borders.standard.into();
        let primary: Hsla = theme.colors.text.primary.into();
        let thumb_radius = px(theme.metrics.radius_small);
        gpui_theme.scrollbar.styles = ScrollbarStyles::default()
            .thumb(|thumb| thumb.bg(muted).radius(thumb_radius))
            .thumb_hover(|thumb| thumb.bg(border).radius(thumb_radius))
            .thumb_active(|thumb| thumb.bg(primary).radius(thumb_radius));
        gpui_theme.resizable.handle = theme.colors.borders.subtle.into();
        gpui_theme.resizable.active_handle = theme.colors.borders.focused.into();
    }

    /// Editor caret selection wash derived from the active accent.
    #[must_use]
    pub fn editor_selection(self) -> Hsla {
        match self.appearance {
            ThemeAppearance::Light => hsla(0.092, 0.767, 0.500, 0.25),
            ThemeAppearance::Dark => hsla(0.098, 0.858, 0.580, 0.24),
        }
    }

    /// Elevation shadow color for temporary raised surfaces.
    #[must_use]
    pub fn elevation_shadow(self, alpha: f32) -> Hsla {
        let mut shadow = match self.appearance {
            ThemeAppearance::Light => self.colors.text.primary.into(),
            ThemeAppearance::Dark => hsla(0.0, 0.0, 0.0, 1.0),
        };
        shadow.a = alpha;
        shadow
    }

    fn semantic_tokens(self) -> SemanticThemeTokens {
        let shadow_alpha = match self.appearance {
            ThemeAppearance::Light => 0.10,
            ThemeAppearance::Dark => 0.32,
        };
        let shadow = self.elevation_shadow(shadow_alpha);
        let body_line = px(self.typography.body_size * self.typography.body_line_height);
        SemanticThemeTokens {
            colors: ColorTokens {
                background: self.colors.surfaces.window.into(),
                foreground: self.colors.text.primary.into(),
                surface: self.colors.surfaces.raised.into(),
                surface_foreground: self.colors.text.primary.into(),
                primary: self.colors.actions.accent.into(),
                primary_foreground: self.colors.text.inverse.into(),
                secondary: self.colors.surfaces.raised.into(),
                secondary_foreground: self.colors.text.secondary.into(),
                muted: self.colors.surfaces.sidebar.into(),
                muted_foreground: self.colors.text.muted.into(),
                accent: self.colors.selection.inactive_background.into(),
                accent_foreground: self.colors.text.primary.into(),
                destructive: self.colors.status.error.into(),
                destructive_foreground: self.colors.text.inverse.into(),
                border: self.colors.borders.standard.into(),
                input: self.colors.borders.standard.into(),
                ring: self.colors.borders.focused.into(),
            },
            radius: RadiusTokens {
                none: px(0.0),
                sm: px(self.metrics.radius_small),
                md: px(self.metrics.radius_medium),
                lg: px(self.metrics.radius_large),
                xl: px(self.metrics.radius_large),
                full: px(9999.0),
            },
            spacing: SpacingTokens {
                xxs: px(2.0),
                xs: px(self.metrics.spacing_1),
                sm: px(self.metrics.spacing_2),
                md: px(self.metrics.spacing_3),
                lg: px(self.metrics.spacing_4),
                xl: px(20.0),
                xxl: px(24.0),
            },
            typography: TypographyTokens {
                sans: self.typography.interface_family.into(),
                mono: self.typography.monospace_family.into(),
                xs: TextStyleToken {
                    size: px(self.typography.caption_size),
                    line_height: px(16.0),
                    weight: FontWeight::NORMAL,
                },
                sm: TextStyleToken {
                    size: px(self.typography.body_size),
                    line_height: body_line,
                    weight: FontWeight::NORMAL,
                },
                md: TextStyleToken {
                    size: px(self.typography.body_size),
                    line_height: body_line,
                    weight: FontWeight::NORMAL,
                },
                lg: TextStyleToken {
                    size: px(15.0),
                    line_height: px(22.0),
                    weight: FontWeight::NORMAL,
                },
                xl: TextStyleToken {
                    size: px(self.typography.title_size),
                    line_height: px(32.0),
                    weight: FontWeight::SEMIBOLD,
                },
                mono_md: TextStyleToken {
                    size: px(self.typography.body_size),
                    line_height: px(20.0),
                    weight: FontWeight::NORMAL,
                },
            },
            shadow: ShadowTokens::elevations(shadow),
        }
    }

    #[must_use]
    pub fn method_color(self, method: &str) -> Rgba {
        match method.to_ascii_uppercase().as_str() {
            "GET" => self.colors.methods.get,
            "POST" => self.colors.methods.post,
            "PUT" => self.colors.methods.put,
            "PATCH" => self.colors.methods.patch,
            "DELETE" => self.colors.methods.delete,
            _ => self.colors.methods.other,
        }
    }

    #[must_use]
    pub fn light() -> Self {
        Self {
            appearance: ThemeAppearance::Light,
            colors: porcelain_honey(),
            typography: platform_typography(),
            metrics: default_metrics(),
            motion: default_motion(),
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            appearance: ThemeAppearance::Dark,
            colors: graphite_honey(),
            typography: platform_typography(),
            metrics: default_metrics(),
            motion: default_motion(),
        }
    }
}

/// Porcelain Honey — Probe's light theme based on the creamy app icon background.
fn porcelain_honey() -> Colors {
    Colors {
        surfaces: SurfaceColors {
            window: rgba(0xf8f6f2ff),  // warm porcelain
            sidebar: rgba(0xf1eee9ff), // soft stone navigation
            editor: rgba(0xfaf8f5ff),  // clean ivory editor
            raised: rgba(0xfcf9f5ff),  // lifted paper
            overlay: rgba(0xfdfbf8ff), // warm overlay
            scrim: rgba(0x0000003d),   // black @ ~24%
        },
        text: TextColors {
            primary: rgba(0x4b4641ff),     // warm graphite
            secondary: rgba(0x6c655eff),   // soft graphite
            muted: rgba(0x8b837aff),       // softened graphite
            placeholder: rgba(0xa9a199ff), // stone
            inverse: rgba(0xffffffff),     // on-accent
        },
        borders: BorderColors {
            subtle: rgba(0xe7e1d9ff),   // porcelain edge
            standard: rgba(0xdcd4cbff), // light stone divider
            strong: rgba(0xc5b9adff),   // grounded divider
            focused: rgba(0xe7821b99),  // orange focus
        },
        actions: ActionColors {
            accent: rgba(0xe7821bff),              // reference orange
            hover: rgba(0xd96f10ff),               // burnished orange
            pressed: rgba(0xb9570cff),             // deep orange
            disabled: rgba(0xe7e1d9ff),            // porcelain edge
            disabled_foreground: rgba(0xa9a199ff), // stone
        },
        selection: SelectionColors {
            active_background: rgba(0xe7821bff),   // filled reference orange
            active_foreground: rgba(0xffffffff),   // on-accent
            inactive_background: rgba(0xebe5deff), // parchment wash
            inactive_foreground: rgba(0x4b4641ff), // warm graphite
        },
        status: StatusColors {
            success: rgba(0x2d8a5bff),       // jade
            warning: rgba(0xe7821bff),       // reference orange
            error: rgba(0xc43d3dff),         // red
            informational: rgba(0x227c8fff), // blue teal
        },
        methods: MethodColors {
            get: rgba(0x1f8a70ff),    // teal
            post: rgba(0xe7821bff),   // reference orange
            put: rgba(0xaa7d24ff),    // light brown
            patch: rgba(0x7c5bbdff),  // violet
            delete: rgba(0xc43d3dff), // red
            other: rgba(0x7a7469ff),  // warm gray
        },
        responses: ResponseColors {
            informational: rgba(0x227c8fff), // blue teal
            success: rgba(0x2d8a5bff),       // jade
            redirect: rgba(0xaa7d24ff),      // light brown
            client_error: rgba(0xe7821bff),  // reference orange
            server_error: rgba(0xc43d3dff),  // red
        },
        syntax: SyntaxColors {
            plain: rgba(0x4b4641ff),       // warm graphite
            property: rgba(0x227c8fff),    // blue teal
            string: rgba(0x2d8a5bff),      // jade
            number: rgba(0xe7821bff),      // reference orange
            boolean: rgba(0x7c5bbdff),     // violet
            null: rgba(0x7a7469ff),        // warm gray
            punctuation: rgba(0xa9a199ff), // stone
        },
    }
}

/// Graphite Honey — Probe's dark theme, paired with Porcelain Honey.
fn graphite_honey() -> Colors {
    Colors {
        surfaces: SurfaceColors {
            window: rgba(0x1d1e22ff),  // deep carbon
            sidebar: rgba(0x1a1b1fff), // charcoal navigation
            editor: rgba(0x222327ff),  // graphite editor
            raised: rgba(0x222327ff),  // lifted graphite
            overlay: rgba(0x292a2eff), // dark metal overlay
            scrim: rgba(0x00000059),   // black @ ~35%
        },
        text: TextColors {
            primary: rgba(0xe9e6e0ff),     // soft off-white
            secondary: rgba(0xcac5bdff),   // muted text
            muted: rgba(0xa7a19aff),       // graphite text
            placeholder: rgba(0x807c76ff), // stone
            inverse: rgba(0x1d1e22ff),     // on-accent
        },
        borders: BorderColors {
            subtle: rgba(0x33353aa6),   // graphite edge
            standard: rgba(0x45474dc2), // graphite divider
            strong: rgba(0x606268d9),   // grounded divider
            focused: rgba(0xd98e2699),  // quiet orange focus
        },
        actions: ActionColors {
            accent: rgba(0xd98e26ff),              // filled golden orange
            hover: rgba(0xf0a338ff),               // bright golden orange
            pressed: rgba(0xb97620ff),             // deep golden orange
            disabled: rgba(0x33353aa6),            // graphite edge
            disabled_foreground: rgba(0x807c76ff), // stone
        },
        selection: SelectionColors {
            active_background: rgba(0xd98e26ff),   // filled golden orange
            active_foreground: rgba(0x1d1e22ff),   // carbon
            inactive_background: rgba(0x33353aa6), // graphite edge
            inactive_foreground: rgba(0xe9e6e0ff), // soft off-white
        },
        status: StatusColors {
            success: rgba(0x79d19aff),       // jade
            warning: rgba(0xe39a32ff),       // burnt amber
            error: rgba(0xff7f7fff),         // red
            informational: rgba(0x75c6d4ff), // blue teal
        },
        methods: MethodColors {
            get: rgba(0x72d6c2ff),    // teal
            post: rgba(0xe39a32ff),   // burnt amber
            put: rgba(0xcaa95fff),    // light brown
            patch: rgba(0xb89af7ff),  // violet
            delete: rgba(0xff7f7fff), // red
            other: rgba(0xa6a39eff),  // warm gray
        },
        responses: ResponseColors {
            informational: rgba(0x75c6d4ff), // blue teal
            success: rgba(0x79d19aff),       // jade
            redirect: rgba(0xcaa95fff),      // light brown
            client_error: rgba(0xe39a32ff),  // burnt amber
            server_error: rgba(0xff7f7fff),  // red
        },
        syntax: SyntaxColors {
            plain: rgba(0xe9e6e0ff),       // soft off-white
            property: rgba(0x75c6d4ff),    // blue teal
            string: rgba(0x79d19aff),      // jade
            number: rgba(0xe39a32ff),      // burnt amber
            boolean: rgba(0xb89af7ff),     // violet
            null: rgba(0xa6a39eff),        // warm gray
            punctuation: rgba(0x85817bff), // stone
        },
    }
}

const fn platform_typography() -> Typography {
    Typography {
        interface_family: interface_font(),
        monospace_family: monospace_font(),
        body_size: 13.0,
        caption_size: 12.0,
        title_size: 24.0,
        dialog_title_size: 21.0,
        body_line_height: 1.45,
        regular_weight: 400,
        medium_weight: 500,
        semibold_weight: 600,
    }
}

#[cfg(target_os = "macos")]
const fn interface_font() -> &'static str {
    ".SystemUIFont"
}

#[cfg(target_os = "windows")]
const fn interface_font() -> &'static str {
    "Segoe UI Variable"
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const fn interface_font() -> &'static str {
    "sans-serif"
}

const fn monospace_font() -> &'static str {
    "JetBrains Mono"
}

/// JetBrains Mono 2.304 (SIL OFL 1.1), bundled in `assets/fonts/jetbrains-mono`.
fn load_bundled_fonts(cx: &mut gpui::App) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(
                include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-Medium.ttf")
                    .as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/jetbrains-mono/JetBrainsMono-SemiBold.ttf")
                    .as_slice(),
            ),
        ])
        .expect("failed to load bundled JetBrains Mono fonts");
}

const fn default_metrics() -> Metrics {
    Metrics {
        spacing_1: 4.0,
        spacing_2: 8.0,
        spacing_3: 12.0,
        spacing_4: 16.0,
        radius_small: 6.0,
        radius_medium: 8.0,
        radius_large: 10.0,
        control_height: 28.0,
        tree_row_height: 28.0,
        tab_bar_height: 32.0,
        icon_small: 12.0,
        icon_standard: 16.0,
        elevation_raised: 1.0,
    }
}

const fn default_motion() -> Motion {
    Motion {
        fast_ms: 80,
        standard_ms: 160,
        slow_ms: 240,
        easing: "cubic-bezier(0.2, 0, 0, 1)",
        reduced_duration_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{Theme, ThemeAppearance};
    use gpui::WindowAppearance;

    #[test]
    fn native_appearances_select_the_expected_built_in_theme() {
        assert_eq!(
            Theme::for_window_appearance(WindowAppearance::Light).appearance,
            ThemeAppearance::Light
        );
        assert_eq!(
            Theme::for_window_appearance(WindowAppearance::VibrantLight).appearance,
            ThemeAppearance::Light
        );
        assert_eq!(
            Theme::for_window_appearance(WindowAppearance::Dark).appearance,
            ThemeAppearance::Dark
        );
        assert_eq!(
            Theme::for_window_appearance(WindowAppearance::VibrantDark).appearance,
            ThemeAppearance::Dark
        );
    }
}
