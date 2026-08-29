//! Serialization-independent environment models.

/// A bundled OpenCollection environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Environment {
    /// Environment name.
    pub name: String,
    /// Optional display color.
    pub color: Option<String>,
    /// Parent environment name.
    pub extends: Option<String>,
    /// Optional dotenv file path.
    pub dot_env_file_path: Option<String>,
    /// Environment variables.
    pub variables: Vec<EnvironmentVariable>,
}

/// A normal or secret environment variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvironmentVariable {
    /// A value stored in the collection.
    Plain(Variable),
    /// A secret stored outside the collection.
    Secret(SecretVariable),
}

/// A non-secret environment variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variable {
    /// Variable name.
    pub name: Option<String>,
    /// Variable value or selectable values.
    pub value: Option<VariableValueSet>,
    /// Whether the variable is disabled.
    pub disabled: bool,
}

/// A secret environment-variable declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretVariable {
    /// Variable name.
    pub name: Option<String>,
    /// Declared value type.
    pub value_type: Option<VariableValueType>,
    /// Whether the variable is disabled.
    pub disabled: bool,
}

/// A single variable value or selectable variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValueSet {
    /// One value.
    Single(VariableValue),
    /// Named selectable values.
    Variants(Vec<VariableValueVariant>),
}

/// A named variable-value variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableValueVariant {
    /// Variant title.
    pub title: String,
    /// Whether the variant is selected.
    pub selected: bool,
    /// Variant value.
    pub value: VariableValue,
}

/// An environment variable value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValue {
    /// The shorthand string form.
    String(String),
    /// An explicitly typed value retained as string data.
    Typed {
        /// Declared value type.
        kind: VariableValueType,
        /// String representation of the value.
        data: String,
    },
}

/// Types available for explicitly typed and secret variables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VariableValueType {
    /// String data.
    String,
    /// Numeric data.
    Number,
    /// Boolean data.
    Boolean,
    /// Null data.
    Null,
    /// Object data.
    Object,
}

impl VariableValueType {
    /// Returns the OpenCollection type name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Null => "null",
            Self::Object => "object",
        }
    }
}
