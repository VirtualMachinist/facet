use std::fs;

use probe_core::{
    Authentication, AuthenticationKind, AuthenticationValue, Body, Collection, CollectionItem,
    CollectionMetadata, EnvironmentResolutionError, FormField, Header, HttpRequest, ItemMetadata,
    QueryParameter, RequestBody, RequestUpdate, resolve_environment,
};
use probe_opencollection::{
    CreateError, SaveError, StructureError, StructureOperation, create_bundled_workspace,
    create_bundled_workspace_from_collection, load_workspace, load_workspace_from_str,
};

mod support;

use support::{copy_directory, fixture, temporary_path};

#[path = "repository/environment_persistence.rs"]
mod environment_persistence;
#[path = "repository/request_persistence.rs"]
mod request_persistence;
#[path = "repository/structure_persistence.rs"]
mod structure_persistence;
#[path = "repository/workspace_creation.rs"]
mod workspace_creation;
