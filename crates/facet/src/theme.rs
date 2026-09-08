//! `facet theme`: validate and list `facet-tui` theme files without a
//! terminal. Theme files are local presentation config (DESIGN.md), never
//! collection data, so this command touches no workspace and no Lattice.
//!
//! `check` is the agent/CI surface: exit 0 when the file parses, exit 1
//! (assertion family, `theme_invalid`) when the TUI would fall back to a
//! built-in — with the file and field named in `details`.

use std::path::Path;

use facet_tui::Appearance;
use facet_tui::theme_file::{self, ThemeFile};
use serde_json::json;

use crate::{CommandOutput, FacetError, args};

pub(crate) fn theme(args: &[String]) -> Result<CommandOutput, FacetError> {
    match args.first().map(String::as_str) {
        Some("check") => check(&args[1..]),
        Some("list") => list(&args[1..]),
        _ => Err(FacetError::invalid_arguments(
            "theme requires a subcommand: check <path>, list",
        )),
    }
}

fn check(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    let [path] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "theme check requires exactly one <path>",
        ));
    };
    let path = Path::new(path.as_str());
    match ThemeFile::load(path) {
        Ok(theme) => Ok(CommandOutput::new(
            format!(
                "theme {} valid (extends {}, {} overrides)\n",
                theme.name(),
                appearance_flag(theme.extends()),
                theme.override_count(),
            ),
            json!({ "theme": {
                "path": path.to_string_lossy(),
                "name": theme.name(),
                "extends": appearance_flag(theme.extends()),
                "overrides": theme.override_count(),
                "valid": true,
            } }),
        )),
        Err(error) => Err(FacetError::theme_invalid(&error)),
    }
}

fn list(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &[], &[])?;
    if !parsed.positionals().is_empty() {
        return Err(FacetError::invalid_arguments(
            "theme list takes no arguments",
        ));
    }
    let entries = theme_file::list_themes();
    let mut lines = vec!["NAME\tSOURCE\tEXTENDS\tPATH".to_owned()];
    let mut themes = Vec::with_capacity(entries.len());
    for entry in &entries {
        let extends = entry.extends.map(appearance_flag);
        lines.push(format!(
            "{}\t{}\t{}\t{}{}",
            entry.name,
            entry.source,
            extends.unwrap_or("-"),
            entry
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_owned()),
            entry
                .error
                .as_ref()
                .map(|error| format!(" (invalid: {error})"))
                .unwrap_or_default(),
        ));
        themes.push(json!({
            "name": entry.name,
            "source": entry.source,
            "extends": extends,
            "path": entry.path.as_ref().map(|path| path.to_string_lossy()),
            "valid": entry.error.is_none(),
            "error": entry.error,
        }));
    }
    Ok(CommandOutput::new(
        format!("{}\n", lines.join("\n")),
        json!({ "themes": themes }),
    ))
}

fn appearance_flag(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Dark => "graphite",
        Appearance::Light => "porcelain",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_a_bad_version_with_file_and_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "version = 2\nextends = \"graphite\"\n").unwrap();
        let error = theme(&["check".to_owned(), path.display().to_string()]).unwrap_err();
        assert_eq!(error.category, "theme_invalid");
        assert_eq!(error.exit_code, 1);
        assert!(error.message.contains("broken.toml"));
        assert!(error.message.contains("version 2"));
    }

    #[test]
    fn check_accepts_a_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ok.toml");
        std::fs::write(
            &path,
            "version = 1\nextends = \"porcelain\"\n[colors]\naccent = \"#123456\"\n",
        )
        .unwrap();
        let output = theme(&["check".to_owned(), path.display().to_string()]).unwrap();
        assert_eq!(output.json["theme"]["valid"], true);
        assert_eq!(output.json["theme"]["extends"], "porcelain");
        assert_eq!(output.json["theme"]["overrides"], 1);
    }
}
