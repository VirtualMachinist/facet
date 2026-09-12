//! `facet ncl pack --out <dir>`: materialize the embedded contract pack so
//! out-of-tree worlds can `import "hedron-ncl/platform.ncl"` under
//! `--import-path <dir>` (tetra M1 G0a). Bytes are the crate's `include_str!`
//! sources; `hedron_ncl::pack::verify` proves it afterwards.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{CommandOutput, FacetError, args, versioned_json};

pub(crate) fn pack(args: &[String]) -> Result<CommandOutput, FacetError> {
    let parsed = args::parse(args, &["--out"], &[])?;
    if !parsed.positionals().is_empty() {
        return Err(FacetError::invalid_arguments(
            "ncl pack takes no positional arguments; use --out <dir>",
        ));
    }
    let out = parsed
        .value("--out")?
        .map(PathBuf::from)
        .ok_or_else(|| FacetError::invalid_arguments("ncl pack requires --out <dir>"))?;
    let written =
        hedron_ncl::pack::write_to(&out).map_err(|error| FacetError::output(&out, &error))?;
    let files: Vec<Value> = hedron_ncl::pack::FILES
        .iter()
        .map(|file| json!(format!("{}/{}", hedron_ncl::pack::ROOT, file.path)))
        .collect();
    let json = versioned_json(json!({
        "importPath": out.display().to_string(),
        "root": written.root.display().to_string(),
        "contractSet": written.contract_set,
        "marker": written.marker.display().to_string(),
        "files": files,
    }));
    let mut human = format!(
        "ok {} (contractSet={} files={})\n",
        written.root.display(),
        written.contract_set,
        written.files.len()
    );
    human.push_str(&format!(
        "import with: facet ncl <verb> <world.ncl> --import-path {}\n",
        out.display()
    ));
    Ok(CommandOutput::new(human, json))
}

#[cfg(test)]
mod tests {
    use crate::{INVALID_ARGUMENTS_EXIT_CODE, run};

    #[test]
    fn pack_writes_and_reports_the_pack() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("pack");
        let output = run(["ncl", "pack", "--out", out.to_str().unwrap(), "--json"]);
        assert_eq!(
            output.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["contractSet"], hedron_ncl::CONTRACT_SET);
        assert_eq!(value["importPath"], out.to_str().unwrap());
        let files = value["files"].as_array().unwrap();
        assert_eq!(files.len(), hedron_ncl::pack::FILES.len());
        assert!(files.iter().any(|f| f == "hedron-ncl/platform.ncl"));
        assert!(hedron_ncl::pack::verify(&out).unwrap().is_empty());
        assert!(
            out.join("hedron-ncl")
                .join(hedron_ncl::pack::CONTRACT_SET_MARKER_FILE)
                .is_file()
        );
    }

    #[test]
    fn pack_human_line_names_import_path() {
        let dir = tempfile::tempdir().unwrap();
        let output = run(["ncl", "pack", "--out", dir.path().to_str().unwrap()]);
        assert_eq!(output.exit_code, 0);
        let human = String::from_utf8(output.stdout).unwrap();
        assert!(human.starts_with("ok "), "{human}");
        assert!(human.contains("--import-path"), "{human}");
    }

    #[test]
    fn pack_requires_out() {
        let output = run(["ncl", "pack", "--json"]);
        assert_eq!(output.exit_code, INVALID_ARGUMENTS_EXIT_CODE);
        assert!(String::from_utf8_lossy(&output.stdout).contains("--out"));
    }
}
