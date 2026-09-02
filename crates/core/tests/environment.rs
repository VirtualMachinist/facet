use std::collections::BTreeMap;

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Environment,
    EnvironmentResolutionError, EnvironmentVariable, FormField, Header, HttpRequest, MultipartPart,
    MultipartPartKind, MultipartValue, QueryParameter, RawBody, RawBodyKind, RequestBody,
    SecretVariable, Variable, VariableValue, VariableValueSet, VariableValueVariant,
    resolve_environment, resolve_environment_with_overrides, resolve_request,
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

fn environment(
    name: &str,
    extends: Option<&str>,
    variables: Vec<EnvironmentVariable>,
) -> Environment {
    Environment {
        name: name.to_owned(),
        color: None,
        extends: extends.map(str::to_owned),
        dot_env_file_path: None,
        variables,
    }
}

#[test]
fn resolves_inheritance_overrides_variants_and_nested_values() {
    let environments = vec![
        environment(
            "base",
            None,
            vec![
                variable("host", "api.example.com"),
                variable("baseUrl", "https://{{host}}"),
                EnvironmentVariable::Plain(Variable {
                    name: Some("region".to_owned()),
                    value: Some(VariableValueSet::Variants(vec![
                        VariableValueVariant {
                            title: "US".to_owned(),
                            selected: false,
                            value: VariableValue::String("us".to_owned()),
                        },
                        VariableValueVariant {
                            title: "AU".to_owned(),
                            selected: true,
                            value: VariableValue::String("au".to_owned()),
                        },
                    ])),
                    disabled: false,
                }),
            ],
        ),
        environment(
            "development",
            Some("base"),
            vec![variable("host", "dev.example.com")],
        ),
    ];

    let resolved = resolve_environment(&environments, "development").unwrap();

    assert_eq!(resolved.name(), "development");
    assert_eq!(resolved.variable("host"), Some("dev.example.com"));
    assert_eq!(
        resolved.variable("baseUrl"),
        Some("https://dev.example.com")
    );
    assert_eq!(resolved.variable("region"), Some("au"));
    assert_eq!(
        resolved
            .interpolate("{{ baseUrl }}/{{region}}/users")
            .unwrap(),
        "https://dev.example.com/au/users"
    );
}

#[test]
fn runtime_variables_override_selected_and_inherited_values_before_resolution() {
    let environments = vec![
        environment(
            "base",
            None,
            vec![
                variable("baseUrl", "https://api.example.com"),
                variable("usersUrl", "{{baseUrl}}/users"),
                variable("region", "au"),
            ],
        ),
        environment(
            "development",
            Some("base"),
            vec![variable("baseUrl", "https://dev.example.com")],
        ),
    ];

    let resolved = resolve_environment_with_overrides(
        &environments,
        Some("development"),
        &[
            (
                "baseUrl".to_owned(),
                "https://staging.example.com".to_owned(),
            ),
            ("region".to_owned(), "us".to_owned()),
            ("runtimeOnly".to_owned(), "present".to_owned()),
        ],
    )
    .unwrap();

    assert_eq!(
        resolved.variable("baseUrl"),
        Some("https://staging.example.com")
    );
    assert_eq!(resolved.variable("region"), Some("us"));
    assert_eq!(resolved.variable("runtimeOnly"), Some("present"));
    assert_eq!(
        resolved.variable("usersUrl"),
        Some("https://staging.example.com/users")
    );
}

#[test]
fn runtime_only_variables_resolve_without_a_selected_environment() {
    let resolved =
        resolve_environment_with_overrides(&[], None, &[("userId".to_owned(), "123".to_owned())])
            .unwrap();

    assert_eq!(resolved.name(), "");
    assert_eq!(resolved.variable("userId"), Some("123"));
    assert_eq!(
        resolved.interpolate("/users/{{userId}}").unwrap(),
        "/users/123"
    );
}

#[test]
fn duplicate_runtime_variables_use_the_last_value() {
    let resolved = resolve_environment_with_overrides(
        &[],
        None,
        &[
            ("userId".to_owned(), "123".to_owned()),
            ("userId".to_owned(), "456".to_owned()),
        ],
    )
    .unwrap();

    assert_eq!(resolved.variable("userId"), Some("456"));
}

#[test]
fn runtime_variables_reject_an_empty_name() {
    assert_eq!(
        resolve_environment_with_overrides(&[], None, &[(String::new(), "value".to_owned())])
            .unwrap_err(),
        EnvironmentResolutionError::InvalidVariableName
    );
}

#[test]
fn resolves_supported_request_fields_without_mutating_the_source() {
    let request = HttpRequest {
        method: Some("{{method}}".to_owned()),
        url: Some("{{baseUrl}}/users".to_owned()),
        headers: vec![Header {
            name: "X-{{tenant}}".to_owned(),
            value: "Bearer {{token}}".to_owned(),
            disabled: false,
        }],
        query_parameters: vec![QueryParameter {
            name: "tenant".to_owned(),
            value: "{{tenant}}".to_owned(),
            disabled: false,
        }],
        path_parameters: vec![QueryParameter {
            name: "tenant".to_owned(),
            value: "{{tenant}}".to_owned(),
            disabled: false,
        }],
        body: Some(RequestBody::Variants(vec![probe_core::BodyVariant {
            title: "form".to_owned(),
            selected: true,
            body: Body::FormUrlEncoded(vec![FormField {
                name: "owner".to_owned(),
                value: "{{tenant}}".to_owned(),
                disabled: false,
            }]),
        }])),
        authentication: Some(Authentication {
            kind: AuthenticationKind::Bearer,
            properties: BTreeMap::from([(
                "token".to_owned(),
                AuthenticationValue::String("{{token}}".to_owned()),
            )]),
        }),
        ..HttpRequest::default()
    };
    let resolved = resolve_environment(
        &[environment(
            "complete",
            None,
            vec![
                variable("method", "POST"),
                variable("baseUrl", "https://dev.example.com"),
                variable("token", "test-token"),
                variable("tenant", "probe"),
            ],
        )],
        "complete",
    )
    .unwrap();

    let request_with_values = resolve_request(&request, &resolved).unwrap();

    assert_eq!(request.url.as_deref(), Some("{{baseUrl}}/users"));
    assert_eq!(request_with_values.method.as_deref(), Some("POST"));
    assert_eq!(
        request_with_values.url.as_deref(),
        Some("https://dev.example.com/users")
    );
    assert_eq!(request_with_values.headers[0].name, "X-probe");
    assert_eq!(request_with_values.headers[0].value, "Bearer test-token");
    assert_eq!(request_with_values.query_parameters[0].value, "probe");
    assert_eq!(request_with_values.path_parameters[0].value, "probe");
    let Some(RequestBody::Variants(variants)) = &request_with_values.body else {
        panic!("expected body variants");
    };
    let Body::FormUrlEncoded(fields) = &variants[0].body else {
        panic!("expected form body");
    };
    assert_eq!(fields[0].value, "probe");
    assert_eq!(
        request_with_values
            .authentication
            .as_ref()
            .unwrap()
            .properties["token"],
        AuthenticationValue::String("test-token".to_owned())
    );
}

#[test]
fn preserves_undefined_variables_and_rejects_unavailable_secrets() {
    let disabled = EnvironmentVariable::Plain(Variable {
        name: Some("disabled".to_owned()),
        value: Some(VariableValueSet::Single(VariableValue::String(
            "hidden".to_owned(),
        ))),
        disabled: true,
    });
    let secret = EnvironmentVariable::Secret(SecretVariable {
        name: Some("token".to_owned()),
        value_type: None,
        disabled: false,
    });
    let environments = [environment("development", None, vec![disabled, secret])];
    let resolved = resolve_environment(&environments, "development").unwrap();

    assert_eq!(
        resolved.interpolate("{{missing}}"),
        Ok("{{missing}}".to_owned())
    );
    assert_eq!(
        resolved.interpolate("prefix {{ disabled }} suffix"),
        Ok("prefix {{ disabled }} suffix".to_owned())
    );
    assert_eq!(
        resolved.interpolate_strict("{{missing}}").unwrap_err(),
        EnvironmentResolutionError::MissingVariable("missing".to_owned())
    );
    assert_eq!(
        resolved.interpolate_strict("{{disabled}}").unwrap_err(),
        EnvironmentResolutionError::MissingVariable("disabled".to_owned())
    );
    assert_eq!(
        resolved.interpolate("{{token}}").unwrap_err(),
        EnvironmentResolutionError::SecretVariableUnavailable("token".to_owned())
    );
}

#[test]
fn rejects_environment_and_variable_cycles() {
    let inheritance_cycle = [
        environment("a", Some("b"), vec![]),
        environment("b", Some("a"), vec![]),
    ];
    assert!(matches!(
        resolve_environment(&inheritance_cycle, "a"),
        Err(EnvironmentResolutionError::EnvironmentInheritanceCycle(_))
    ));

    let variable_cycle = [environment(
        "development",
        None,
        vec![variable("a", "{{b}}"), variable("b", "{{a}}")],
    )];
    assert!(matches!(
        resolve_environment(&variable_cycle, "development"),
        Err(EnvironmentResolutionError::VariableInterpolationCycle(_))
    ));
}

#[test]
fn rejects_unselected_variable_variants() {
    let environments = [environment(
        "development",
        None,
        vec![EnvironmentVariable::Plain(Variable {
            name: Some("region".to_owned()),
            value: Some(VariableValueSet::Variants(vec![VariableValueVariant {
                title: "AU".to_owned(),
                selected: false,
                value: VariableValue::String("au".to_owned()),
            }])),
            disabled: false,
        })],
    )];

    assert_eq!(
        resolve_environment(&environments, "development").unwrap_err(),
        EnvironmentResolutionError::NoSelectedVariant {
            environment: "development".to_owned(),
            variable: "region".to_owned(),
        }
    );
}

#[test]
fn resolves_raw_and_multipart_body_values() {
    let environment = resolve_environment(
        &[environment(
            "development",
            None,
            vec![variable("value", "resolved")],
        )],
        "development",
    )
    .unwrap();
    let mut raw_request = HttpRequest {
        body: Some(RequestBody::Single(Body::Raw(RawBody {
            kind: RawBodyKind::Json,
            data: "{\"value\":\"{{value}}\"}".to_owned(),
        }))),
        ..HttpRequest::default()
    };
    let multipart_request = HttpRequest {
        body: Some(RequestBody::Single(Body::Multipart(vec![MultipartPart {
            name: "upload".to_owned(),
            kind: MultipartPartKind::File,
            value: MultipartValue::Multiple(vec!["./{{value}}.txt".to_owned()]),
            content_type: Some("text/{{value}}".to_owned()),
            disabled: false,
        }]))),
        ..HttpRequest::default()
    };

    raw_request = resolve_request(&raw_request, &environment).unwrap();
    let multipart_request = resolve_request(&multipart_request, &environment).unwrap();
    let Some(RequestBody::Single(Body::Raw(raw))) = raw_request.body else {
        panic!("expected raw body");
    };
    assert_eq!(raw.data, "{\"value\":\"resolved\"}");
    let Some(RequestBody::Single(Body::Multipart(parts))) = multipart_request.body else {
        panic!("expected multipart body");
    };
    assert_eq!(
        parts[0].value,
        MultipartValue::Multiple(vec!["./resolved.txt".to_owned()])
    );
    assert_eq!(parts[0].content_type.as_deref(), Some("text/resolved"));
}

#[test]
fn set_environment_variable_updates_overrides_and_rejects_secrets() {
    let mut environments = vec![
        environment(
            "base",
            None,
            vec![
                variable("host", "api.example.com"),
                EnvironmentVariable::Secret(SecretVariable {
                    name: Some("inheritedSecret".to_owned()),
                    value_type: None,
                    disabled: false,
                }),
            ],
        ),
        environment(
            "development",
            Some("base"),
            vec![
                variable("token", "development-token"),
                EnvironmentVariable::Secret(SecretVariable {
                    name: Some("secretToken".to_owned()),
                    value_type: None,
                    disabled: false,
                }),
            ],
        ),
    ];

    probe_core::set_environment_variable(
        &mut environments,
        "development",
        "token",
        "rotated".to_owned(),
    )
    .unwrap();
    probe_core::set_environment_variable(
        &mut environments,
        "development",
        "host",
        "dev.example.com".to_owned(),
    )
    .unwrap();

    let resolved = resolve_environment(&environments, "development").unwrap();
    assert_eq!(resolved.variable("token"), Some("rotated"));
    assert_eq!(resolved.variable("host"), Some("dev.example.com"));
    assert_eq!(
        resolve_environment(&environments, "base")
            .unwrap()
            .variable("host"),
        Some("api.example.com")
    );

    assert_eq!(
        probe_core::set_environment_variable(
            &mut environments,
            "development",
            "secretToken",
            "nope".to_owned(),
        )
        .unwrap_err(),
        EnvironmentResolutionError::SecretVariableUnavailable("secretToken".to_owned())
    );
    assert_eq!(
        probe_core::set_environment_variable(
            &mut environments,
            "development",
            "inheritedSecret",
            "nope".to_owned(),
        )
        .unwrap_err(),
        EnvironmentResolutionError::SecretVariableUnavailable("inheritedSecret".to_owned())
    );
    assert_eq!(
        probe_core::set_environment_variable(
            &mut environments,
            "missing",
            "host",
            "nope".to_owned(),
        )
        .unwrap_err(),
        EnvironmentResolutionError::EnvironmentNotFound("missing".to_owned())
    );
    assert_eq!(
        probe_core::set_environment_variable(
            &mut environments,
            "development",
            "",
            "nope".to_owned()
        )
        .unwrap_err(),
        EnvironmentResolutionError::InvalidVariableName
    );
    assert!(
        environments[1]
            .variables
            .iter()
            .all(|variable| match variable {
                EnvironmentVariable::Plain(variable) => {
                    variable.name.as_deref() != Some("inheritedSecret")
                }
                EnvironmentVariable::Secret(_) => true,
            })
    );
}

#[test]
fn unset_environment_variable_removes_local_entry_and_restores_parent() {
    let mut environments = vec![
        environment(
            "base",
            None,
            vec![
                variable("host", "api.example.com"),
                EnvironmentVariable::Secret(SecretVariable {
                    name: Some("inheritedSecret".to_owned()),
                    value_type: None,
                    disabled: false,
                }),
            ],
        ),
        environment(
            "development",
            Some("base"),
            vec![
                variable("host", "dev.example.com"),
                variable("token", "development-token"),
                EnvironmentVariable::Secret(SecretVariable {
                    name: Some("secretToken".to_owned()),
                    value_type: None,
                    disabled: false,
                }),
            ],
        ),
    ];

    probe_core::unset_environment_variable(&mut environments, "development", "host").unwrap();
    let resolved = resolve_environment(&environments, "development").unwrap();
    assert_eq!(resolved.variable("host"), Some("api.example.com"));
    assert_eq!(resolved.variable("token"), Some("development-token"));
    assert_eq!(
        resolve_environment(&environments, "base")
            .unwrap()
            .variable("host"),
        Some("api.example.com")
    );

    assert_eq!(
        probe_core::unset_environment_variable(&mut environments, "development", "host")
            .unwrap_err(),
        EnvironmentResolutionError::VariableNotFound {
            environment: "development".to_owned(),
            variable: "host".to_owned(),
        }
    );
    assert_eq!(
        probe_core::unset_environment_variable(&mut environments, "development", "secretToken")
            .unwrap_err(),
        EnvironmentResolutionError::SecretVariableUnavailable("secretToken".to_owned())
    );
    assert_eq!(
        probe_core::unset_environment_variable(
            &mut environments,
            "development",
            "inheritedSecret",
        )
        .unwrap_err(),
        EnvironmentResolutionError::VariableNotFound {
            environment: "development".to_owned(),
            variable: "inheritedSecret".to_owned(),
        }
    );
    assert_eq!(
        probe_core::unset_environment_variable(&mut environments, "missing", "host").unwrap_err(),
        EnvironmentResolutionError::EnvironmentNotFound("missing".to_owned())
    );
    assert!(environments[1].variables.iter().any(|variable| {
        matches!(
            variable,
            EnvironmentVariable::Secret(secret) if secret.name.as_deref() == Some("secretToken")
        )
    }));
}

#[test]
fn create_environment_appends_and_validates_inheritance() {
    let mut environments = vec![environment(
        "base",
        None,
        vec![variable("host", "api.example.com")],
    )];

    probe_core::create_environment(
        &mut environments,
        "development".to_owned(),
        Some("base".to_owned()),
    )
    .unwrap();

    assert_eq!(environments.len(), 2);
    assert_eq!(environments[1].name, "development");
    assert_eq!(environments[1].extends.as_deref(), Some("base"));
    assert!(environments[1].variables.is_empty());
    assert_eq!(
        resolve_environment(&environments, "development")
            .unwrap()
            .variable("host"),
        Some("api.example.com")
    );
}

#[test]
fn create_environment_rejects_invalid_names_and_parents() {
    let mut environments = vec![environment("base", None, vec![])];

    assert_eq!(
        probe_core::create_environment(&mut environments, String::new(), None).unwrap_err(),
        EnvironmentResolutionError::InvalidEnvironmentName
    );
    assert_eq!(
        probe_core::create_environment(
            &mut environments,
            "base".to_owned(),
            Some("base".to_owned()),
        )
        .unwrap_err(),
        EnvironmentResolutionError::DuplicateEnvironment("base".to_owned())
    );
    assert_eq!(
        probe_core::create_environment(
            &mut environments,
            "staging".to_owned(),
            Some("missing".to_owned()),
        )
        .unwrap_err(),
        EnvironmentResolutionError::ParentEnvironmentNotFound {
            environment: "staging".to_owned(),
            parent: "missing".to_owned(),
        }
    );
}

#[test]
fn create_environment_rejects_inheritance_cycles() {
    let mut environments = vec![Environment {
        name: "a".to_owned(),
        color: None,
        extends: Some("b".to_owned()),
        dot_env_file_path: None,
        variables: Vec::new(),
    }];

    let before = environments.clone();
    assert!(matches!(
        probe_core::create_environment(&mut environments, "b".to_owned(), Some("a".to_owned()))
            .unwrap_err(),
        EnvironmentResolutionError::EnvironmentInheritanceCycle(_)
    ));
    assert_eq!(environments, before);
}

#[test]
fn revert_created_environment_removes_unused_names_but_keeps_parents() {
    let mut environments = vec![environment("base", None, vec![])];
    probe_core::create_environment(&mut environments, "staging".to_owned(), None).unwrap();
    probe_core::revert_created_environment(&mut environments, "staging");
    assert_eq!(environments.len(), 1);
    assert_eq!(environments[0].name, "base");

    probe_core::create_environment(
        &mut environments,
        "staging".to_owned(),
        Some("base".to_owned()),
    )
    .unwrap();
    probe_core::revert_created_environment(&mut environments, "base");
    assert_eq!(environments.len(), 2);
}

#[test]
fn replace_environment_validates_and_updates_child_references() {
    let mut environments = vec![
        environment("base", None, vec![]),
        environment("development", Some("base"), vec![]),
    ];
    let mut replacement = environments[0].clone();
    replacement.name = "shared".to_owned();
    probe_core::replace_environment(&mut environments, "base", replacement).unwrap();
    assert_eq!(environments[0].name, "shared");
    assert_eq!(environments[1].extends.as_deref(), Some("shared"));

    let before = environments.clone();
    let mut invalid = environments[1].clone();
    invalid.extends = Some("missing".to_owned());
    assert!(matches!(
        probe_core::replace_environment(&mut environments, "development", invalid),
        Err(EnvironmentResolutionError::ParentEnvironmentNotFound { .. })
    ));
    assert_eq!(environments, before);
}

#[test]
fn replace_environment_rejects_duplicate_variable_names() {
    let original = environment(
        "development",
        None,
        vec![variable("host", "dev.example.com")],
    );
    let cases = [
        (
            vec![
                variable("host", "one.example.com"),
                variable("host", "two.example.com"),
            ],
            "host",
        ),
        (vec![variable("token", "plain"), secret("token")], "token"),
        (vec![secret("token"), variable("token", "plain")], "token"),
    ];
    for (variables, name) in cases {
        let mut environments = vec![original.clone()];
        let before = environments.clone();
        let replacement = environment("development", None, variables);
        assert_eq!(
            probe_core::replace_environment(&mut environments, "development", replacement)
                .unwrap_err(),
            EnvironmentResolutionError::DuplicateVariable {
                environment: "development".to_owned(),
                variable: name.to_owned(),
            }
        );
        assert_eq!(environments, before);
    }
}

#[test]
fn delete_environment_rejects_parents() {
    let mut environments = vec![
        environment("base", None, vec![]),
        environment("development", Some("base"), vec![]),
    ];
    assert_eq!(
        probe_core::delete_environment(&mut environments, "base").unwrap_err(),
        EnvironmentResolutionError::EnvironmentInUse("base".to_owned())
    );
    probe_core::delete_environment(&mut environments, "development").unwrap();
    assert_eq!(environments.len(), 1);
}

fn effective_names(rows: &[probe_core::EffectiveEnvironmentVariable]) -> Vec<(&str, &str, bool)> {
    rows.iter()
        .map(|row| {
            (
                row.variable.name.as_deref().unwrap_or(""),
                row.defined_in.as_str(),
                row.direct_index.is_some(),
            )
        })
        .collect()
}

#[test]
fn effective_environment_variables_resolve_inheritance_overrides_and_secrets() {
    let environments = vec![
        environment(
            "root",
            None,
            vec![variable("host", "root.example.com"), secret("shadowed")],
        ),
        environment(
            "base",
            Some("root"),
            vec![
                variable("host", "base.example.com"),
                variable("region", "us"),
                EnvironmentVariable::Plain(Variable {
                    name: Some("disabled".to_owned()),
                    value: Some(VariableValueSet::Single(VariableValue::String(
                        "hidden".to_owned(),
                    ))),
                    disabled: true,
                }),
                variable("token", "parent-token"),
            ],
        ),
        environment(
            "development",
            Some("base"),
            vec![secret("token"), variable("local", "dev")],
        ),
    ];
    let rows = probe_core::effective_environment_variables(&environments, &environments[2]);
    assert_eq!(
        effective_names(&rows),
        vec![
            ("local", "development", true),
            ("host", "base", false),
            ("region", "base", false),
            ("disabled", "base", false),
        ]
    );
    assert!(
        rows.iter().any(|row| {
            row.variable.name.as_deref() == Some("disabled") && row.variable.disabled
        })
    );
}
