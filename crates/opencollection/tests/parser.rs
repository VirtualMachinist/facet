use std::{fs, path::PathBuf, time::Duration};

use probe_core::{
    AuthenticationKind, AuthenticationValue, Body, CollectionItem, EnvironmentVariable,
    MultipartValue, RawBodyKind, RequestBody, VariableValue, VariableValueSet, VariableValueType,
    Workspace, WorkspaceItemRef,
};
use probe_opencollection::parse;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection")
        .join(name);
    fs::read_to_string(path).expect("fixture should be readable")
}

#[test]
fn parses_collection_folders_and_http_requests() {
    let parsed = parse(&fixture("phase1-bundled.yml")).expect("fixture should parse");
    let collection = parsed.collection();

    assert_eq!(collection.metadata.name.as_deref(), Some("Pet Store"));
    assert_eq!(
        collection.metadata.summary.as_deref(),
        Some("Requests for the example pet service")
    );
    assert_eq!(collection.metadata.version.as_deref(), Some("2.1.0"));
    assert_eq!(collection.metadata.authors.len(), 1);
    assert_eq!(
        collection.metadata.authors[0].email.as_deref(),
        Some("probe@example.com")
    );
    assert_eq!(collection.items.len(), 2);

    let CollectionItem::Folder(folder) = &collection.items[0] else {
        panic!("first item should be a folder");
    };
    assert_eq!(folder.metadata.name.as_deref(), Some("Pets"));
    assert_eq!(folder.metadata.sequence, Some(1.0));
    assert_eq!(folder.items.len(), 1);

    let CollectionItem::HttpRequest(request) = &folder.items[0] else {
        panic!("folder child should be an HTTP request");
    };
    assert_eq!(request.metadata.name.as_deref(), Some("List pets"));
    assert_eq!(request.method.as_deref(), Some("GET"));
    assert_eq!(request.url.as_deref(), Some("https://api.example.com/pets"));
    assert_eq!(request.headers.len(), 2);
    assert_eq!(request.headers[0].name, "Accept");
    assert!(!request.headers[0].disabled);
    assert!(request.headers[1].disabled);
    assert_eq!(request.query_parameters.len(), 1);
    assert_eq!(request.query_parameters[0].name, "limit");
    assert_eq!(request.query_parameters[0].value, "25");
    assert_eq!(request.path_parameters.len(), 1);
    assert_eq!(request.path_parameters[0].name, "ownerId");
    assert_eq!(request.path_parameters[0].value, "42");
    assert_eq!(
        request.settings.timeout,
        Some(Duration::from_micros(2_500_500))
    );
    assert_eq!(request.settings.follow_redirects, Some(false));
    assert_eq!(request.settings.max_redirects, Some(3));
}

#[test]
fn fixtures_round_trip_without_data_loss() {
    for name in [
        "phase1-round-trip.yml",
        "phase1-bodies-auth-environments.yml",
    ] {
        let source = fixture(name);
        let parsed = parse(&source).unwrap_or_else(|_| panic!("{name} should parse"));
        let serialized = parsed
            .to_yaml()
            .unwrap_or_else(|_| panic!("{name} should serialize"));
        let reparsed =
            parse(&serialized).unwrap_or_else(|_| panic!("{name} serialized should parse"));

        assert_eq!(parsed.collection(), reparsed.collection(), "{name}");

        let before: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&source).expect("source should be YAML");
        let after: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(&serialized).expect("serialized output should be YAML");
        assert_eq!(before, after, "{name}");
    }
}

#[test]
fn parses_bodies_authentication_and_environments() {
    let parsed = parse(&fixture("phase1-bodies-auth-environments.yml"))
        .expect("complete fixture should parse");
    let collection = parsed.collection();

    assert_eq!(collection.environments.len(), 2);
    let development = &collection.environments[0];
    assert_eq!(development.name, "development");
    assert_eq!(development.color.as_deref(), Some("green"));
    assert_eq!(
        development.dot_env_file_path.as_deref(),
        Some(".env.development")
    );
    assert_eq!(development.variables.len(), 4);

    let EnvironmentVariable::Plain(retries) = &development.variables[1] else {
        panic!("retries should be a plain variable");
    };
    assert_eq!(
        retries.value,
        Some(VariableValueSet::Single(VariableValue::Typed {
            kind: VariableValueType::Number,
            data: "3".to_owned(),
        }))
    );

    let EnvironmentVariable::Plain(region) = &development.variables[2] else {
        panic!("region should be a plain variable");
    };
    let Some(VariableValueSet::Variants(region_values)) = &region.value else {
        panic!("region should have variants");
    };
    assert_eq!(region_values.len(), 2);
    assert!(region_values[0].selected);

    let EnvironmentVariable::Secret(secret) = &development.variables[3] else {
        panic!("apiToken should be secret");
    };
    assert_eq!(secret.name.as_deref(), Some("apiToken"));
    assert_eq!(secret.value_type, Some(VariableValueType::String));
    assert_eq!(
        collection.environments[1].extends.as_deref(),
        Some("development")
    );

    let requests: Vec<_> = collection
        .items
        .iter()
        .map(|item| match item {
            CollectionItem::HttpRequest(request) => request,
            CollectionItem::Folder(_) => panic!("fixture should contain only requests"),
        })
        .collect();
    assert_eq!(requests.len(), 5);

    let Some(RequestBody::Single(Body::Raw(raw))) = &requests[0].body else {
        panic!("first request should have a raw body");
    };
    assert_eq!(raw.kind, RawBodyKind::Json);
    assert_eq!(raw.data, r#"{"name":"Milo"}"#);
    let bearer = requests[0]
        .authentication
        .as_ref()
        .expect("first request should have auth");
    assert_eq!(bearer.kind, AuthenticationKind::Bearer);
    assert_eq!(
        bearer.properties.get("token"),
        Some(&AuthenticationValue::String("{{apiToken}}".to_owned()))
    );

    let Some(RequestBody::Single(Body::FormUrlEncoded(fields))) = &requests[1].body else {
        panic!("second request should have a form body");
    };
    assert_eq!(fields.len(), 2);
    assert!(fields[1].disabled);
    assert_eq!(
        requests[1].authentication.as_ref().map(|auth| &auth.kind),
        Some(&AuthenticationKind::Basic)
    );

    let Some(RequestBody::Single(Body::Multipart(parts))) = &requests[2].body else {
        panic!("third request should have a multipart body");
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(
        parts[1].value,
        MultipartValue::Multiple(vec![
            "./images/one.png".to_owned(),
            "./images/two.png".to_owned()
        ])
    );

    let Some(RequestBody::Single(Body::File(files))) = &requests[3].body else {
        panic!("fourth request should have a file body");
    };
    assert_eq!(files[0].file_path, "./archive.zip");
    assert!(files[0].selected);
    assert_eq!(
        requests[3].authentication.as_ref().map(|auth| &auth.kind),
        Some(&AuthenticationKind::Inherit)
    );

    let Some(RequestBody::Variants(variants)) = &requests[4].body else {
        panic!("fifth request should have body variants");
    };
    assert_eq!(variants.len(), 2);
    assert!(variants[0].selected);
    let oauth = requests[4]
        .authentication
        .as_ref()
        .expect("fifth request should have auth");
    assert_eq!(oauth.kind, AuthenticationKind::OAuth2);
    assert_eq!(
        oauth.properties.get("flow"),
        Some(&AuthenticationValue::String(
            "client_credentials".to_owned()
        ))
    );
}

#[test]
fn loads_and_indexes_more_than_one_thousand_requests() {
    let parsed = parse(&fixture("phase2-large-workspace.yml"))
        .expect("large workspace fixture should parse");
    let workspace = Workspace::from_collection(parsed.into_collection());

    assert_eq!(workspace.request_count(), 1_001);
    assert_eq!(workspace.root_items().len(), 1_001);
    let Some(WorkspaceItemRef::Request(last_request)) = workspace.root_items().last() else {
        panic!("last root item should be a request");
    };
    assert_eq!(
        workspace
            .request(*last_request)
            .and_then(|request| request.metadata.name.as_deref()),
        Some("Request 1000")
    );
}

#[test]
fn rejects_missing_or_unsupported_collection_headers() {
    for source in [
        "info: { name: Missing version }\nbundled: true\n",
        "opencollection: 1.0.0\nbundled: true\n",
        "opencollection: 1.0.0\ninfo: { name: Missing mode }\n",
        "opencollection: 999.0.0\ninfo: { name: Future }\nbundled: true\n",
    ] {
        assert!(parse(source).is_err(), "unexpectedly accepted:\n{source}");
    }
}

#[test]
fn rejects_invalid_environment_inheritance_during_parse() {
    let source = concat!(
        "opencollection: 1.0.0\n",
        "info: { name: Invalid environments }\n",
        "bundled: true\n",
        "config:\n",
        "  environments:\n",
        "    - { name: development, extends: missing }\n",
    );

    assert!(parse(source).is_err());
}
