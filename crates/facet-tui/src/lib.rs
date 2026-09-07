//! Facet's ratatui TUI (`facet tui`).
//!
//! Layout + semantic-token translation of the Porcelain Honey /
//! Graphite Honey palette from the desktop `theme.rs` onto terminal cells.
//! The TUI is an interface over the shared application and domain crates
//! (CLI / desktop parity), not a parallel runtime.

mod app;
mod theme;
mod tree;
pub mod ui;

pub use app::{
    App, BodyKind, Editor, EnvironmentEntry, Focus, Mode, RecordSummary, RequestFocus, ResponseTab,
    ResponseView, RunResult, RunStatus, Section, TuiError,
};
pub use theme::{Appearance, Depth, Palette, PaletteToken, Role, Styles, Theme};
