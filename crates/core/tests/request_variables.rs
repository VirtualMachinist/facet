use std::collections::BTreeMap;

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, BodyVariant, Environment,
    EnvironmentResolutionError, EnvironmentVariable, FileReference, FormField, Header, HttpRequest,
    MultipartPart, MultipartPartKind, MultipartValue, QueryParameter, RawBody, RawBodyKind,
    RequestBody, SecretVariable, Variable, VariableUsage, VariableValue, VariableValueSet,
    discover_request_variables,
};

fn plain(name: &str) -> EnvironmentVariable {
    EnvironmentVariable::Plain(Variable {
        name: Some(name.to_owned()),
        value: None,
        disabled: false,
    })
}

fn valued(name: &str, value: &str) -> EnvironmentVariable {
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

fn comprehensive_request() -> HttpRequest {
    HttpRequest {
        method: Some("{{ method }}".to_owned()),
        url: Some("{{baseUrl}}/users/{{userId}}?again={{baseUrl}}".to_owned()),
        headers: vec![Header {
            name: "X-{{headerName}}".to_owned(),
            value: "Bearer {{token}} {{shared}} {{shared}}".to_owned(),
            disabled: false,
        }],
        query_parameters: vec![QueryParameter {
            name: "{{queryName}}".to_owned(),
            value: "{{queryValue}}/{{shared}}".to_owned(),
            disabled: false,
        }],
        path_parameters: vec![QueryParameter {
            name: "{{pathName}}".to_owned(),
            value: "{{pathValue}}/{{shared}}".to_owned(),
            disabled: false,
        }],
        body: Some(RequestBody::Variants(vec![
            BodyVariant {
                title: "raw".to_owned(),
                selected: true,
                body: Body::Raw(RawBody {
                    kind: RawBodyKind::Json,
                    data: "{{rawValue}} {{shared}}".to_owned(),
                }),
            },
            BodyVariant {
                title: "form".to_owned(),
                selected: false,
                body: Body::FormUrlEncoded(vec![FormField {
                    name: "{{formName}}".to_owned(),
                    value: "{{formValue}}".to_owned(),
                    disabled: false,
                }]),
            },
            BodyVariant {
                title: "multipart".to_owned(),
                selected: false,
                body: Body::Multipart(vec![MultipartPart {
                    name: "{{multipartName}}".to_owned(),
                    kind: MultipartPartKind::File,
                    value: MultipartValue::Multiple(vec![
                        "{{multipartValue}}".to_owned(),
                        "{{multipartValue}}".to_owned(),
                    ]),
                    content_type: Some("{{multipartType}}".to_owned()),
                    disabled: false,
                }]),
            },
            BodyVariant {
                title: "file".to_owned(),
                selected: false,
                body: Body::File(vec![FileReference {
                    file_path: "{{filePath}}".to_owned(),
                    content_type: "{{fileType}}".to_owned(),
                    selected: true,
                }]),
            },
        ])),
        authentication: Some(Authentication {
            kind: AuthenticationKind::OAuth2,
            properties: BTreeMap::from([
                (
                    "number".to_owned(),
                    AuthenticationValue::Number("{{authNumber}}".to_owned()),
                ),
                (
                    "token".to_owned(),
                    AuthenticationValue::Object(BTreeMap::from([(
                        "nested".to_owned(),
                        AuthenticationValue::Sequence(vec![AuthenticationValue::String(
                            "{{nestedAuth}}".to_owned(),
                        )]),
                    )])),
                ),
            ]),
        }),
        ..HttpRequest::default()
    }
}

#[test]
fn discovers_every_resolved_request_field_and_deduplicates_deterministically() {
    let environments = vec![
        environment(
            "base",
            None,
            vec![
                valued("baseUrl", "https://example.com"),
                secret("token"),
                plain("shared"),
                plain("plainToSecret"),
                secret("secretToPlain"),
                plain("disabledByChild"),
            ],
        ),
        environment(
            "child",
            Some("base"),
            vec![
                secret("plainToSecret"),
                plain("secretToPlain"),
                EnvironmentVariable::Secret(SecretVariable {
                    name: Some("disabledByChild".to_owned()),
                    value_type: None,
                    disabled: true,
                }),
            ],
        ),
    ];

    let variables =
        discover_request_variables(&comprehensive_request(), &environments, Some("child")).unwrap();
    let names = variables
        .iter()
        .map(|variable| variable.name.as_str())
        .collect::<Vec<_>>();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);

    let find = |name: &str| {
        variables
            .iter()
            .find(|variable| variable.name == name)
            .unwrap()
    };
    assert!(find("baseUrl").defined);
    assert!(!find("baseUrl").secret);
    assert_eq!(find("baseUrl").usages, vec![VariableUsage::Url]);
    assert!(find("token").defined);
    assert!(find("token").secret);
    assert!(
        find("shared").defined,
        "a plain declaration without a value is metadata-defined"
    );
    assert!(!find("userId").defined);
    assert!(!find("userId").secret);
    assert_eq!(find("multipartValue").usages.len(), 1);
    assert_eq!(find("shared").usages.len(), 4);
    assert_eq!(find("method").usages, vec![VariableUsage::Method]);
    assert_eq!(
        find("headerName").usages,
        vec![VariableUsage::Header {
            name: "X-{{headerName}}".to_owned()
        }]
    );
    assert_eq!(
        find("queryValue").usages,
        vec![VariableUsage::QueryParameter {
            name: "{{queryName}}".to_owned()
        }]
    );
    assert_eq!(
        find("pathValue").usages,
        vec![VariableUsage::PathParameter {
            name: "{{pathName}}".to_owned()
        }]
    );
    assert_eq!(find("rawValue").usages, vec![VariableUsage::Body]);
    assert_eq!(
        find("formValue").usages,
        vec![VariableUsage::FormUrlEncoded {
            name: "{{formName}}".to_owned()
        }]
    );
    assert_eq!(
        find("multipartType").usages,
        vec![VariableUsage::Multipart {
            name: "{{multipartName}}".to_owned()
        }]
    );
    assert_eq!(find("filePath").usages, vec![VariableUsage::File]);
    assert_eq!(
        find("nestedAuth").usages,
        vec![VariableUsage::Authentication {
            name: "token".to_owned()
        }]
    );

    for expected in [
        "method",
        "headerName",
        "queryName",
        "queryValue",
        "pathName",
        "pathValue",
        "rawValue",
        "formName",
        "formValue",
        "multipartName",
        "multipartValue",
        "multipartType",
        "filePath",
        "fileType",
        "authNumber",
        "nestedAuth",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
}

#[test]
fn effective_declarations_respect_inheritance_kind_overrides_and_disabled_shadowing() {
    let request = HttpRequest {
        url: Some(
            "{{inheritedPlain}}/{{inheritedSecret}}/{{plainToSecret}}/{{secretToPlain}}/{{disabledByChild}}"
                .to_owned(),
        ),
        ..HttpRequest::default()
    };
    let environments = vec![
        environment(
            "base",
            None,
            vec![
                plain("inheritedPlain"),
                secret("inheritedSecret"),
                plain("plainToSecret"),
                secret("secretToPlain"),
                plain("disabledByChild"),
            ],
        ),
        environment(
            "child",
            Some("base"),
            vec![
                secret("plainToSecret"),
                plain("secretToPlain"),
                EnvironmentVariable::Plain(Variable {
                    name: Some("disabledByChild".to_owned()),
                    value: None,
                    disabled: true,
                }),
            ],
        ),
    ];
    let variables = discover_request_variables(&request, &environments, Some("child")).unwrap();
    let declaration = |name: &str| {
        let variable = variables
            .iter()
            .find(|variable| variable.name == name)
            .unwrap();
        (variable.defined, variable.secret)
    };

    assert_eq!(declaration("inheritedPlain"), (true, false));
    assert_eq!(declaration("inheritedSecret"), (true, true));
    assert_eq!(declaration("plainToSecret"), (true, true));
    assert_eq!(declaration("secretToPlain"), (true, false));
    assert_eq!(declaration("disabledByChild"), (false, false));
}

#[test]
fn missing_and_unavailable_secrets_do_not_block_discovery() {
    let request = HttpRequest {
        url: Some("{{missing}}/{{secret}}".to_owned()),
        ..HttpRequest::default()
    };
    let variables = discover_request_variables(
        &request,
        &[environment("selected", None, vec![secret("secret")])],
        Some("selected"),
    )
    .unwrap();

    assert_eq!(variables[0].name, "missing");
    assert!(!variables[0].defined);
    assert_eq!(variables[1].name, "secret");
    assert!(variables[1].defined);
    assert!(variables[1].secret);
}

#[test]
fn malformed_and_empty_interpolation_match_resolution_parser_semantics() {
    for malformed in ["{{", "{{ }}", "{{outer{{inner}}"] {
        let request = HttpRequest {
            url: Some(malformed.to_owned()),
            ..HttpRequest::default()
        };
        assert_eq!(
            discover_request_variables(&request, &[], None).unwrap_err(),
            EnvironmentResolutionError::MalformedInterpolation
        );
    }

    assert!(
        discover_request_variables(&HttpRequest::default(), &[], None)
            .unwrap()
            .is_empty()
    );
}
