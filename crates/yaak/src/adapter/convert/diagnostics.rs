use super::*;
pub(super) use probe_core::{
    lossy_import_diagnostic as lossy, nonempty_string as nonempty,
    warning_import_diagnostic as warning,
};

pub(super) fn diagnose_workspace(
    workspace: &YaakWorkspace,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    diagnose_extra_fields(
        "workspace",
        Some(&workspace.id),
        &workspace.extra,
        diagnostics,
    );
    if workspace.encryption_key_challenge.is_some() {
        diagnostics.push(lossy(
            "unsupported_field",
            "workspace",
            Some(&workspace.id),
            Some("encryptionKeyChallenge"),
            "Yaak workspace encryption metadata cannot be represented by OpenCollection",
        ));
    }
    if !workspace.setting_dns_overrides.is_empty() {
        diagnostics.push(lossy(
            "unsupported_setting",
            "workspace",
            Some(&workspace.id),
            Some("settingDnsOverrides"),
            "Yaak DNS overrides cannot be represented by the current Probe domain",
        ));
    }
}

pub(super) fn diagnose_folder(folder: &YaakFolder, diagnostics: &mut Vec<ImportDiagnostic>) {
    diagnose_extra_fields("folder", Some(&folder.id), &folder.extra, diagnostics);
    if !folder.description.trim().is_empty() {
        diagnostics.push(lossy(
            "unsupported_field",
            "folder",
            Some(&folder.id),
            Some("description"),
            "folder descriptions cannot be represented by the current Probe domain",
        ));
    }
}

pub(crate) fn diagnose_extra_fields(
    resource_type: &str,
    resource_id: Option<&str>,
    extra: &BTreeMap<String, Value>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    diagnostics.extend(extra.keys().map(|field| {
        lossy(
            "unknown_field",
            resource_type,
            resource_id,
            Some(field),
            &format!("unknown Yaak field '{field}' cannot be guaranteed to survive import"),
        )
    }));
}

pub(super) fn convert_templates(
    input: &str,
    resource_type: &str,
    resource_id: &str,
    field: &str,
    diagnostics: &mut Vec<ImportDiagnostic>,
) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${[") {
        output.push_str(&rest[..start]);
        let expression = &rest[start + 3..];
        let Some(end) = expression.find("]}") else {
            output.push_str(&rest[start..]);
            diagnostics.push(lossy(
                "unsupported_template",
                resource_type,
                Some(resource_id),
                Some(field),
                "unterminated Yaak template expression was preserved literally",
            ));
            return output;
        };
        let raw = expression[..end].trim();
        let name = raw.strip_prefix("env.").unwrap_or(raw).trim();
        let simple = !name.is_empty()
            && !name.contains('(')
            && !name.contains(')')
            && !name.contains(' ')
            && (!raw.contains('.') || raw.starts_with("env."));
        if simple {
            output.push_str("{{");
            output.push_str(name);
            output.push_str("}}");
        } else {
            output.push_str(&rest[start..start + 3 + end + 2]);
            diagnostics.push(lossy(
                "unsupported_template",
                resource_type,
                Some(resource_id),
                Some(field),
                &format!("Yaak template '${{[{raw}]}}' was preserved literally"),
            ));
        }
        rest = &expression[end + 2..];
    }
    output.push_str(rest);
    output
}
