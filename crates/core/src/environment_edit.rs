//! Environment creation, mutation, and effective-variable presentation.

use std::collections::BTreeSet;

use crate::{
    EffectiveEnvironmentVariable, Environment, EnvironmentResolutionError, EnvironmentVariable,
    Variable, VariableValue, VariableValueSet,
    environment::{RawVariable, raw_variables},
    validate_environments,
};

/// Creates a new environment with an optional parent.
///
/// The new environment starts with no variables. Validation rejects duplicate names,
/// missing parents, and inheritance cycles.
pub fn create_environment(
    environments: &mut Vec<Environment>,
    name: String,
    extends: Option<String>,
) -> Result<(), EnvironmentResolutionError> {
    if name.is_empty() {
        return Err(EnvironmentResolutionError::InvalidEnvironmentName);
    }
    if environments
        .iter()
        .any(|environment| environment.name == name)
    {
        return Err(EnvironmentResolutionError::DuplicateEnvironment(name));
    }
    if let Some(parent) = extends.as_deref()
        && !environments
            .iter()
            .any(|environment| environment.name == parent)
    {
        return Err(EnvironmentResolutionError::ParentEnvironmentNotFound {
            environment: name.clone(),
            parent: parent.to_owned(),
        });
    }
    let mut candidate = environments.clone();
    candidate.push(Environment {
        name,
        color: None,
        extends,
        dot_env_file_path: None,
        variables: Vec::new(),
    });
    validate_environments(&candidate)?;
    *environments = candidate;
    Ok(())
}

/// Removes an environment created in this session if it still has no children.
pub fn revert_created_environment(environments: &mut Vec<Environment>, name: &str) {
    if environments
        .iter()
        .any(|environment| environment.extends.as_deref() == Some(name))
    {
        return;
    }
    if let Some(index) = environments
        .iter()
        .position(|environment| environment.name == name)
    {
        environments.remove(index);
    }
}

/// Replaces one environment and revalidates the complete inheritance graph.
pub fn replace_environment(
    environments: &mut [Environment],
    original_name: &str,
    replacement: Environment,
) -> Result<(), EnvironmentResolutionError> {
    if replacement.name.is_empty() {
        return Err(EnvironmentResolutionError::InvalidEnvironmentName);
    }
    let Some(index) = environments
        .iter()
        .position(|environment| environment.name == original_name)
    else {
        return Err(EnvironmentResolutionError::EnvironmentNotFound(
            original_name.to_owned(),
        ));
    };
    if replacement.name != original_name
        && environments
            .iter()
            .any(|environment| environment.name == replacement.name)
    {
        return Err(EnvironmentResolutionError::DuplicateEnvironment(
            replacement.name,
        ));
    }
    validate_unique_variable_names(&replacement)?;

    let mut candidate = environments.to_vec();
    let renamed = replacement.name.clone();
    candidate[index] = replacement;
    if renamed != original_name {
        for environment in &mut candidate {
            if environment.extends.as_deref() == Some(original_name) {
                environment.extends = Some(renamed.clone());
            }
        }
    }
    validate_environments(&candidate)?;
    environments.clone_from_slice(&candidate);
    Ok(())
}

/// Deletes an environment that is not used as another environment's parent.
pub fn delete_environment(
    environments: &mut Vec<Environment>,
    name: &str,
) -> Result<Environment, EnvironmentResolutionError> {
    if environments
        .iter()
        .any(|environment| environment.extends.as_deref() == Some(name))
    {
        return Err(EnvironmentResolutionError::EnvironmentInUse(
            name.to_owned(),
        ));
    }
    let Some(index) = environments
        .iter()
        .position(|environment| environment.name == name)
    else {
        return Err(EnvironmentResolutionError::EnvironmentNotFound(
            name.to_owned(),
        ));
    };
    Ok(environments.remove(index))
}

/// Updates a plain variable on the selected environment, or adds an override.
///
/// Secret variables cannot be written. A variable defined only on a parent is
/// added to the selected environment so the parent document is left unchanged.
pub fn set_environment_variable(
    environments: &mut [Environment],
    environment_name: &str,
    variable_name: &str,
    value: String,
) -> Result<(), EnvironmentResolutionError> {
    if variable_name.is_empty() {
        return Err(EnvironmentResolutionError::InvalidVariableName);
    }
    let raw = raw_variables(environments, environment_name)?;
    if matches!(raw.get(variable_name), Some(RawVariable::Secret)) {
        return Err(EnvironmentResolutionError::SecretVariableUnavailable(
            variable_name.to_owned(),
        ));
    }
    let environment = named_environment_mut(environments, environment_name)?;
    for variable in &mut environment.variables {
        let EnvironmentVariable::Plain(variable) = variable else {
            continue;
        };
        if variable.name.as_deref() != Some(variable_name) {
            continue;
        }
        variable.disabled = false;
        assign_variable_value(&mut variable.value, value);
        return Ok(());
    }

    environment
        .variables
        .push(EnvironmentVariable::Plain(Variable {
            name: Some(variable_name.to_owned()),
            value: Some(VariableValueSet::Single(VariableValue::String(value))),
            disabled: false,
        }));
    Ok(())
}

/// Removes a plain variable from the named environment so a parent value can show through.
///
/// Only that environment's entry is deleted. Parent variables are left unchanged.
/// Secrets are not converted into plain values or removed.
pub fn unset_environment_variable(
    environments: &mut [Environment],
    environment_name: &str,
    variable_name: &str,
) -> Result<(), EnvironmentResolutionError> {
    if variable_name.is_empty() {
        return Err(EnvironmentResolutionError::InvalidVariableName);
    }
    let environment = named_environment_mut(environments, environment_name)?;
    let Some(index) = environment
        .variables
        .iter()
        .position(|variable| variable_entry_name(variable) == Some(variable_name))
    else {
        return Err(EnvironmentResolutionError::VariableNotFound {
            environment: environment_name.to_owned(),
            variable: variable_name.to_owned(),
        });
    };
    if matches!(environment.variables[index], EnvironmentVariable::Secret(_)) {
        return Err(EnvironmentResolutionError::SecretVariableUnavailable(
            variable_name.to_owned(),
        ));
    }
    environment.variables.remove(index);
    Ok(())
}

/// Rejects duplicate variable names across plain and secret entries in one environment.
pub fn validate_unique_variable_names(
    environment: &Environment,
) -> Result<(), EnvironmentResolutionError> {
    let mut names = BTreeSet::new();
    for variable in &environment.variables {
        let Some(name) = variable_entry_name(variable) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if !names.insert(name) {
            return Err(EnvironmentResolutionError::DuplicateVariable {
                environment: environment.name.clone(),
                variable: name.to_owned(),
            });
        }
    }
    Ok(())
}

/// Returns effective plain variables for `selected`, including inherited values.
///
/// Child entries override parents by name. Disabled variables remain visible. Secrets
/// are omitted from the result but still shadow inherited plains with the same name.
#[must_use]
pub fn effective_environment_variables(
    environments: &[Environment],
    selected: &Environment,
) -> Vec<EffectiveEnvironmentVariable> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, variable) in selected.variables.iter().enumerate() {
        match variable {
            EnvironmentVariable::Plain(variable) => {
                if let Some(name) = &variable.name {
                    seen.insert(name.clone());
                }
                rows.push(EffectiveEnvironmentVariable {
                    variable: variable.clone(),
                    defined_in: selected.name.clone(),
                    direct_index: Some(index),
                });
            }
            EnvironmentVariable::Secret(variable) => {
                if let Some(name) = &variable.name {
                    seen.insert(name.clone());
                }
            }
        }
    }

    let mut visited_parents = BTreeSet::new();
    let mut parent = selected.extends.as_deref();
    while let Some(parent_name) = parent {
        if !visited_parents.insert(parent_name.to_owned()) {
            break;
        }
        let Some(environment) = environments
            .iter()
            .find(|environment| environment.name == parent_name)
        else {
            break;
        };
        for variable in &environment.variables {
            let Some(name) = variable_entry_name(variable) else {
                continue;
            };
            if !seen.insert(name.to_owned()) {
                continue;
            }
            if let EnvironmentVariable::Plain(variable) = variable {
                rows.push(EffectiveEnvironmentVariable {
                    variable: variable.clone(),
                    defined_in: environment.name.clone(),
                    direct_index: None,
                });
            }
        }
        parent = environment.extends.as_deref();
    }
    rows
}

fn named_environment_mut<'a>(
    environments: &'a mut [Environment],
    environment_name: &str,
) -> Result<&'a mut Environment, EnvironmentResolutionError> {
    environments
        .iter_mut()
        .find(|environment| environment.name == environment_name)
        .ok_or_else(|| EnvironmentResolutionError::EnvironmentNotFound(environment_name.to_owned()))
}

fn variable_entry_name(variable: &EnvironmentVariable) -> Option<&str> {
    match variable {
        EnvironmentVariable::Plain(variable) => variable.name.as_deref(),
        EnvironmentVariable::Secret(variable) => variable.name.as_deref(),
    }
}

fn assign_variable_value(slot: &mut Option<VariableValueSet>, value: String) {
    match slot {
        Some(VariableValueSet::Single(VariableValue::String(existing))) => *existing = value,
        Some(VariableValueSet::Single(VariableValue::Typed { data, .. })) => *data = value,
        Some(VariableValueSet::Variants(variants)) => {
            if let Some(selected) = variants.iter_mut().find(|variant| variant.selected) {
                match &mut selected.value {
                    VariableValue::String(existing) => *existing = value,
                    VariableValue::Typed { data, .. } => *data = value,
                }
            } else if let Some(first) = variants.first_mut() {
                first.selected = true;
                match &mut first.value {
                    VariableValue::String(existing) => *existing = value,
                    VariableValue::Typed { data, .. } => *data = value,
                }
            } else {
                *slot = Some(VariableValueSet::Single(VariableValue::String(value)));
            }
        }
        None => *slot = Some(VariableValueSet::Single(VariableValue::String(value))),
    }
}
