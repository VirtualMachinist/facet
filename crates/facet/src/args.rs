//! Minimal argument parsing for Facet-owned commands. Mirrors upstream's
//! non-interactive style: `--flag value` or `--flag=value`, unknown options
//! rejected as `invalid_arguments`.

use crate::FacetError;

use std::path::{Path, PathBuf};

use crate::{CommandOutput, ncl, versioned_json};
use facet_record::{ConfigOverrides, open_store};
use lattice::{HistoryQuery, WorkspaceStore};

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    positionals: Vec<String>,
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

pub(crate) fn parse(
    args: &[String],
    value_flags: &[&str],
    switch_flags: &[&str],
) -> Result<Parsed, FacetError> {
    let mut parsed = Parsed::default();
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        if let Some((flag, value)) = argument.split_once('=')
            && value_flags.contains(&flag)
        {
            parsed.values.push((flag.to_owned(), value.to_owned()));
            continue;
        }
        if value_flags.contains(&argument.as_str()) {
            let value = iter.next().ok_or_else(|| {
                FacetError::invalid_arguments(format!("{argument} requires a value"))
            })?;
            parsed.values.push((argument.clone(), value.clone()));
            continue;
        }
        if switch_flags.contains(&argument.as_str()) {
            if parsed.switches.contains(argument) {
                return Err(FacetError::invalid_arguments(format!(
                    "{argument} may only be specified once"
                )));
            }
            parsed.switches.push(argument.clone());
            continue;
        }
        if argument.starts_with('-') && argument != "-" {
            return Err(FacetError::invalid_arguments(format!(
                "unknown option: {argument}"
            )));
        }
        parsed.positionals.push(argument.clone());
    }
    Ok(parsed)
}

impl Parsed {
    pub(crate) fn positionals(&self) -> &[String] {
        &self.positionals
    }

    /// A single-valued flag; repeating it is an error.
    pub(crate) fn value(&self, flag: &str) -> Result<Option<&str>, FacetError> {
        let mut found = self
            .values
            .iter()
            .filter(|(name, _)| name == flag)
            .map(|(_, value)| value.as_str());
        let first = found.next();
        if found.next().is_some() {
            return Err(FacetError::invalid_arguments(format!(
                "{flag} may only be specified once"
            )));
        }
        Ok(first)
    }

    /// Every value of a repeatable flag, in order.
    pub(crate) fn values(&self, flag: &str) -> Vec<&str> {
        self.values
            .iter()
            .filter(|(name, _)| name == flag)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    pub(crate) fn switch(&self, flag: &str) -> bool {
        self.switches.iter().any(|name| name == flag)
    }

    pub(crate) fn parsed_value<T: std::str::FromStr>(
        &self,
        flag: &str,
        what: &str,
    ) -> Result<Option<T>, FacetError> {
        self.value(flag)?
            .map(|value| {
                value.parse::<T>().map_err(|_| {
                    FacetError::invalid_arguments(format!("{flag} expects {what}, got {value:?}"))
                })
            })
            .transpose()
    }
}

const NCL_VALUE_FLAGS: &[&str] = &["--var"];
const NCL_SWITCH_FLAGS: &[&str] = &["--frozen"];

const NCL_SECRET_SEGMENTS: &[&str] = &["token", "password", "api_key", "authorization", "secret"];

pub(crate) fn ncl(args: &[String]) -> Result<CommandOutput, FacetError> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(FacetError::invalid_arguments(
            "ncl requires a subcommand: check, export, apply, or pack",
        ));
    };
    match verb.as_str() {
        "check" => ncl_check(rest),
        "export" => ncl_export(rest),
        "apply" => ncl_apply(rest),
        "pack" => crate::ncl_pack::pack(rest),
        other => Err(FacetError::invalid_arguments(format!(
            "unknown ncl subcommand: {other} (expected check, export, apply, or pack)"
        ))),
    }
}

pub(crate) fn reject_secret_overrides(overrides: &[String]) -> Result<(), FacetError> {
    for assignment in overrides {
        let path = assignment
            .split_once("=")
            .map(|(path, _)| path)
            .unwrap_or(assignment.as_str());
        for segment in path.split(".") {
            let lower = segment.to_ascii_lowercase();
            if NCL_SECRET_SEGMENTS.iter().any(|secret| lower == *secret) {
                return Err(FacetError::invalid_arguments(format!(
                    "refuses secret field override `{path}`; use secret_ref and Facet env hydration instead"
                )));
            }
        }
    }
    Ok(())
}

fn ncl_check(args: &[String]) -> Result<CommandOutput, FacetError> {
    let (path, overrides, frozen) = parse_ncl_cli(args)?;
    if frozen {
        return Err(FacetError::invalid_arguments(
            "ncl check does not support --frozen; use ncl export --frozen",
        ));
    }
    let result = ncl::check(&path, &overrides)?;
    let json = versioned_json(result.to_json());
    let human = format!(
        "ok {} (moduleHash={} contractSet={})\n",
        result.path.display(),
        result.module_hash,
        result.contract_set,
    );
    Ok(CommandOutput::new(human, json))
}

fn ncl_apply(args: &[String]) -> Result<CommandOutput, FacetError> {
    let (path, overrides, frozen) = parse_ncl_cli(args)?;
    if frozen {
        return Err(FacetError::invalid_arguments(
            "ncl apply does not support --frozen; use ncl export --frozen",
        ));
    }
    let result = ncl::apply(&path, &overrides)?;
    let json = versioned_json(result.to_json());
    let human = format!(
        "ok {} (moduleHash={} exportHash={} contractSet={} cluster.posted={} intent.put={})\n",
        result.path.display(),
        result.module_hash,
        result.export_hash,
        result.contract_set,
        result.action.cluster.posted,
        result.action.intent.put,
    );
    Ok(CommandOutput::new(human, json))
}

fn ncl_export(args: &[String]) -> Result<CommandOutput, FacetError> {
    let (path, overrides, frozen) = parse_ncl_cli(args)?;
    if frozen {
        refuse_if_export_changed(&path, &overrides)?;
    }
    let result = ncl::export(&path, &overrides)?;
    let json = versioned_json(result.to_json());
    let human = format!(
        "ok {} (moduleHash={} exportHash={} contractSet={})\n",
        result.path.display(),
        result.module_hash,
        result.export_hash,
        result.contract_set,
    );
    Ok(CommandOutput::new(human, json))
}

fn parse_ncl_cli(args: &[String]) -> Result<(PathBuf, Vec<String>, bool), FacetError> {
    let parsed = parse(args, NCL_VALUE_FLAGS, NCL_SWITCH_FLAGS)?;
    let [path] = parsed.positionals() else {
        return Err(FacetError::invalid_arguments(
            "ncl requires <path> to a .ncl module",
        ));
    };
    let overrides = parsed
        .values("--var")
        .into_iter()
        .map(|entry| {
            if !entry.contains("=") {
                return Err(FacetError::invalid_arguments(format!(
                    "--var expects path.to.field=value, got {entry:?}"
                )));
            }
            Ok(entry.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    reject_secret_overrides(&overrides)?;
    Ok((PathBuf::from(path), overrides, parsed.switch("--frozen")))
}

/// Re-evaluates without recording and refuses when the export hash drifted.
fn refuse_if_export_changed(path: &Path, overrides: &[String]) -> Result<(), FacetError> {
    let Some((run_id, recorded_hash)) = lookup_recorded_export(path)? else {
        return Err(FacetError::invalid_arguments(
            "--frozen requires a prior ncl export recorded in Lattice for this module",
        ));
    };

    let current = ncl::evaluate(path, overrides)?;

    if current.export_hash != recorded_hash {
        return Err(FacetError::replay_changed(
            &run_id,
            &recorded_hash,
            &current.export_hash,
        ));
    }
    Ok(())
}

fn lookup_recorded_export(path: &Path) -> Result<Option<(String, String)>, FacetError> {
    let workspace_root = workspace_root_for(path)?;
    let store = match WorkspaceStore::discover(&workspace_root) {
        Some(root) => {
            open_store(&root, &ConfigOverrides::default()).map_err(FacetError::lattice)?
        }
        None => return Ok(None),
    };
    let module_url = module_url(&workspace_root, path);
    let rows = store
        .history(&HistoryQuery {
            limit: 50,
            request_path: Some("ncl:export".to_owned()),
            status: None,
            actor: None,
            since: None,
            session_id: None,
            environment: None,
            tags: Vec::new(),
            hash: None,
        })
        .map_err(FacetError::lattice)?;
    for row in rows {
        if row.url != module_url {
            continue;
        }
        if let Some(hash) = export_hash_from_tags(row.tags.as_deref()) {
            return Ok(Some((row.id.clone(), hash)));
        }
    }
    Ok(None)
}

fn export_hash_from_tags(tags: Option<&str>) -> Option<String> {
    let tags: Vec<String> = tags
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default();
    tags.into_iter()
        .find_map(|tag| tag.strip_prefix("ncl:export:").map(str::to_owned))
}

fn workspace_root_for(path: &Path) -> Result<PathBuf, FacetError> {
    if let Some(root) = WorkspaceStore::discover(path) {
        return Ok(root);
    }
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(path);
    Ok(parent.to_path_buf())
}

fn module_url(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn parses_values_switches_and_positionals() {
        let parsed = parse(
            &args(&["a", "--limit", "5", "--tag=x", "--tag", "y", "--yes", "b"]),
            &["--limit", "--tag"],
            &["--yes"],
        )
        .unwrap();
        assert_eq!(parsed.positionals(), ["a", "b"]);
        assert_eq!(parsed.value("--limit").unwrap(), Some("5"));
        assert_eq!(parsed.values("--tag"), ["x", "y"]);
        assert!(parsed.switch("--yes"));
        assert_eq!(
            parsed.parsed_value::<usize>("--limit", "a number").unwrap(),
            Some(5)
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_options() {
        assert!(parse(&args(&["--nope"]), &[], &[]).is_err());
        assert!(parse(&args(&["--yes", "--yes"]), &[], &["--yes"]).is_err());
        assert!(parse(&args(&["--limit"]), &["--limit"], &[]).is_err());
        let parsed = parse(&args(&["--limit", "1", "--limit", "2"]), &["--limit"], &[]).unwrap();
        assert!(parsed.value("--limit").is_err());
        assert!(parsed.parsed_value::<usize>("--limit", "a number").is_err());
    }
}
