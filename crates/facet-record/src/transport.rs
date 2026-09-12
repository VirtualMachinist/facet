//! Explicit project kubeconfig loading for Facet's shared HTTP transport.
//! No ambient KUBECONFIG discovery, credential plugins, insecure TLS, or mutation
//! of canonical YAML. Diagnostics never include credential values or YAML text.

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use base64::{Engine as _, prelude::BASE64_STANDARD};
use probe_http::{ClusterTls, HttpEngine, HttpError};
use serde::Deserialize;
use serde_json::Value;

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Builds the shared transport from an explicitly selected project kubeconfig.
/// Without `FACET_KUBECONFIG`, ordinary Facet HTTP behavior is unchanged.
/// `FACET_KUBE_CONTEXT` optionally selects a context within that one file.
pub fn http_engine_from_env() -> Result<HttpEngine, HttpError> {
    let Some(path) = std::env::var_os("FACET_KUBECONFIG") else {
        return HttpEngine::new();
    };
    if path.is_empty() {
        return Err(config("FACET_KUBECONFIG is empty"));
    }
    let context = match std::env::var("FACET_KUBE_CONTEXT") {
        Ok(value) if !value.is_empty() => Some(value),
        Ok(_) => return Err(config("FACET_KUBE_CONTEXT is empty")),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err(config("FACET_KUBE_CONTEXT is not valid UTF-8")),
    };
    http_engine_from_kubeconfig(Path::new(&path), context.as_deref())
}

/// Returns the Kubernetes API server URL from `FACET_KUBECONFIG` when set.
pub fn kube_api_base_from_env() -> Result<Option<String>, HttpError> {
    let Some(path) = std::env::var_os("FACET_KUBECONFIG") else {
        return Ok(None);
    };
    if path.is_empty() {
        return Err(config("FACET_KUBECONFIG is empty"));
    }
    let context = match std::env::var("FACET_KUBE_CONTEXT") {
        Ok(value) if !value.is_empty() => Some(value),
        Ok(_) => return Err(config("FACET_KUBE_CONTEXT is empty")),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err(config("FACET_KUBE_CONTEXT is not valid UTF-8")),
    };
    let resolved = resolve_kubeconfig(Path::new(&path), context.as_deref())?;
    Ok(Some(resolved.cluster.server.trim_end_matches('/').to_owned()))
}

/// Loads a single kubeconfig and creates a verified client-certificate engine.
/// This synchronous filesystem work belongs off the UI event thread.
pub fn http_engine_from_kubeconfig(
    path: &Path,
    selected: Option<&str>,
) -> Result<HttpEngine, HttpError> {
    let resolved = resolve_kubeconfig(path, selected)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let ca = material(base, &resolved.cluster.ca_data, &resolved.cluster.ca_file)?;
    let cert = material(base, &resolved.user.cert_data, &resolved.user.cert_file)?;
    let key = material(base, &resolved.user.key_data, &resolved.user.key_file)?;
    let mut identity = cert;
    identity.push(b'\n');
    identity.extend_from_slice(&key);
    HttpEngine::with_cluster_tls(ClusterTls::from_pem(
        &resolved.cluster.server,
        &ca,
        &identity,
    )?)
}

struct ResolvedKube {
    cluster: Cluster,
    user: User,
}

fn resolve_kubeconfig(path: &Path, selected: Option<&str>) -> Result<ResolvedKube, HttpError> {
    let source = read_bounded(path)?;
    let kube: Kubeconfig =
        serde_yaml_ng::from_slice(&source).map_err(|_| config("invalid kubeconfig document"))?;
    if kube.api_version != "v1" || kube.kind != "Config" {
        return Err(config("kubeconfig requires apiVersion v1 and kind Config"));
    }
    let context_name = selected.unwrap_or(&kube.current_context);
    let context = unique(&kube.contexts, context_name, |entry| &entry.name)?;
    let cluster = unique(&kube.clusters, &context.context.cluster, |entry| &entry.name)?
        .cluster
        .clone();
    let user = unique(&kube.users, &context.context.user, |entry| &entry.name)?
        .user
        .clone();
    if cluster.insecure_skip_tls_verify || !cluster.extra.is_empty() || !user.extra.is_empty() {
        return Err(config(
            "unsupported kubeconfig transport/authentication: use verified CA and client certificate/key; exec, auth-provider, bearer tokens, proxies, TLS-name overrides and impersonation are not supported by this profile",
        ));
    }
    Ok(ResolvedKube { cluster, user })
}

fn unique<'a, T>(
    entries: &'a [T],
    selected: &str,
    name: impl Fn(&T) -> &str,
) -> Result<&'a T, HttpError> {
    if selected.is_empty() {
        return Err(config(
            "kubeconfig context, cluster and user names must be nonempty",
        ));
    }
    let mut matches = entries.iter().filter(|entry| name(entry) == selected);
    let found = matches
        .next()
        .ok_or_else(|| config("selected kubeconfig context, cluster or user is missing"))?;
    if matches.next().is_some() {
        return Err(config("selected kubeconfig name is duplicated"));
    }
    Ok(found)
}

fn material(
    base: &Path,
    data: &Option<String>,
    file: &Option<PathBuf>,
) -> Result<Vec<u8>, HttpError> {
    match (data, file) {
        (Some(data), None) => BASE64_STANDARD
            .decode(data)
            .map_err(|_| config("invalid base64 credential data in kubeconfig")),
        (None, Some(file)) => read_bounded(&base.join(file)),
        _ => Err(config(
            "kubeconfig requires exactly one data or file source for each CA, client certificate and private key",
        )),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, HttpError> {
    let file = File::open(path)
        .map_err(|_| config("cannot open kubeconfig or referenced credential file"))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| config("cannot read kubeconfig or referenced credential file"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(config("kubeconfig or credential file exceeds 4 MiB"));
    }
    Ok(bytes)
}

fn config(message: &str) -> HttpError {
    HttpError::ClientConfiguration(message.to_owned())
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Kubeconfig {
    #[serde(rename = "apiVersion")]
    api_version: String,
    kind: String,
    #[serde(default)]
    current_context: String,
    clusters: Vec<NamedCluster>,
    users: Vec<NamedUser>,
    contexts: Vec<NamedContext>,
}
#[derive(Deserialize)]
struct NamedCluster {
    name: String,
    cluster: Cluster,
}
#[derive(Deserialize)]
struct NamedUser {
    name: String,
    user: User,
}
#[derive(Deserialize)]
struct NamedContext {
    name: String,
    context: Context,
}
#[derive(Deserialize)]
struct Context {
    cluster: String,
    user: String,
}
#[derive(Deserialize, Clone)]
struct Cluster {
    server: String,
    #[serde(default, rename = "certificate-authority-data")]
    ca_data: Option<String>,
    #[serde(default, rename = "certificate-authority")]
    ca_file: Option<PathBuf>,
    #[serde(default, rename = "insecure-skip-tls-verify")]
    insecure_skip_tls_verify: bool,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}
#[derive(Deserialize, Clone)]
struct User {
    #[serde(default, rename = "client-certificate-data")]
    cert_data: Option<String>,
    #[serde(default, rename = "client-certificate")]
    cert_file: Option<PathBuf>,
    #[serde(default, rename = "client-key-data")]
    key_data: Option<String>,
    #[serde(default, rename = "client-key")]
    key_file: Option<PathBuf>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}
