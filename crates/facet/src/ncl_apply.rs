//! `facet ncl apply`: export frozen World projections, then act on them.
//!
//! - POST frozen `cluster` objects to the Kubernetes API (`FACET_KUBECONFIG`).
//! - `Store::put` for each `intent` row when `FACET_HEDRON_DB` is set.
//! - OpenCollection `calls` items are skipped when there are no HTTP requests.
//!
//! Records one Lattice row (`ncl:apply`) covering eval + action. Never evals on :6443.

use std::path::PathBuf;

use facet_record::kube_api_base_from_env;
use hedron_core::Store;
use hedron_ncl::r#gen::docs_eod::check_shape;
use probe_core::{
    Body, Header, HttpRequest, ItemMetadata, RawBody, RawBodyKind, RequestBody, RequestSettings,
};
use probe_http::HttpError;
use serde_json::{Value, json};

use crate::ncl::NclExport;
use crate::{FacetError, run};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NclApply {
    pub path: PathBuf,
    pub module_hash: String,
    pub export_hash: String,
    pub contract_set: &'static str,
    pub cluster: Value,
    pub intent: Value,
    pub calls: Value,
    pub action: ApplyAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAction {
    pub cluster: ClusterAction,
    pub intent: IntentAction,
    pub calls: CallsAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterAction {
    pub skipped: bool,
    pub posted: usize,
    pub results: Vec<ClusterPostResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterPostResult {
    pub kind: String,
    pub name: String,
    pub status: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentAction {
    pub skipped: bool,
    pub put: usize,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallsAction {
    pub skipped: bool,
    pub executed: usize,
}

impl NclApply {
    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path.display().to_string(),
            "moduleHash": self.module_hash,
            "exportHash": self.export_hash,
            "contractSet": self.contract_set,
            "cluster": self.cluster,
            "intent": self.intent,
            "calls": self.calls,
            "action": self.action.to_json(),
        })
    }
}

impl ApplyAction {
    pub fn to_json(&self) -> Value {
        json!({
            "cluster": self.cluster.to_json(),
            "intent": self.intent.to_json(),
            "calls": self.calls.to_json(),
        })
    }
}

impl ClusterAction {
    fn to_json(&self) -> Value {
        json!({
            "skipped": self.skipped,
            "posted": self.posted,
            "results": self.results.iter().map(|row| json!({
                "kind": row.kind,
                "name": row.name,
                "status": row.status,
            })).collect::<Vec<_>>(),
        })
    }
}

impl IntentAction {
    fn to_json(&self) -> Value {
        json!({
            "skipped": self.skipped,
            "put": self.put,
            "names": self.names,
        })
    }
}

impl CallsAction {
    fn to_json(&self) -> Value {
        json!({
            "skipped": self.skipped,
            "executed": self.executed,
        })
    }
}

pub fn execute(export: &NclExport) -> Result<ApplyAction, FacetError> {
    let cluster = apply_cluster(&export.cluster)?;
    let intent = apply_intent(&export.intent)?;
    let calls = apply_calls(&export.calls)?;
    Ok(ApplyAction {
        cluster,
        intent,
        calls,
    })
}

pub fn from_export(export: NclExport, action: ApplyAction) -> NclApply {
    NclApply {
        path: export.path,
        module_hash: export.module_hash,
        export_hash: export.export_hash,
        contract_set: export.contract_set,
        cluster: export.cluster,
        intent: export.intent,
        calls: export.calls,
        action,
    }
}

fn apply_cluster(cluster: &Value) -> Result<ClusterAction, FacetError> {
    let objects = cluster_objects(cluster)?;
    if objects.is_empty() {
        return Ok(ClusterAction {
            skipped: true,
            posted: 0,
            results: Vec::new(),
        });
    }
    let Some(api_base) = kube_api_base_from_env().map_err(http_config)? else {
        return Ok(ClusterAction {
            skipped: true,
            posted: 0,
            results: Vec::new(),
        });
    };
    let mut results = Vec::new();
    for object in objects {
        let url = post_url(&api_base, object)?;
        let request = json_post_request(&url, object);
        let execution = run::execute(&request, None, None)?;
        let status = execution
            .result
            .as_ref()
            .map(|response| response.status)
            .map_err(FacetError::http)?;
        results.push(ClusterPostResult {
            kind: object["kind"].as_str().unwrap_or("").to_owned(),
            name: object["metadata"]["name"].as_str().unwrap_or("").to_owned(),
            status,
        });
    }
    Ok(ClusterAction {
        skipped: false,
        posted: results.len(),
        results,
    })
}

fn apply_intent(intent: &Value) -> Result<IntentAction, FacetError> {
    let rows = intent_rows(intent)?;
    if rows.is_empty() {
        return Ok(IntentAction {
            skipped: true,
            put: 0,
            names: Vec::new(),
        });
    }
    let Some(db_path) = std::env::var_os("FACET_HEDRON_DB") else {
        return Ok(IntentAction {
            skipped: true,
            put: 0,
            names: Vec::new(),
        });
    };
    if db_path.is_empty() {
        return Err(FacetError::invalid_arguments("FACET_HEDRON_DB is empty"));
    }
    let path = PathBuf::from(db_path);
    let (mut store, token) = open_hedron_store(&path)?;
    let mut names = Vec::new();
    for row in rows {
        let name = row["name"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| FacetError::invalid_arguments("intent rows require a non-empty name"))?;
        let importance = row["importance"].as_f64().unwrap_or(0.5);
        let spec = &row["spec"];
        check_shape(spec).map_err(|error| {
            FacetError::invalid_arguments(format!("intent spec shape: {error}"))
        })?;
        let yaml_spec = serde_yaml::from_str(&serde_json::to_string(spec).map_err(|error| {
            FacetError::invalid_arguments(format!("intent spec is not JSON-serializable: {error}"))
        })?)
        .map_err(|error| {
            FacetError::invalid_arguments(format!("intent spec is not YAML-serializable: {error}"))
        })?;
        store
            .put_desired_state(&token, name, yaml_spec, importance)
            .map_err(hedron_error)?;
        names.push(name.to_owned());
    }
    Ok(IntentAction {
        skipped: false,
        put: names.len(),
        names,
    })
}

fn apply_calls(calls: &Value) -> Result<CallsAction, FacetError> {
    let items = calls
        .get("items")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty());
    if items.is_none() {
        return Ok(CallsAction {
            skipped: true,
            executed: 0,
        });
    }
    Ok(CallsAction {
        skipped: true,
        executed: 0,
    })
}

fn cluster_objects(cluster: &Value) -> Result<Vec<&Value>, FacetError> {
    if cluster.is_null() {
        return Ok(Vec::new());
    }
    let array = cluster
        .as_array()
        .ok_or_else(|| FacetError::invalid_arguments("cluster projection must be an array"))?;
    Ok(array.iter().collect())
}

fn intent_rows(intent: &Value) -> Result<Vec<&Value>, FacetError> {
    if intent.is_null() {
        return Ok(Vec::new());
    }
    let array = intent
        .as_array()
        .ok_or_else(|| FacetError::invalid_arguments("intent projection must be an array"))?;
    Ok(array.iter().collect())
}

fn post_url(api_base: &str, object: &Value) -> Result<String, FacetError> {
    let api_version = object["apiVersion"]
        .as_str()
        .ok_or_else(|| FacetError::invalid_arguments("cluster object requires apiVersion"))?;
    let kind = object["kind"]
        .as_str()
        .ok_or_else(|| FacetError::invalid_arguments("cluster object requires kind"))?;
    if api_version != "v1" {
        return Err(FacetError::invalid_arguments(format!(
            "unsupported apiVersion {api_version} in frozen cluster object"
        )));
    }
    let namespace = object["metadata"]["namespace"]
        .as_str()
        .unwrap_or("default");
    let resource = kind_to_resource(kind)?;
    Ok(format!(
        "{}/{}/namespaces/{}/{}",
        api_base.trim_end_matches('/'),
        "api/v1",
        namespace,
        resource
    ))
}

fn kind_to_resource(kind: &str) -> Result<&'static str, FacetError> {
    match kind {
        "Pod" => Ok("pods"),
        "Service" => Ok("services"),
        other => Err(FacetError::invalid_arguments(format!(
            "unsupported cluster kind {other}"
        ))),
    }
}

fn json_post_request(url: &str, body: &Value) -> HttpRequest {
    HttpRequest {
        metadata: ItemMetadata::default(),
        method: Some("POST".to_owned()),
        url: Some(url.to_owned()),
        headers: vec![Header {
            name: "Content-Type".to_owned(),
            value: "application/json".to_owned(),
            disabled: false,
        }],
        query_parameters: Vec::new(),
        path_parameters: Vec::new(),
        body: Some(RequestBody::Single(Body::Raw(RawBody {
            kind: RawBodyKind::Json,
            data: serde_json::to_string(body).expect("cluster JSON serialization cannot fail"),
        }))),
        authentication: None,
        settings: RequestSettings::default(),
    }
}

fn http_config(error: HttpError) -> FacetError {
    match error {
        HttpError::ClientConfiguration(message) => FacetError::invalid_arguments(message),
        other => FacetError::http(&other),
    }
}

fn hedron_error(error: hedron_core::Error) -> FacetError {
    FacetError::invalid_arguments(error.to_string())
}

fn open_hedron_store(path: &std::path::Path) -> Result<(Store, String), FacetError> {
    let mut store = Store::open(path).map_err(hedron_error)?;
    if let Ok(token) = std::env::var("FACET_HEDRON_TOKEN")
        && !token.is_empty()
    {
        return Ok((store, token));
    }
    if hedrondb_empty(path)? {
        let boot = store
            .bootstrap(
                &hedron_env("FACET_HEDRON_VAULT", "facet"),
                &hedron_env("FACET_HEDRON_AGENT", "facet"),
                &hedron_env("FACET_HEDRON_HTEC", "facet"),
            )
            .map_err(hedron_error)?;
        return Ok((store, boot.token));
    }
    Err(FacetError::invalid_arguments(
        "FACET_HEDRON_TOKEN is required for a non-empty HedronDB store",
    ))
}

fn hedrondb_empty(path: &std::path::Path) -> Result<bool, FacetError> {
    let conn = rusqlite::Connection::open(path).map_err(|error| {
        FacetError::invalid_arguments(format!("cannot open HedronDB store: {error}"))
    })?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
        .map_err(|error| {
            FacetError::invalid_arguments(format!("cannot read HedronDB nodes: {error}"))
        })?;
    Ok(count == 0)
}

fn hedron_env(name: &str, default: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => default.to_owned(),
    }
}
