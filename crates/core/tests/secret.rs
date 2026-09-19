use std::collections::BTreeMap;

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Environment,
    EnvironmentResolutionError, EnvironmentVariable, Header, HttpRequest, MapSecretProvider,
    SecretVariable, Variable, VariableValue, VariableValueSet, resolve_environment,
    resolve_environment_with_secret_provider, resolve_request, resolve_request_redacted,
};

fn variable(name: &str, value: &str) -> EnvironmentVariable {
    EnvironmentVariable::Plain(Variable {
        name: Some(name.to_owned()),
        value: Some(VariableValueSet::Single(VariableValue::String(
            value.to_owned(),
        ))),
        disabled: false,
    })
}

fn secret(name: &str) -> EnvironmentVariable {
    EnvironmentVariable::Secret(SecretVariable {
        name: Some(name.to_owned()),
        value_type: None,
        disabled: false,
    })
}

fn environment(name: &str, variables: Vec<EnvironmentVariable>) -> Environment {
    Environment {
        name: name.to_owned(),
        color: None,
        extends: None,
        dot_env_file_path: None,
        variables,
    }
}

fn bearer_request() -> HttpRequest {
    HttpRequest {
        method: Some("GET".to_owned()),
        url: Some("{{baseUrl}}/secret".to_owned()),
        headers: vec![Header {
            name: "Authorization".to_owned(),
            value: "Bearer {{secretToken}}".to_owned(),
            disabled: false,
        }],
        authentication: Some(Authentication {
            kind: AuthenticationKind::Bearer,
            properties: BTreeMap::from([(
                "token".to_owned(),
                AuthenticationValue::String("{{secretToken}}".to_owned()),
            )]),
        }),
        ..HttpRequest::default()
    }
}

#[test]
fn fake_provider_resolves_secrets_for_execution_and_keeps_refs_for_display() {
    let environments = [environment(
        "development",
        vec![
            variable("baseUrl", "https://api.example.com"),
            secret("secretToken"),
        ],
    )];
    let mut provider = MapSecretProvider::new();
    provider.insert("secretToken", "live-token-value");

    let resolved = resolve_environment_with_secret_provider(
        &environments,
        Some("development"),
        &[],
        Some(&provider),
    )
    .unwrap();

    assert_eq!(
        resolved.variable("baseUrl"),
        Some("https://api.example.com")
    );
    assert_eq!(resolved.variable("secretToken"), None);
    assert!(resolved.secrets_without_values().is_empty());
    assert_eq!(
        resolved.interpolate("Bearer {{secretToken}}").unwrap(),
        "Bearer live-token-value"
    );
    assert_eq!(
        resolved
            .interpolate_redacted("Bearer {{secretToken}}")
            .unwrap(),
        "Bearer {{secretToken}}"
    );

    let request = bearer_request();
    let executed = resolve_request(&request, &resolved).unwrap();
    let display = resolve_request_redacted(&request, &resolved).unwrap();
    assert_eq!(executed.headers[0].value, "Bearer live-token-value");
    assert_eq!(display.headers[0].value, "Bearer {{secretToken}}");
    assert_eq!(
        executed.authentication.as_ref().unwrap().properties["token"],
        AuthenticationValue::String("live-token-value".to_owned())
    );
    assert_eq!(
        display.authentication.as_ref().unwrap().properties["token"],
        AuthenticationValue::String("{{secretToken}}".to_owned())
    );
}

#[test]
fn missing_provider_or_key_fails_closed_without_emitting_values() {
    let environments = [environment("development", vec![secret("secretToken")])];

    let unresolved = resolve_environment(&environments, "development").unwrap();
    assert_eq!(
        unresolved.interpolate("{{secretToken}}").unwrap_err(),
        EnvironmentResolutionError::SecretVariableUnavailable("secretToken".to_owned())
    );

    let mut provider = MapSecretProvider::new();
    provider.insert("other", "must-not-be-used");
    let missing = resolve_environment_with_secret_provider(
        &environments,
        Some("development"),
        &[],
        Some(&provider),
    )
    .unwrap();
    let error = missing.interpolate("{{secretToken}}").unwrap_err();
    assert_eq!(
        error,
        EnvironmentResolutionError::SecretVariableUnavailable("secretToken".to_owned())
    );
    assert_eq!(
        error.to_string(),
        "secret variable has no runtime value: secretToken"
    );
    assert!(!error.to_string().contains("must-not-be-used"));
}

#[test]
fn structured_refs_resolve_by_key_and_stay_redacted() {
    let environments = [environment(
        "development",
        vec![secret("env:API_TOKEN"), secret("secret://vault/prod/token")],
    )];
    let mut provider = MapSecretProvider::new();
    provider
        .insert("API_TOKEN", "env-backend-value")
        .insert("secret://vault/prod/token", "vault-backend-value");

    let resolved = resolve_environment_with_secret_provider(
        &environments,
        Some("development"),
        &[],
        Some(&provider),
    )
    .unwrap();

    assert_eq!(
        resolved.interpolate("{{env:API_TOKEN}}").unwrap(),
        "env-backend-value"
    );
    assert_eq!(
        resolved
            .interpolate("{{secret://vault/prod/token}}")
            .unwrap(),
        "vault-backend-value"
    );
    assert_eq!(
        resolved
            .interpolate_redacted("{{env:API_TOKEN}} {{secret://vault/prod/token}}")
            .unwrap(),
        "{{env:API_TOKEN}} {{secret://vault/prod/token}}"
    );
}

#[test]
fn runtime_override_of_a_declared_secret_is_not_a_public_variable() {
    let environments = [environment(
        "development",
        vec![variable("host", "api.example.com"), secret("secretToken")],
    )];
    let resolved = resolve_environment_with_secret_provider(
        &environments,
        Some("development"),
        &[("secretToken".to_owned(), "override-token".to_owned())],
        None,
    )
    .unwrap();

    assert_eq!(resolved.variable("secretToken"), None);
    assert_eq!(
        resolved.interpolate("{{secretToken}}").unwrap(),
        "override-token"
    );
    assert_eq!(
        resolved.interpolate_redacted("{{secretToken}}").unwrap(),
        "{{secretToken}}"
    );
}

#[test]
fn debug_output_lists_secret_refs_never_values() {
    let environments = [environment("development", vec![secret("secretToken")])];
    let mut provider = MapSecretProvider::new();
    provider.insert("secretToken", "live-token-value");
    let resolved = resolve_environment_with_secret_provider(
        &environments,
        Some("development"),
        &[],
        Some(&provider),
    )
    .unwrap();

    let rendered = format!("{resolved:?}");
    assert!(rendered.contains("secretToken"));
    assert!(!rendered.contains("live-token-value"));
}
