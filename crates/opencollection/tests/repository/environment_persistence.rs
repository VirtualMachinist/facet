use super::*;

pub(super) fn resolved_variable(
    loaded: &probe_opencollection::LoadedWorkspace,
    environment: &str,
    name: &str,
) -> Option<String> {
    resolve_environment(loaded.workspace().environments(), environment)
        .ok()
        .and_then(|resolved| resolved.variable(name).map(str::to_owned))
}

#[test]
fn bundled_environment_set_unset_save_reload_preserves_unknown_fields() {
    let path = temporary_path("bundled-env.yml");
    fs::write(
        &path,
        concat!(
            "opencollection: 1.0.0\n",
            "info:\n  name: Env persist\n",
            "bundled: true\n",
            "config:\n",
            "  environments:\n",
            "    - name: base\n",
            "      vendor.example: retained-base\n",
            "      variables:\n",
            "        - name: host\n",
            "          value: api.example.com\n",
            "          description: Canonical host\n",
            "        - name: region\n",
            "          value:\n",
            "            - title: AU\n",
            "              selected: true\n",
            "              value: au\n",
            "              note: default\n",
            "            - title: US\n",
            "              value: us\n",
            "    - name: development\n",
            "      extends: base\n",
            "      variables:\n",
            "        - name: host\n",
            "          value: dev.example.com\n",
            "        - name: token\n",
            "          value: development-token\n",
            "items:\n",
            "  - info:\n",
            "      name: Health\n",
            "      type: http\n",
            "    http:\n",
            "      method: GET\n",
            "      url: https://example.com/health\n",
        ),
    )
    .unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    loaded
        .update_environment_variable("development", "host", "local.example.com".to_owned())
        .unwrap();
    loaded
        .update_environment_variable("development", "baseUrl", "https://local.example".to_owned())
        .unwrap();
    loaded
        .update_environment_variable("base", "region", "nz".to_owned())
        .unwrap();
    loaded
        .unset_environment_variable("development", "host")
        .unwrap();

    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        resolved_variable(&reloaded, "development", "host").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(
        resolved_variable(&reloaded, "development", "baseUrl").as_deref(),
        Some("https://local.example")
    );
    assert_eq!(
        resolved_variable(&reloaded, "base", "region").as_deref(),
        Some("nz")
    );
    let saved = fs::read_to_string(&path).unwrap();
    assert!(saved.contains("vendor.example: retained-base"));
    assert!(saved.contains("description: Canonical host"));
    assert!(saved.contains("note: default"));
    assert!(saved.contains("title: US"));
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_environment_set_unset_save_reload_preserves_unknown_fields() {
    let root = temporary_path("unbundled-env");
    copy_directory(&fixture("unbundled"), &root);
    fs::write(
        root.join("environments/base.yml"),
        concat!(
            "name: base\n",
            "variables:\n",
            "  - name: host\n",
            "    value: api.example.com\n",
        ),
    )
    .unwrap();
    fs::write(
        root.join("environments/development.yml"),
        concat!(
            "name: development\n",
            "extends: base\n",
            "color: green\n",
            "vendor.example: retained-env\n",
            "variables:\n",
            "  - name: host\n",
            "    value: dev.example.com\n",
            "    description: Child host\n",
            "  - name: baseUrl\n",
            "    value: https://{{host}}\n",
        ),
    )
    .unwrap();

    let mut loaded = load_workspace(&root).unwrap();
    loaded
        .update_environment_variable("development", "host", "local.example.com".to_owned())
        .unwrap();
    loaded
        .unset_environment_variable("development", "host")
        .unwrap();
    loaded
        .update_environment_variable("development", "token", "dev-token".to_owned())
        .unwrap();

    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(
        resolved_variable(&reloaded, "development", "host").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(
        resolved_variable(&reloaded, "development", "token").as_deref(),
        Some("dev-token")
    );
    let saved = fs::read_to_string(root.join("environments/development.yml")).unwrap();
    assert!(saved.contains("vendor.example: retained-env"));
    assert!(saved.contains("color: green"));
    let base = fs::read_to_string(root.join("environments/base.yml")).unwrap();
    assert!(base.contains("value: api.example.com"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_update_refuses_externally_modified_document() {
    let path = temporary_path("env-conflict.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let mut external = fs::read_to_string(&path).unwrap();
    external.push_str("external: true\n");
    fs::write(&path, &external).unwrap();

    let error = loaded
        .update_environment_variable("development", "token", "rotated".to_owned())
        .expect_err("external modification should be rejected");
    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
    assert_eq!(
        resolved_variable(&loaded, "development", "token").as_deref(),
        Some("rotated")
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_replace_preserves_unknown_and_secret_fields() {
    let path = temporary_path("env-replace.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let mut replacement = loaded.workspace().environments()[1].clone();
    replacement.extends = None;
    replacement.variables.retain(|variable| match variable {
        probe_core::EnvironmentVariable::Plain(variable) => {
            variable.name.as_deref() != Some("host")
        }
        probe_core::EnvironmentVariable::Secret(_) => true,
    });
    replacement
        .variables
        .push(probe_core::EnvironmentVariable::Plain(
            probe_core::Variable {
                name: Some("region".to_owned()),
                value: Some(probe_core::VariableValueSet::Single(
                    probe_core::VariableValue::String("ap-southeast-2".to_owned()),
                )),
                disabled: true,
            },
        ));

    let prepared = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);

    let source = fs::read_to_string(&path).unwrap();
    assert!(!source.contains("extends: base"));
    assert!(source.contains("name: region"));
    assert!(source.contains("disabled: true"));
    assert!(source.contains("secret: true"));
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        loaded.workspace().environments()[1],
        reloaded.workspace().environments()[1]
    );
    assert_eq!(reloaded.workspace().environments()[1].extends, None);
    assert!(reloaded.workspace().environments()[1].variables.iter().all(
        |variable| match variable {
            probe_core::EnvironmentVariable::Plain(variable) => {
                variable.name.as_deref() != Some("host")
            }
            probe_core::EnvironmentVariable::Secret(_) => true,
        }
    ));

    let mut base = loaded.workspace().environments()[0].clone();
    base.variables
        .retain(|variable| matches!(variable, probe_core::EnvironmentVariable::Plain(_)));
    let prepared = loaded.prepare_environment_replace("base", base).unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(
        loaded.workspace().environments()[0],
        reloaded.workspace().environments()[0]
    );
    assert!(
        reloaded.workspace().environments()[0]
            .variables
            .iter()
            .any(|variable| matches!(
                variable,
                probe_core::EnvironmentVariable::Secret(secret)
                    if secret.name.as_deref() == Some("secretToken")
            ))
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_replace_rejects_variable_name_collisions_without_writing() {
    let path = temporary_path("env-replace-collisions.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let original = fs::read_to_string(&path).unwrap();
    let loaded = load_workspace(&path).unwrap();
    let base = loaded.workspace().environments()[0].clone();
    let mut replacement = base.clone();
    replacement.variables.retain(|variable| {
        !matches!(
            variable,
            probe_core::EnvironmentVariable::Secret(secret)
                if secret.name.as_deref() == Some("secretToken")
        )
    });
    replacement
        .variables
        .push(probe_core::EnvironmentVariable::Plain(
            probe_core::Variable {
                name: Some("secretToken".to_owned()),
                value: Some(probe_core::VariableValueSet::Single(
                    probe_core::VariableValue::String("plain".to_owned()),
                )),
                disabled: false,
            },
        ));

    let error = loaded
        .prepare_environment_replace("base", replacement)
        .unwrap_err();
    assert!(
        matches!(
            error,
            SaveError::Environment(EnvironmentResolutionError::DuplicateVariable {
                ref environment,
                ref variable,
            }) if environment == "base" && variable == "secretToken"
        ),
        "{error:?}"
    );
    assert_eq!(loaded.workspace().environments()[0], base);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    fs::remove_file(path).unwrap();
}

#[test]
fn bundled_environment_delete_updates_following_indices() {
    let path = temporary_path("env-delete.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();
    let prepared = loaded.prepare_environment_delete("development").unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_delete(saved);
    assert!(
        loaded
            .workspace()
            .environments()
            .iter()
            .all(|environment| environment.name != "development")
    );
    let reloaded = load_workspace(&path).unwrap();
    assert_eq!(reloaded.workspace().environments().len(), 1);
    fs::remove_file(path).unwrap();
}

#[test]
fn unbundled_environment_delete_removes_only_the_environment_document() {
    let root = temporary_path("unbundled-env-delete");
    copy_directory(&fixture("unbundled"), &root);
    fs::write(
        root.join("environments/development.yml"),
        "name: development\nvariables:\n  - name: token\n    value: dev\n",
    )
    .unwrap();
    let mut loaded = load_workspace(&root).unwrap();
    let prepared = loaded.prepare_environment_delete("development").unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_delete(saved);
    assert!(!root.join("environments/development.yml").exists());
    assert!(root.join("opencollection.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_environment_rename_moves_the_document() {
    let root = temporary_path("unbundled-env-rename");
    copy_directory(&fixture("unbundled"), &root);
    let mut loaded = load_workspace(&root).unwrap();
    let mut replacement = loaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .cloned()
        .unwrap();
    replacement.name = "staging".to_owned();
    let prepared = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap();
    let saved = prepared.execute().unwrap();
    loaded.complete_environment_replace(saved);

    assert!(!root.join("environments/development.yml").exists());
    assert!(root.join("environments/staging.yml").exists());
    let saved = fs::read_to_string(root.join("environments/staging.yml")).unwrap();
    assert!(saved.contains("name: staging"));
    assert!(saved.contains("color: green"));
    let reloaded = load_workspace(&root).unwrap();
    assert_eq!(
        loaded
            .workspace()
            .environments()
            .iter()
            .map(|environment| environment.name.as_str())
            .collect::<Vec<_>>(),
        reloaded
            .workspace()
            .environments()
            .iter()
            .map(|environment| environment.name.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .any(|environment| environment.name == "staging")
    );
    loaded
        .create_environment("development".to_owned(), None)
        .unwrap();
    assert!(root.join("environments/development.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unbundled_environment_rename_refuses_an_existing_destination() {
    let root = temporary_path("unbundled-env-rename-conflict");
    copy_directory(&fixture("unbundled"), &root);
    let loaded = load_workspace(&root).unwrap();
    let original = fs::read_to_string(root.join("environments/development.yml")).unwrap();
    fs::write(
        root.join("environments/staging.yml"),
        "name: staging\nvariables:\n  - name: token\n    value: staging\n",
    )
    .unwrap();
    let mut replacement = loaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .cloned()
        .unwrap();
    replacement.name = "staging".to_owned();
    let error = loaded
        .prepare_environment_replace("development", replacement)
        .unwrap()
        .execute()
        .unwrap_err();
    assert!(matches!(error, SaveError::ConcurrentModification(_)));
    assert_eq!(
        fs::read_to_string(root.join("environments/development.yml")).unwrap(),
        original
    );
    assert_eq!(
        loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == "development")
            .map(|environment| environment.name.as_str()),
        Some("development")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn environment_update_rejects_secrets_and_missing_environments() {
    let path = temporary_path("env-secrets.yml");
    fs::copy(fixture("phase4-environments.yml"), &path).unwrap();
    let mut loaded = load_workspace(&path).unwrap();

    let secret = loaded
        .update_environment_variable("development", "secretToken", "nope".to_owned())
        .unwrap_err();
    assert!(matches!(
        secret,
        SaveError::Environment(EnvironmentResolutionError::SecretVariableUnavailable(_))
    ));
    let missing = loaded
        .update_environment_variable("production", "host", "nope".to_owned())
        .unwrap_err();
    assert!(matches!(
        missing,
        SaveError::Environment(EnvironmentResolutionError::EnvironmentNotFound(_))
    ));
    let unset_missing = loaded
        .unset_environment_variable("development", "baseUrl")
        .unwrap_err();
    assert!(matches!(
        unset_missing,
        SaveError::Environment(EnvironmentResolutionError::VariableNotFound { .. })
    ));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        fs::read_to_string(fixture("phase4-environments.yml")).unwrap()
    );
    fs::remove_file(path).unwrap();
}

#[test]
fn environment_update_rejects_stdin_workspaces() {
    let source = fs::read_to_string(fixture("phase4-environments.yml")).unwrap();
    let mut loaded = load_workspace_from_str(&source).unwrap();
    let error = loaded
        .update_environment_variable("development", "token", "rotated".to_owned())
        .unwrap_err();
    assert!(matches!(error, SaveError::ReadOnlySource));
    assert_eq!(
        resolved_variable(&loaded, "development", "token").as_deref(),
        Some("rotated")
    );
}
