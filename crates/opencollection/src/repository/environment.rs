use super::*;

pub(super) fn persist_environment_yaml(
    persistence: &EnvironmentPersistence,
    original_source: &[u8],
    mutation: &EnvironmentYamlMutation,
) -> Result<Vec<u8>, SaveError> {
    mutate_existing_document(&persistence.document_path, original_source, |document| {
        let environment = environment_document_mut(document, persistence)?;
        apply_environment_mutation(environment, mutation)
    })
}

pub(super) fn persist_bundled_environment_create(
    document_path: &Path,
    original_source: &[u8],
    environment: &Environment,
) -> Result<Vec<u8>, SaveError> {
    mutate_existing_document(document_path, original_source, |document| {
        let mapping = document.as_mapping_mut().ok_or_else(|| {
            SaveError::InvalidDocument("the collection document is not a mapping".to_owned())
        })?;
        let config = mapping_child(mapping, "config")?;
        let key = string_key("environments");
        if !config.get(&key).is_some_and(Value::is_sequence) {
            config.insert(key.clone(), Value::Sequence(Vec::new()));
        }
        let environments = config
            .get_mut(&key)
            .and_then(Value::as_sequence_mut)
            .expect("environments sequence must exist after insertion");
        environments.push(environment_value(environment));
        Ok(())
    })
}

pub(super) fn persist_unbundled_environment_create(
    root: &Path,
    environment: &Environment,
) -> Result<(PathBuf, Vec<u8>), SaveError> {
    let directory = root.join("environments");
    if !directory.exists() {
        fs::create_dir_all(&directory).map_err(|source| SaveError::Io {
            path: directory.clone(),
            source,
        })?;
    }
    let document_path = directory.join(format!("{}.yml", environment.name));
    if document_path.exists() {
        return Err(SaveError::ConcurrentModification(document_path));
    }
    let serialized = serde_yaml_ng::to_string(&environment_value(environment))
        .map_err(SaveError::Serialize)?
        .into_bytes();
    write_new_environment_file(&document_path, &serialized)?;
    Ok((document_path, serialized))
}

pub(super) fn persist_bundled_environment_delete(
    document_path: &Path,
    original_source: &[u8],
    index: usize,
) -> Result<Vec<u8>, SaveError> {
    mutate_existing_document(document_path, original_source, |document| {
        let mapping = document.as_mapping_mut().ok_or_else(|| {
            SaveError::InvalidDocument("the collection document is not a mapping".to_owned())
        })?;
        let config = mapping_child(mapping, "config")?;
        let environments = config
            .get_mut(string_key("environments"))
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| {
                SaveError::InvalidDocument(
                    "collection config has no environments sequence".to_owned(),
                )
            })?;
        if index >= environments.len() {
            return Err(SaveError::InvalidDocument(format!(
                "environment index {index} is out of bounds"
            )));
        }
        environments.remove(index);
        Ok(())
    })
}

pub(super) fn unbundled_rename_destination(
    persistence: &EnvironmentPersistence,
    original_name: &str,
    new_name: &str,
) -> Option<PathBuf> {
    if persistence.bundled_index.is_some() || original_name == new_name {
        return None;
    }
    let extension = persistence
        .document_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("yml");
    let destination = persistence
        .document_path
        .with_file_name(format!("{new_name}.{extension}"));
    (destination != persistence.document_path).then_some(destination)
}

pub(super) fn persist_unbundled_environment_rename(
    old_path: &Path,
    new_path: &Path,
    original_source: &[u8],
    replacement: &Environment,
) -> Result<(Vec<u8>, PathBuf), SaveError> {
    let _old_lock = SaveLock::acquire(old_path)?;
    let _new_lock = SaveLock::acquire(new_path)?;
    if new_path.exists() {
        return Err(SaveError::ConcurrentModification(new_path.to_owned()));
    }
    let current = fs::read(old_path).map_err(|source| SaveError::Io {
        path: old_path.to_owned(),
        source,
    })?;
    if current != original_source {
        return Err(SaveError::ConcurrentModification(old_path.to_owned()));
    }
    let mut document: Value = serde_yaml_ng::from_slice(original_source).map_err(|error| {
        SaveError::InvalidDocument(format!("retained source cannot be parsed: {error}"))
    })?;
    apply_environment_replace(&mut document, replacement)?;
    let serialized = serde_yaml_ng::to_string(&document)
        .map_err(SaveError::Serialize)?
        .into_bytes();
    write_new_environment_file(new_path, &serialized)?;
    if let Err(error) = remove_unbundled_environment_file(old_path) {
        let _ = fs::remove_file(new_path);
        return Err(error);
    }
    Ok((serialized, new_path.to_owned()))
}

pub(super) fn persist_unbundled_environment_delete(
    document_path: &Path,
    original_source: &[u8],
) -> Result<(), SaveError> {
    let _save_lock = SaveLock::acquire(document_path)?;
    let current = fs::read(document_path).map_err(|source| SaveError::Io {
        path: document_path.to_owned(),
        source,
    })?;
    if current != original_source {
        return Err(SaveError::ConcurrentModification(document_path.to_owned()));
    }
    remove_unbundled_environment_file(document_path)
}

pub(super) fn remove_unbundled_environment_file(document_path: &Path) -> Result<(), SaveError> {
    static NEXT_DELETE_ID: AtomicU64 = AtomicU64::new(0);
    let filename = document_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("environment.yml");
    let tombstone = loop {
        let id = NEXT_DELETE_ID.fetch_add(1, Ordering::Relaxed);
        let candidate = document_path.with_file_name(format!(
            ".{filename}.probe-delete-{}-{id}",
            std::process::id()
        ));
        if !candidate.exists() {
            break candidate;
        }
    };
    fs::rename(document_path, &tombstone).map_err(|source| SaveError::Io {
        path: document_path.to_owned(),
        source,
    })?;
    if let Err(source) = fs::remove_file(&tombstone) {
        let _ = fs::rename(&tombstone, document_path);
        return Err(SaveError::Io {
            path: tombstone,
            source,
        });
    }
    Ok(())
}

pub(super) fn write_new_environment_file(path: &Path, contents: &[u8]) -> Result<(), SaveError> {
    match write_new_file(path, contents) {
        Ok(()) => Ok(()),
        Err(NewFileError::AlreadyExists) => Err(SaveError::ConcurrentModification(path.to_owned())),
        Err(NewFileError::Io(source)) => Err(SaveError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

pub(super) fn environment_document_mut<'a>(
    document: &'a mut Value,
    persistence: &EnvironmentPersistence,
) -> Result<&'a mut Value, SaveError> {
    let Some(index) = persistence.bundled_index else {
        return Ok(document);
    };
    let mapping = document.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the collection document is not a mapping".to_owned())
    })?;
    let config = mapping_child(mapping, "config")?;
    let environments = config
        .get_mut(string_key("environments"))
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| {
            SaveError::InvalidDocument("collection config has no environments sequence".to_owned())
        })?;
    environments.get_mut(index).ok_or_else(|| {
        SaveError::InvalidDocument(format!("environment index {index} is out of bounds"))
    })
}

pub(super) fn apply_environment_mutation(
    environment: &mut Value,
    mutation: &EnvironmentYamlMutation,
) -> Result<(), SaveError> {
    match mutation {
        EnvironmentYamlMutation::Set { variable } => {
            apply_environment_variable_set(environment, variable)
        }
        EnvironmentYamlMutation::Unset { name } => {
            apply_environment_variable_unset(environment, name)
        }
        EnvironmentYamlMutation::Replace {
            environment: replacement,
        } => apply_environment_replace(environment, replacement),
    }
}

pub(super) fn environment_replacement_with_retained_secrets(
    original: &Environment,
    mut replacement: Environment,
) -> Result<Environment, SaveError> {
    validate_unique_variable_names(&replacement).map_err(SaveError::Environment)?;
    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    for variable in &original.variables {
        match variable {
            EnvironmentVariable::Secret(secret) => {
                if let Some(name) = secret.name.as_deref().filter(|name| !name.is_empty()) {
                    if replacement.variables.iter().any(|variable| {
                        matches!(
                            variable,
                            EnvironmentVariable::Plain(variable)
                                if variable.name.as_deref() == Some(name)
                        )
                    }) {
                        return Err(SaveError::Environment(
                            EnvironmentResolutionError::DuplicateVariable {
                                environment: replacement.name.clone(),
                                variable: name.to_owned(),
                            },
                        ));
                    }
                    seen.insert(name.to_owned());
                }
                merged.push(variable.clone());
            }
            EnvironmentVariable::Plain(plain) => {
                let Some(name) = plain.name.as_deref().filter(|name| !name.is_empty()) else {
                    merged.push(variable.clone());
                    continue;
                };
                let Some(updated) =
                    replacement
                        .variables
                        .iter()
                        .find_map(|variable| match variable {
                            EnvironmentVariable::Plain(variable)
                                if variable.name.as_deref() == Some(name) =>
                            {
                                Some(variable.clone())
                            }
                            _ => None,
                        })
                else {
                    continue;
                };
                seen.insert(name.to_owned());
                merged.push(EnvironmentVariable::Plain(updated));
            }
        }
    }
    for variable in replacement.variables {
        let EnvironmentVariable::Plain(plain) = &variable else {
            continue;
        };
        let Some(name) = plain.name.as_deref().filter(|name| !name.is_empty()) else {
            merged.push(variable);
            continue;
        };
        if seen.contains(name) {
            continue;
        }
        seen.insert(name.to_owned());
        merged.push(variable);
    }
    replacement.variables = merged;
    Ok(replacement)
}

pub(super) fn apply_environment_replace(
    environment: &mut Value,
    replacement: &Environment,
) -> Result<(), SaveError> {
    let mapping = environment.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the environment document is not a mapping".to_owned())
    })?;
    mapping.insert(string_key("name"), Value::String(replacement.name.clone()));
    match &replacement.extends {
        Some(parent) => {
            mapping.insert(string_key("extends"), Value::String(parent.clone()));
        }
        None => {
            mapping.remove(string_key("extends"));
        }
    }

    let variables = sequence_child(mapping, "variables")?;
    let plain = replacement
        .variables
        .iter()
        .filter_map(|variable| match variable {
            EnvironmentVariable::Plain(variable) => Some(variable),
            EnvironmentVariable::Secret(_) => None,
        })
        .collect::<Vec<_>>();
    let mut retained = Vec::new();
    for mut entry in std::mem::take(variables) {
        let Some(existing) = entry.as_mapping_mut() else {
            retained.push(entry);
            continue;
        };
        if yaml_bool_field(existing, "secret") == Some(true) {
            retained.push(entry);
            continue;
        }
        let Some(name) = yaml_string_field(existing, "name").map(str::to_owned) else {
            retained.push(entry);
            continue;
        };
        let Some(variable) = plain
            .iter()
            .find(|variable| variable.name.as_deref() == Some(name.as_str()))
        else {
            continue;
        };
        existing.insert(string_key("disabled"), Value::Bool(variable.disabled));
        merge_environment_variable_value(existing, variable);
        retained.push(entry);
    }
    for variable in plain {
        let name = variable.name.as_deref();
        let already_retained = retained.iter().any(|entry| {
            entry
                .as_mapping()
                .is_some_and(|entry| yaml_string_field(entry, "name") == name)
        });
        if !already_retained {
            retained.push(new_environment_variable_value(variable));
        }
    }
    *variables = retained;
    Ok(())
}

pub(super) fn apply_environment_variable_set(
    environment: &mut Value,
    variable: &Variable,
) -> Result<(), SaveError> {
    let name = variable.name.as_deref().ok_or_else(|| {
        SaveError::InvalidDocument("updated environment variable has no name".to_owned())
    })?;
    let mapping = environment.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the environment document is not a mapping".to_owned())
    })?;
    let variables = sequence_child(mapping, "variables")?;
    if let Some(existing) = variables.iter_mut().find_map(|entry| {
        let mapping = entry.as_mapping_mut()?;
        (yaml_string_field(mapping, "name") == Some(name)).then_some(mapping)
    }) {
        if yaml_bool_field(existing, "secret") == Some(true) {
            return Err(SaveError::InvalidDocument(format!(
                "cannot overwrite secret variable '{name}'"
            )));
        }
        existing.insert(string_key("disabled"), Value::Bool(variable.disabled));
        merge_environment_variable_value(existing, variable);
        return Ok(());
    }
    variables.push(new_environment_variable_value(variable));
    Ok(())
}

pub(super) fn apply_environment_variable_unset(
    environment: &mut Value,
    name: &str,
) -> Result<(), SaveError> {
    let mapping = environment.as_mapping_mut().ok_or_else(|| {
        SaveError::InvalidDocument("the environment document is not a mapping".to_owned())
    })?;
    let variables = sequence_child(mapping, "variables")?;
    let Some(index) = variables.iter().position(|entry| {
        entry
            .as_mapping()
            .is_some_and(|mapping| yaml_string_field(mapping, "name") == Some(name))
    }) else {
        return Err(SaveError::InvalidDocument(format!(
            "variable '{name}' is missing from the retained environment document"
        )));
    };
    if variables[index]
        .as_mapping()
        .is_some_and(|mapping| yaml_bool_field(mapping, "secret") == Some(true))
    {
        return Err(SaveError::InvalidDocument(format!(
            "cannot unset secret variable '{name}'"
        )));
    }
    variables.remove(index);
    Ok(())
}

pub(super) fn merge_environment_variable_value(
    existing: &mut serde_yaml_ng::Mapping,
    variable: &Variable,
) {
    match (&variable.value, existing.get_mut(string_key("value"))) {
        (Some(VariableValueSet::Variants(variants)), Some(Value::Sequence(existing_variants))) => {
            merge_variable_variants(existing_variants, variants);
        }
        (Some(value), Some(existing_value)) => {
            merge_yaml_variable_value(existing_value, value);
        }
        (Some(value), None) => {
            existing.insert(string_key("value"), variable_value_set_yaml(value));
        }
        (None, _) => {
            existing.remove(string_key("value"));
        }
    }
}

pub(super) fn merge_variable_variants(existing: &mut [Value], variants: &[VariableValueVariant]) {
    for (index, variant) in variants.iter().enumerate() {
        let Some(existing) = existing.get_mut(index).and_then(Value::as_mapping_mut) else {
            continue;
        };
        existing.insert(string_key("selected"), Value::Bool(variant.selected));
        match existing.get_mut(string_key("value")) {
            Some(existing_value) => {
                merge_yaml_variable_value(
                    existing_value,
                    &VariableValueSet::Single(variant.value.clone()),
                );
            }
            None => {
                existing.insert(string_key("value"), variable_value_yaml(&variant.value));
            }
        }
    }
}

pub(super) fn merge_yaml_variable_value(existing: &mut Value, value: &VariableValueSet) {
    match (existing, value) {
        (Value::Mapping(existing), VariableValueSet::Single(VariableValue::Typed { data, .. }))
            if existing.contains_key(string_key("data")) =>
        {
            existing.insert(string_key("data"), Value::String(data.clone()));
        }
        (Value::Mapping(existing), VariableValueSet::Single(VariableValue::String(data)))
            if existing.contains_key(string_key("data")) =>
        {
            existing.insert(string_key("data"), Value::String(data.clone()));
        }
        (existing, value) => *existing = variable_value_set_yaml(value),
    }
}

pub(super) fn new_environment_variable_value(variable: &Variable) -> Value {
    let mut mapping = serde_yaml_ng::Mapping::new();
    if let Some(name) = &variable.name {
        mapping.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(value) = &variable.value {
        mapping.insert(string_key("value"), variable_value_set_yaml(value));
    }
    if variable.disabled {
        mapping.insert(string_key("disabled"), Value::Bool(true));
    }
    Value::Mapping(mapping)
}

pub(super) fn variable_value_set_yaml(value: &VariableValueSet) -> Value {
    match value {
        VariableValueSet::Single(value) => variable_value_yaml(value),
        VariableValueSet::Variants(variants) => Value::Sequence(
            variants
                .iter()
                .map(|variant| {
                    map([
                        ("title", Value::String(variant.title.clone())),
                        ("selected", Value::Bool(variant.selected)),
                        ("value", variable_value_yaml(&variant.value)),
                    ])
                })
                .collect(),
        ),
    }
}

pub(super) fn variable_value_yaml(value: &VariableValue) -> Value {
    match value {
        VariableValue::String(value) => Value::String(value.clone()),
        VariableValue::Typed { kind, data } => map([
            ("type", Value::String(kind.as_str().to_owned())),
            ("data", Value::String(data.clone())),
        ]),
    }
}

pub(super) fn sequence_child<'a>(
    parent: &'a mut serde_yaml_ng::Mapping,
    name: &str,
) -> Result<&'a mut Vec<Value>, SaveError> {
    let key = string_key(name);
    if !parent.contains_key(&key) {
        parent.insert(key.clone(), Value::Sequence(Vec::new()));
    }
    parent
        .get_mut(&key)
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| SaveError::InvalidDocument(format!("'{name}' is not a sequence")))
}

pub(super) fn yaml_string_field<'a>(
    mapping: &'a serde_yaml_ng::Mapping,
    name: &str,
) -> Option<&'a str> {
    mapping.get(string_key(name)).and_then(Value::as_str)
}

pub(super) fn yaml_bool_field(mapping: &serde_yaml_ng::Mapping, name: &str) -> Option<bool> {
    mapping.get(string_key(name)).and_then(Value::as_bool)
}
