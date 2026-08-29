use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{Environment, EnvironmentVariable, Variable, VariableValue, VariableValueSet};

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
    EnvironmentIndex::new(environments).map(|_| ())
}

struct EnvironmentIndex<'a> {
    by_name: BTreeMap<&'a str, &'a Environment>,
}

impl<'a> EnvironmentIndex<'a> {
    fn new(environments: &'a [Environment]) -> Result<Self, EnvironmentResolutionError> {
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
        Ok(Self { by_name })
    }

    fn get(&self, name: &str) -> Option<&'a Environment> {
        self.by_name.get(name).copied()
    }

    fn inheritance_chain(
        &self,
        selected: &str,
    ) -> Result<Vec<&'a Environment>, EnvironmentResolutionError> {
        let mut environment = self
            .get(selected)
            .ok_or_else(|| EnvironmentResolutionError::EnvironmentNotFound(selected.to_owned()))?;
        let mut chain = Vec::new();
        loop {
            chain.push(environment);
            let Some(parent) = environment.extends.as_deref() else {
                break;
            };
            environment = self
                .get(parent)
                .expect("validated parent environment must exist");
        }
        chain.reverse();
        Ok(chain)
    }
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
pub(crate) enum RawVariable {
    Value(String),
    Unavailable,
    Secret,
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

pub(crate) fn raw_variables(
    environments: &[Environment],
    selected: &str,
) -> Result<BTreeMap<String, RawVariable>, EnvironmentResolutionError> {
    let index = EnvironmentIndex::new(environments)?;
    let mut raw = BTreeMap::new();
    for environment in index.inheritance_chain(selected)? {
        for variable in &environment.variables {
            match variable {
                EnvironmentVariable::Plain(variable) => {
                    let Some(name) = variable.name.as_ref() else {
                        continue;
                    };
                    let value = match (&variable.value, variable.disabled) {
                        (_, true) | (None, false) => RawVariable::Unavailable,
                        (Some(value), false) => {
                            RawVariable::Value(select_value(value, &environment.name, name)?)
                        }
                    };
                    raw.insert(name.clone(), value);
                }
                EnvironmentVariable::Secret(variable) => {
                    let Some(name) = variable.name.as_ref() else {
                        continue;
                    };
                    raw.insert(
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
    }
    Ok(raw)
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
