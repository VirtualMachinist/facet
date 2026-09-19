//! Runtime secret-provider hook.
//!
//! OpenCollection secret variables store refs only (`secret://…`, `provider:key`,
//! or process environment names). A [`SecretProvider`] resolves those refs at run
//! time. Implementations must never write live values back into collection YAML.

use std::{collections::BTreeMap, fmt};

/// A secret lookup declared by name or structured ref.
///
/// Recipes may use a bare environment-variable name, `provider:key`, or
/// `secret://provider/key`. The original text is the only form used in errors
/// and redacted output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretReference {
    raw: String,
    provider: Option<String>,
    key: String,
}

impl SecretReference {
    /// Parses a secret variable name or stored ref.
    ///
    /// `secret://provider/key` and `provider:key` select a backend; a bare name
    /// is an environment-variable key for the default (`env`) backend.
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        if let Some(rest) = raw.strip_prefix("secret://") {
            if let Some((provider, key)) = rest.split_once('/')
                && is_provider_name(provider)
                && !key.is_empty()
            {
                return Self {
                    raw: raw.to_owned(),
                    provider: Some(provider.to_owned()),
                    key: key.to_owned(),
                };
            }
            return Self {
                raw: raw.to_owned(),
                provider: None,
                key: rest.to_owned(),
            };
        }
        if let Some((provider, key)) = raw.split_once(':')
            && is_provider_name(provider)
            && !key.is_empty()
            && !key.starts_with('/')
        {
            return Self {
                raw: raw.to_owned(),
                provider: Some(provider.to_owned()),
                key: key.to_owned(),
            };
        }
        Self {
            raw: raw.to_owned(),
            provider: None,
            key: raw.to_owned(),
        }
    }

    /// Original ref or OpenCollection variable name.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Optional backend identifier (`env`, `vault`, …).
    #[must_use]
    pub fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    /// Lookup key after stripping a provider prefix.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

fn is_provider_name(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic())
        && characters
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

/// Resolves secret refs at run time. Missing keys must fail closed.
///
/// Implementations return `None` when the ref is unknown or the backend does
/// not handle it. Callers map that to `secret_variable_unavailable`. Returned
/// values are used only to construct the outbound HTTP request.
pub trait SecretProvider {
    /// Looks up one secret. Empty values are treated as missing by the resolver.
    fn resolve_secret(&self, reference: &SecretReference) -> Option<String>;
}

/// Process-environment backend for CI and other ephemeral runners.
///
/// Accepts bare env names and refs whose provider is `env`. Other provider ids
/// fail closed so a vault or keychain ref is never silently read from the
/// process environment. Probe does not load `.env` files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EnvSecretProvider;

impl SecretProvider for EnvSecretProvider {
    fn resolve_secret(&self, reference: &SecretReference) -> Option<String> {
        match reference.provider() {
            None | Some("env") if !reference.key().is_empty() => std::env::var(reference.key())
                .ok()
                .filter(|value| !value.is_empty()),
            _ => None,
        }
    }
}

/// In-memory provider for tests and programmatic callers.
///
/// Values are looked up by the original ref or by the parsed key. Debug output
/// lists refs only.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct MapSecretProvider {
    values: BTreeMap<String, String>,
}

impl MapSecretProvider {
    /// Creates an empty provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret value under a ref or key.
    pub fn insert(&mut self, reference: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.values.insert(reference.into(), value.into());
        self
    }
}

impl SecretProvider for MapSecretProvider {
    fn resolve_secret(&self, reference: &SecretReference) -> Option<String> {
        self.values
            .get(reference.raw())
            .or_else(|| self.values.get(reference.key()))
            .cloned()
            .filter(|value| !value.is_empty())
    }
}

impl fmt::Debug for MapSecretProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MapSecretProvider")
            .field("refs", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvSecretProvider, MapSecretProvider, SecretProvider, SecretReference};

    #[test]
    fn parses_env_names_and_structured_refs() {
        let bare = SecretReference::parse("API_TOKEN");
        assert_eq!(bare.raw(), "API_TOKEN");
        assert_eq!(bare.provider(), None);
        assert_eq!(bare.key(), "API_TOKEN");

        let prefixed = SecretReference::parse("env:API_TOKEN");
        assert_eq!(prefixed.provider(), Some("env"));
        assert_eq!(prefixed.key(), "API_TOKEN");

        let uri = SecretReference::parse("secret://vault/prod/token");
        assert_eq!(uri.provider(), Some("vault"));
        assert_eq!(uri.key(), "prod/token");

        let uri_env = SecretReference::parse("secret://env/API_TOKEN");
        assert_eq!(uri_env.provider(), Some("env"));
        assert_eq!(uri_env.key(), "API_TOKEN");

        let uri_bare = SecretReference::parse("secret://API_TOKEN");
        assert_eq!(uri_bare.provider(), None);
        assert_eq!(uri_bare.key(), "API_TOKEN");
    }

    #[test]
    fn does_not_treat_urls_as_provider_keys() {
        let url = SecretReference::parse("https://example.com");
        assert_eq!(url.provider(), None);
        assert_eq!(url.key(), "https://example.com");
    }

    #[test]
    fn map_provider_resolves_by_ref_or_key_and_hides_values_in_debug() {
        let mut provider = MapSecretProvider::new();
        provider
            .insert("secretToken", "live-token-value")
            .insert("API_TOKEN", "from-key");

        assert_eq!(
            provider
                .resolve_secret(&SecretReference::parse("secretToken"))
                .as_deref(),
            Some("live-token-value")
        );
        assert_eq!(
            provider
                .resolve_secret(&SecretReference::parse("env:API_TOKEN"))
                .as_deref(),
            Some("from-key")
        );
        assert_eq!(
            provider.resolve_secret(&SecretReference::parse("missing")),
            None
        );

        let rendered = format!("{provider:?}");
        assert!(rendered.contains("secretToken"));
        assert!(!rendered.contains("live-token-value"));
        assert!(!rendered.contains("from-key"));
    }

    #[test]
    fn env_provider_rejects_non_env_backends() {
        assert_eq!(
            EnvSecretProvider.resolve_secret(&SecretReference::parse("vault:prod/token")),
            None
        );
        assert_eq!(
            EnvSecretProvider.resolve_secret(&SecretReference::parse("secret://vault/prod/token")),
            None
        );
    }
}
