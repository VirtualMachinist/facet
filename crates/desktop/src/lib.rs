//! Native GPUI presentation adapter for Probe.
//!
//! Presentation and interaction live here. Workspace and request behavior remains in
//! [`probe_core`] so the desktop and CLI continue to share the same application model.

#![forbid(unsafe_code)]

mod app;
mod caret;
mod components;
mod execution;
mod filesystem;
mod persistence;
mod request_editor;
mod response_inspector;
mod response_viewer;
mod session;
mod shell;
mod structure_editor;
mod synchronization;
mod syntax;
mod theme;
mod tree_search;

pub use app::run;
