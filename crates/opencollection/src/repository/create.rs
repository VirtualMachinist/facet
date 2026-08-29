use super::*;
use serde::Serialize;

/// Creates an empty bundled OpenCollection YAML file at `path` and loads it.
///
/// When `path` has no extension, `.yml` is appended. `name` overrides the collection
/// title; otherwise the file stem is used. Existing files are left unchanged unless
/// `replace` is true. Directories are rejected.
pub fn create_bundled_workspace(
    path: impl AsRef<Path>,
    name: Option<&str>,
    replace: bool,
) -> Result<LoadedWorkspace, CreateError> {
    let path = with_yaml_extension(path.as_ref());
    if path.is_dir() {
        return Err(CreateError::IsDirectory(path));
    }
    if path.exists() && !replace {
        return Err(CreateError::AlreadyExists(path));
    }
    let collection_name = collection_name_from_path(&path, name);
    let source = bundled_collection_yaml(&collection_name)?;
    write_collection_file(&path, source.as_bytes())?;
    load_workspace(&path).map_err(CreateError::Load)
}

/// Creates a bundled OpenCollection workspace from a domain collection.
///
/// The destination is written atomically and is never overwritten. The resulting
/// document is loaded again before success is returned, so callers never receive a
/// workspace that the repository itself cannot read.
pub fn create_bundled_workspace_from_collection(
    path: impl AsRef<Path>,
    collection: &probe_core::Collection,
) -> Result<LoadedWorkspace, CreateError> {
    let path = with_yaml_extension(path.as_ref());
    if path.is_dir() {
        return Err(CreateError::IsDirectory(path));
    }
    let document = bundled_collection_value(collection);
    let source = serde_yaml_ng::to_string(&document).map_err(CreateError::Serialize)?;
    let source = source.strip_prefix("---\n").unwrap_or(&source);
    // Validate the complete document before creating the destination. This keeps
    // conversion errors from leaving a newly-created but unusable collection behind.
    load_workspace_from_str(source).map_err(CreateError::Load)?;
    write_new_collection_file(&path, source.as_bytes())?;
    load_workspace(&path).map_err(CreateError::Load)
}

/// Writes a completed document to a previously nonexistent path without ever
/// replacing a concurrently-created destination.
///
/// A temporary file is fully flushed before it is hard-linked into place. Creating
/// the final link is an atomic create-if-absent operation on the destination
/// filesystem, unlike an atomic rename which would replace an existing path.
fn write_new_collection_file(path: &Path, contents: &[u8]) -> Result<(), CreateError> {
    match write_new_file(path, contents) {
        Ok(()) => Ok(()),
        Err(NewFileError::AlreadyExists) => Err(CreateError::AlreadyExists(path.to_owned())),
        Err(NewFileError::Io(source)) => Err(CreateError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn with_yaml_extension(path: &Path) -> PathBuf {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml") =>
        {
            path.to_owned()
        }
        Some(_) => path.to_owned(),
        None => path.with_extension("yml"),
    }
}

fn collection_name_from_path(path: &Path, name: Option<&str>) -> String {
    if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Collection")
        .to_owned()
}

fn bundled_collection_yaml(name: &str) -> Result<String, CreateError> {
    let document = NewBundledCollection {
        opencollection: "1.0.0",
        info: NewCollectionInfo { name },
        bundled: true,
    };
    let yaml = serde_yaml_ng::to_string(&document).map_err(CreateError::Serialize)?;
    Ok(yaml
        .strip_prefix("---\n")
        .unwrap_or(&yaml)
        .trim_start()
        .to_owned())
}

fn write_collection_file(path: &Path, contents: &[u8]) -> Result<(), CreateError> {
    let map_io = |source| CreateError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(map_io)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(map_io)?;
    file.write_all(contents).map_err(map_io)?;
    file.sync_all().map_err(map_io)?;
    file.commit().map_err(map_io)?;
    Ok(())
}

#[derive(Serialize)]
struct NewBundledCollection<'a> {
    opencollection: &'a str,
    info: NewCollectionInfo<'a>,
    bundled: bool,
}

#[derive(Serialize)]
struct NewCollectionInfo<'a> {
    name: &'a str,
}

fn bundled_collection_value(collection: &probe_core::Collection) -> Value {
    let mut root = serde_yaml_ng::Mapping::new();
    root.insert(
        string_key("opencollection"),
        Value::String("1.0.0".to_owned()),
    );
    root.insert(
        string_key("info"),
        collection_info_value(&collection.metadata),
    );
    root.insert(string_key("bundled"), Value::Bool(true));
    if !collection.environments.is_empty() {
        root.insert(
            string_key("config"),
            map([(
                "environments",
                Value::Sequence(
                    collection
                        .environments
                        .iter()
                        .map(environment_value)
                        .collect(),
                ),
            )]),
        );
    }
    root.insert(
        string_key("items"),
        Value::Sequence(collection.items.iter().map(collection_item_value).collect()),
    );
    Value::Mapping(root)
}

fn collection_info_value(metadata: &probe_core::CollectionMetadata) -> Value {
    let mut info = serde_yaml_ng::Mapping::new();
    if let Some(name) = &metadata.name {
        info.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(summary) = &metadata.summary {
        info.insert(string_key("summary"), Value::String(summary.clone()));
    }
    if let Some(version) = &metadata.version {
        info.insert(string_key("version"), Value::String(version.clone()));
    }
    if !metadata.authors.is_empty() {
        info.insert(
            string_key("authors"),
            Value::Sequence(
                metadata
                    .authors
                    .iter()
                    .map(|author| {
                        let mut value = serde_yaml_ng::Mapping::new();
                        if let Some(name) = &author.name {
                            value.insert(string_key("name"), Value::String(name.clone()));
                        }
                        if let Some(email) = &author.email {
                            value.insert(string_key("email"), Value::String(email.clone()));
                        }
                        if let Some(url) = &author.url {
                            value.insert(string_key("url"), Value::String(url.clone()));
                        }
                        Value::Mapping(value)
                    })
                    .collect(),
            ),
        );
    }
    Value::Mapping(info)
}

fn collection_item_value(item: &CollectionItem) -> Value {
    match item {
        CollectionItem::Folder(folder) => {
            let mut item = item_info_value(&folder.metadata, "folder");
            item.insert(
                string_key("items"),
                Value::Sequence(folder.items.iter().map(collection_item_value).collect()),
            );
            Value::Mapping(item)
        }
        CollectionItem::HttpRequest(request) => {
            let mut item = item_info_value(&request.metadata, "http");
            let mut http = serde_yaml_ng::Mapping::new();
            if let Some(method) = &request.method {
                http.insert(string_key("method"), Value::String(method.clone()));
            }
            if let Some(url) = &request.url {
                http.insert(string_key("url"), Value::String(url.clone()));
            }
            if !request.headers.is_empty() {
                http.insert(
                    string_key("headers"),
                    Value::Sequence(request.headers.iter().map(header_value).collect()),
                );
            }
            let parameters = request
                .query_parameters
                .iter()
                .map(query_parameter_value)
                .chain(request.path_parameters.iter().map(path_parameter_value))
                .collect::<Vec<_>>();
            if !parameters.is_empty() {
                http.insert(string_key("params"), Value::Sequence(parameters));
            }
            if let Some(body) = &request.body {
                http.insert(string_key("body"), request_body_value(body));
            }
            if let Some(authentication) = &request.authentication {
                http.insert(string_key("auth"), authentication_value(authentication));
            }
            item.insert(string_key("http"), Value::Mapping(http));
            if let Some(settings) = request_settings_value(&request.settings) {
                item.insert(string_key("settings"), settings);
            }
            Value::Mapping(item)
        }
    }
}

fn item_info_value(metadata: &probe_core::ItemMetadata, item_type: &str) -> serde_yaml_ng::Mapping {
    let mut info = serde_yaml_ng::Mapping::new();
    info.insert(string_key("type"), Value::String(item_type.to_owned()));
    if let Some(name) = &metadata.name {
        info.insert(string_key("name"), Value::String(name.clone()));
    }
    if let Some(sequence) = metadata.sequence
        && let Ok(value) = serde_yaml_ng::to_value(sequence)
    {
        info.insert(string_key("seq"), value);
    }
    let mut item = serde_yaml_ng::Mapping::new();
    item.insert(string_key("info"), Value::Mapping(info));
    item
}

fn request_settings_value(settings: &probe_core::RequestSettings) -> Option<Value> {
    let mut value = serde_yaml_ng::Mapping::new();
    if let Some(timeout) = settings.timeout
        && let Ok(timeout) = serde_yaml_ng::to_value(timeout.as_secs_f64() * 1000.0)
    {
        value.insert(string_key("timeout"), timeout);
    }
    if let Some(follow_redirects) = settings.follow_redirects {
        value.insert(string_key("followRedirects"), Value::Bool(follow_redirects));
    }
    if let Some(max_redirects) = settings.max_redirects
        && let Ok(max_redirects) = serde_yaml_ng::to_value(max_redirects)
    {
        value.insert(string_key("maxRedirects"), max_redirects);
    }
    (!value.is_empty()).then(|| Value::Mapping(value))
}

pub(super) fn environment_value(environment: &Environment) -> Value {
    let mut value = serde_yaml_ng::Mapping::new();
    value.insert(string_key("name"), Value::String(environment.name.clone()));
    if let Some(color) = &environment.color {
        value.insert(string_key("color"), Value::String(color.clone()));
    }
    if let Some(extends) = &environment.extends {
        value.insert(string_key("extends"), Value::String(extends.clone()));
    }
    if let Some(path) = &environment.dot_env_file_path {
        value.insert(string_key("dotEnvFilePath"), Value::String(path.clone()));
    }
    if !environment.variables.is_empty() {
        value.insert(
            string_key("variables"),
            Value::Sequence(
                environment
                    .variables
                    .iter()
                    .map(environment_variable_value)
                    .collect(),
            ),
        );
    }
    Value::Mapping(value)
}

fn environment_variable_value(variable: &EnvironmentVariable) -> Value {
    match variable {
        EnvironmentVariable::Plain(variable) => new_environment_variable_value(variable),
        EnvironmentVariable::Secret(variable) => {
            let mut value = serde_yaml_ng::Mapping::new();
            if let Some(name) = &variable.name {
                value.insert(string_key("name"), Value::String(name.clone()));
            }
            value.insert(string_key("secret"), Value::Bool(true));
            if let Some(kind) = &variable.value_type {
                value.insert(string_key("type"), Value::String(kind.as_str().to_owned()));
            }
            if variable.disabled {
                value.insert(string_key("disabled"), Value::Bool(true));
            }
            Value::Mapping(value)
        }
    }
}
