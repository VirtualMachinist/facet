use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{
    AuthenticationValue, Body, Environment, EnvironmentVariable, HttpRequest, MultipartValue,
    RequestBody, Variable, VariableValue, VariableValueSet,
};

/// An environment selected and resolved entirely in memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedEnvironment {
    name: String,
    variables: BTreeMap<String, String>,
    secrets_without_values: BTreeSet<String>,
}

impl ResolvedEnvironment {
    /// Returns the selected environment name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns all resolved, non-secret variables in deterministic name order.
    #[must_use]
    pub const fn variables(&self) -> &BTreeMap<String, String> {
        &self.variables
    }

    /// Returns secret variable names that have no runtime value.
    #[must_use]
    pub const fn secrets_without_values(&self) -> &BTreeSet<String> {
        &self.secrets_without_values
    }

    /// Looks up a resolved variable.
    #[must_use]
    pub fn variable(&self, name: &str) -> Option<&str> {
        self.variables.get(name).map(String::as_str)
    }

    /// Interpolates `{{variable}}` references in a string.
    pub fn interpolate(&self, input: &str) -> Result<String, EnvironmentResolutionError> {
        interpolate(input, |name| {
            if self.secrets_without_values.contains(name) {
                Err(EnvironmentResolutionError::SecretVariableUnavailable(
                    name.to_owned(),
                ))
            } else {
                self.variables
                    .get(name)
                    .cloned()
                    .ok_or_else(|| EnvironmentResolutionError::MissingVariable(name.to_owned()))
            }
        })
    }
}

/// A deterministic environment-selection or interpolation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvironmentResolutionError {
    /// The selected environment does not exist.
    EnvironmentNotFound(String),
    /// Two environment documents use the same name.
    DuplicateEnvironment(String),
    /// An environment extends a parent that does not exist.
    ParentEnvironmentNotFound {
        /// Child environment name.
        environment: String,
        /// Missing parent environment name.
        parent: String,
    },
    /// Environment inheritance contains a cycle.
    EnvironmentInheritanceCycle(Vec<String>),
    /// A referenced variable is absent, disabled, or has no value.
    MissingVariable(String),
    /// A referenced secret has no runtime value provider.
    SecretVariableUnavailable(String),
    /// A variable value has no selected variant.
    NoSelectedVariant {
        /// Environment containing the variable.
        environment: String,
        /// Variable name.
        variable: String,
    },
    /// A variable value has more than one selected variant.
    MultipleSelectedVariants {
        /// Environment containing the variable.
        environment: String,
        /// Variable name.
        variable: String,
    },
    /// Variable values refer to each other cyclically.
    VariableInterpolationCycle(Vec<String>),
    /// An interpolation starts with `{{` but has no valid closing expression.
    MalformedInterpolation,
    /// An environment name is empty or otherwise unusable for creation.
    InvalidEnvironmentName,
    /// A variable name is empty or otherwise unusable for set/unset.
    InvalidVariableName,
    /// The named environment has no entry for this variable.
    VariableNotFound {
        /// Environment that was asked to unset the variable.
        environment: String,
        /// Variable name.
        variable: String,
    },
    /// Two variables in the same environment share a name, including across plain and secret kinds.
    DuplicateVariable {
        /// Environment containing the colliding names.
        environment: String,
        /// Variable name that appears more than once.
        variable: String,
    },
    /// The environment is used as another environment's parent.
    EnvironmentInUse(String),
}

/// A plain environment variable as it appears after inheritance, together with its source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveEnvironmentVariable {
    /// Plain variable definition that currently wins for this name.
    pub variable: Variable,
    /// Environment that defines this effective value.
    pub defined_in: String,
    /// Index in the selected environment's variable list when this entry is local.
    pub direct_index: Option<usize>,
}

/// Validates environment identity and inheritance independently of variable use.
///
/// This is suitable for workspace validation because it does not require selecting an
/// environment or resolving variables that a request may never reference.
pub fn validate_environments(
    environments: &[Environment],
) -> Result<(), EnvironmentResolutionError> {
    let mut by_name = BTreeMap::new();
    for environment in environments {
        if by_name
            .insert(environment.name.as_str(), environment)
            .is_some()
        {
            return Err(EnvironmentResolutionError::DuplicateEnvironment(
                environment.name.clone(),
            ));
        }
    }

    let mut validated = BTreeSet::new();
    for environment in environments {
        validate_environment_inheritance(
            &environment.name,
            &by_name,
            &mut Vec::new(),
            &mut validated,
        )?;
    }
    Ok(())
}

fn validate_environment_inheritance<'a>(
    name: &'a str,
    environments: &BTreeMap<&'a str, &'a Environment>,
    stack: &mut Vec<String>,
    validated: &mut BTreeSet<String>,
) -> Result<(), EnvironmentResolutionError> {
    if validated.contains(name) {
        return Ok(());
    }
    if let Some(position) = stack.iter().position(|item| item == name) {
        let mut cycle = stack[position..].to_vec();
        cycle.push(name.to_owned());
        return Err(EnvironmentResolutionError::EnvironmentInheritanceCycle(
            cycle,
        ));
    }

    let environment = environments
        .get(name)
        .expect("environment name must exist during validation");
    stack.push(name.to_owned());
    if let Some(parent) = environment.extends.as_deref() {
        if !environments.contains_key(parent) {
            return Err(EnvironmentResolutionError::ParentEnvironmentNotFound {
                environment: environment.name.clone(),
                parent: parent.to_owned(),
            });
        }
        validate_environment_inheritance(parent, environments, stack, validated)?;
    }
    stack.pop();
    validated.insert(name.to_owned());
    Ok(())
}

impl fmt::Display for EnvironmentResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvironmentNotFound(name) => write!(formatter, "environment not found: {name}"),
            Self::DuplicateEnvironment(name) => {
                write!(formatter, "environment name is duplicated: {name}")
            }
            Self::ParentEnvironmentNotFound {
                environment,
                parent,
            } => write!(
                formatter,
                "environment '{environment}' extends missing environment '{parent}'"
            ),
            Self::EnvironmentInheritanceCycle(names) => write!(
                formatter,
                "environment inheritance cycle: {}",
                names.join(" -> ")
            ),
            Self::MissingVariable(name) => write!(formatter, "variable not found: {name}"),
            Self::SecretVariableUnavailable(name) => {
                write!(formatter, "secret variable has no runtime value: {name}")
            }
            Self::NoSelectedVariant {
                environment,
                variable,
            } => write!(
                formatter,
                "variable '{variable}' in environment '{environment}' has no selected variant"
            ),
            Self::MultipleSelectedVariants {
                environment,
                variable,
            } => write!(
                formatter,
                "variable '{variable}' in environment '{environment}' has multiple selected variants"
            ),
            Self::VariableInterpolationCycle(names) => {
                write!(
                    formatter,
                    "variable interpolation cycle: {}",
                    names.join(" -> ")
                )
            }
            Self::MalformedInterpolation => write!(formatter, "malformed variable interpolation"),
            Self::InvalidEnvironmentName => {
                formatter.write_str("environment name must not be empty")
            }
            Self::InvalidVariableName => formatter.write_str("variable name must not be empty"),
            Self::VariableNotFound {
                environment,
                variable,
            } => write!(
                formatter,
                "variable '{variable}' is not defined on environment '{environment}'"
            ),
            Self::DuplicateVariable {
                environment,
                variable,
            } => write!(
                formatter,
                "environment '{environment}' has duplicate variable '{variable}'"
            ),
            Self::EnvironmentInUse(name) => write!(
                formatter,
                "environment '{name}' is extended by another environment"
            ),
        }
    }
}

impl Error for EnvironmentResolutionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RawVariable {
    Value(String),
    Unavailable,
    Secret,
}

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
    environments.push(Environment {
        name,
        color: None,
        extends,
        dot_env_file_path: None,
        variables: Vec::new(),
    });
    validate_environments(environments)
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

/// Selects an environment, applies its inheritance chain, and resolves variable values.
pub fn resolve_environment(
    environments: &[Environment],
    selected: &str,
) -> Result<ResolvedEnvironment, EnvironmentResolutionError> {
    let raw = raw_variables(environments, selected)?;

    let mut variables = BTreeMap::new();
    let mut resolving = Vec::new();
    for name in raw.keys() {
        if matches!(raw.get(name), Some(RawVariable::Value(_))) {
            resolve_variable(name, &raw, &mut variables, &mut resolving)?;
        }
    }

    let secrets_without_values = raw
        .iter()
        .filter_map(|(name, value)| matches!(value, RawVariable::Secret).then_some(name.clone()))
        .collect();
    Ok(ResolvedEnvironment {
        name: selected.to_owned(),
        variables,
        secrets_without_values,
    })
}

fn raw_variables(
    environments: &[Environment],
    selected: &str,
) -> Result<BTreeMap<String, RawVariable>, EnvironmentResolutionError> {
    validate_environments(environments)?;
    let mut by_name = BTreeMap::new();
    for environment in environments {
        by_name.insert(environment.name.as_str(), environment);
    }
    if !by_name.contains_key(selected) {
        return Err(EnvironmentResolutionError::EnvironmentNotFound(
            selected.to_owned(),
        ));
    }

    let mut raw = BTreeMap::new();
    apply_environment(
        selected,
        &by_name,
        &mut Vec::new(),
        &mut BTreeSet::new(),
        &mut raw,
    )?;
    Ok(raw)
}

fn apply_environment<'a>(
    name: &'a str,
    environments: &BTreeMap<&'a str, &'a Environment>,
    stack: &mut Vec<String>,
    applied: &mut BTreeSet<String>,
    variables: &mut BTreeMap<String, RawVariable>,
) -> Result<(), EnvironmentResolutionError> {
    if applied.contains(name) {
        return Ok(());
    }
    if let Some(position) = stack.iter().position(|item| item == name) {
        let mut cycle = stack[position..].to_vec();
        cycle.push(name.to_owned());
        return Err(EnvironmentResolutionError::EnvironmentInheritanceCycle(
            cycle,
        ));
    }

    let environment = environments
        .get(name)
        .expect("environment names are checked before inheritance traversal");
    stack.push(name.to_owned());
    if let Some(parent) = environment.extends.as_deref() {
        if !environments.contains_key(parent) {
            return Err(EnvironmentResolutionError::ParentEnvironmentNotFound {
                environment: environment.name.clone(),
                parent: parent.to_owned(),
            });
        }
        apply_environment(parent, environments, stack, applied, variables)?;
    }

    for variable in &environment.variables {
        match variable {
            EnvironmentVariable::Plain(variable) => {
                let Some(name) = variable.name.as_ref() else {
                    continue;
                };
                let value = if variable.disabled {
                    RawVariable::Unavailable
                } else if let Some(value) = variable.value.as_ref() {
                    RawVariable::Value(select_value(value, &environment.name, name)?)
                } else {
                    RawVariable::Unavailable
                };
                variables.insert(name.clone(), value);
            }
            EnvironmentVariable::Secret(variable) => {
                let Some(name) = variable.name.as_ref() else {
                    continue;
                };
                variables.insert(
                    name.clone(),
                    if variable.disabled {
                        RawVariable::Unavailable
                    } else {
                        RawVariable::Secret
                    },
                );
            }
        }
    }

    stack.pop();
    applied.insert(name.to_owned());
    Ok(())
}

fn select_value(
    value: &VariableValueSet,
    environment: &str,
    variable: &str,
) -> Result<String, EnvironmentResolutionError> {
    let value = match value {
        VariableValueSet::Single(value) => value,
        VariableValueSet::Variants(variants) => {
            let mut selected = variants.iter().filter(|variant| variant.selected);
            let value =
                selected
                    .next()
                    .ok_or_else(|| EnvironmentResolutionError::NoSelectedVariant {
                        environment: environment.to_owned(),
                        variable: variable.to_owned(),
                    })?;
            if selected.next().is_some() {
                return Err(EnvironmentResolutionError::MultipleSelectedVariants {
                    environment: environment.to_owned(),
                    variable: variable.to_owned(),
                });
            }
            &value.value
        }
    };
    Ok(match value {
        VariableValue::String(value) | VariableValue::Typed { data: value, .. } => value.clone(),
    })
}

fn resolve_variable(
    name: &str,
    raw: &BTreeMap<String, RawVariable>,
    resolved: &mut BTreeMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<String, EnvironmentResolutionError> {
    if let Some(value) = resolved.get(name) {
        return Ok(value.clone());
    }
    if let Some(position) = stack.iter().position(|item| item == name) {
        let mut cycle = stack[position..].to_vec();
        cycle.push(name.to_owned());
        return Err(EnvironmentResolutionError::VariableInterpolationCycle(
            cycle,
        ));
    }

    let value = match raw.get(name) {
        Some(RawVariable::Value(value)) => value,
        Some(RawVariable::Secret) => {
            return Err(EnvironmentResolutionError::SecretVariableUnavailable(
                name.to_owned(),
            ));
        }
        Some(RawVariable::Unavailable) | None => {
            return Err(EnvironmentResolutionError::MissingVariable(name.to_owned()));
        }
    };
    stack.push(name.to_owned());
    let value = interpolate(value, |reference| {
        resolve_variable(reference, raw, resolved, stack)
    })?;
    stack.pop();
    resolved.insert(name.to_owned(), value.clone());
    Ok(value)
}

fn interpolate<F>(input: &str, mut lookup: F) -> Result<String, EnvironmentResolutionError>
where
    F: FnMut(&str) -> Result<String, EnvironmentResolutionError>,
{
    let mut output = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let Some(end) = expression.find("}}") else {
            return Err(EnvironmentResolutionError::MalformedInterpolation);
        };
        let name = expression[..end].trim();
        if name.is_empty() || name.contains("{{") {
            return Err(EnvironmentResolutionError::MalformedInterpolation);
        }
        output.push_str(&lookup(name)?);
        remaining = &expression[end + 2..];
    }
    output.push_str(remaining);
    Ok(output)
}

/// Clones a request and interpolates every currently supported request-value field.
pub fn resolve_request(
    request: &HttpRequest,
    environment: &ResolvedEnvironment,
) -> Result<HttpRequest, EnvironmentResolutionError> {
    let mut request = request.clone();
    interpolate_optional(&mut request.method, environment)?;
    interpolate_optional(&mut request.url, environment)?;
    for header in &mut request.headers {
        header.name = environment.interpolate(&header.name)?;
        header.value = environment.interpolate(&header.value)?;
    }
    for parameter in &mut request.query_parameters {
        parameter.name = environment.interpolate(&parameter.name)?;
        parameter.value = environment.interpolate(&parameter.value)?;
    }
    for parameter in &mut request.path_parameters {
        parameter.name = environment.interpolate(&parameter.name)?;
        parameter.value = environment.interpolate(&parameter.value)?;
    }
    if let Some(body) = &mut request.body {
        resolve_body(body, environment)?;
    }
    if let Some(authentication) = &mut request.authentication {
        for value in authentication.properties.values_mut() {
            resolve_authentication_value(value, environment)?;
        }
    }
    Ok(request)
}

fn interpolate_optional(
    value: &mut Option<String>,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    if let Some(value) = value {
        *value = environment.interpolate(value)?;
    }
    Ok(())
}

fn resolve_body(
    body: &mut RequestBody,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match body {
        RequestBody::Single(body) => resolve_body_value(body, environment),
        RequestBody::Variants(variants) => {
            for variant in variants {
                resolve_body_value(&mut variant.body, environment)?;
            }
            Ok(())
        }
    }
}

fn resolve_body_value(
    body: &mut Body,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match body {
        Body::Raw(body) => body.data = environment.interpolate(&body.data)?,
        Body::FormUrlEncoded(fields) => {
            for field in fields {
                field.name = environment.interpolate(&field.name)?;
                field.value = environment.interpolate(&field.value)?;
            }
        }
        Body::Multipart(parts) => {
            for part in parts {
                part.name = environment.interpolate(&part.name)?;
                match &mut part.value {
                    MultipartValue::Single(value) => *value = environment.interpolate(value)?,
                    MultipartValue::Multiple(values) => {
                        for value in values {
                            *value = environment.interpolate(value)?;
                        }
                    }
                }
                interpolate_optional(&mut part.content_type, environment)?;
            }
        }
        Body::File(files) => {
            for file in files {
                file.file_path = environment.interpolate(&file.file_path)?;
                file.content_type = environment.interpolate(&file.content_type)?;
            }
        }
    }
    Ok(())
}

fn resolve_authentication_value(
    value: &mut AuthenticationValue,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match value {
        AuthenticationValue::String(value) | AuthenticationValue::Number(value) => {
            *value = environment.interpolate(value)?;
        }
        AuthenticationValue::Sequence(values) => {
            for value in values {
                resolve_authentication_value(value, environment)?;
            }
        }
        AuthenticationValue::Object(values) => {
            for value in values.values_mut() {
                resolve_authentication_value(value, environment)?;
            }
        }
        AuthenticationValue::Boolean(_) | AuthenticationValue::Null => {}
    }
    Ok(())
}
