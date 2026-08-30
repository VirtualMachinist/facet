use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanDocument {
    pub(super) info: PostmanInfo,
    #[serde(default)]
    pub(super) item: Vec<PostmanItem>,
    #[serde(default)]
    pub(super) event: Vec<Value>,
    #[serde(default)]
    pub(super) variable: Vec<PostmanVariable>,
    #[serde(default)]
    pub(super) auth: Option<Value>,
    #[serde(default)]
    pub(super) protocol_profile_behavior: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PostmanInfo {
    pub(super) name: String,
    #[serde(rename = "schema")]
    pub(super) _schema: String,
    #[serde(default, rename = "_postman_id")]
    pub(super) postman_id: Option<String>,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(default)]
    pub(super) version: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanItem {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(default)]
    pub(super) variable: Vec<PostmanVariable>,
    #[serde(default)]
    pub(super) event: Vec<Value>,
    #[serde(default)]
    pub(super) auth: Option<Value>,
    #[serde(default)]
    pub(super) item: Option<Vec<PostmanItem>>,
    #[serde(default)]
    pub(super) request: Option<PostmanRequest>,
    #[serde(default)]
    pub(super) response: Vec<Value>,
    #[serde(default)]
    pub(super) protocol_profile_behavior: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum PostmanRequest {
    Url(String),
    Object(Box<PostmanRequestObject>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanRequestObject {
    #[serde(default)]
    pub(super) method: String,
    #[serde(default)]
    pub(super) url: Value,
    #[serde(default)]
    pub(super) auth: Option<Value>,
    #[serde(default)]
    pub(super) proxy: Value,
    #[serde(default)]
    pub(super) certificate: Value,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(default)]
    pub(super) header: Value,
    #[serde(default)]
    pub(super) body: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanUrl {
    #[serde(default)]
    pub(super) raw: Option<String>,
    #[serde(default)]
    pub(super) protocol: Option<String>,
    #[serde(default)]
    pub(super) host: Value,
    #[serde(default)]
    pub(super) path: Value,
    #[serde(default)]
    pub(super) port: String,
    #[serde(default)]
    pub(super) query: Vec<PostmanParameter>,
    #[serde(default)]
    pub(super) hash: String,
    #[serde(default)]
    pub(super) variable: Vec<PostmanVariable>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PostmanParameter {
    #[serde(default)]
    pub(super) key: Value,
    #[serde(default)]
    pub(super) value: Value,
    #[serde(default)]
    pub(super) disabled: bool,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanBody {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) raw: Option<String>,
    #[serde(default)]
    pub(super) urlencoded: Vec<PostmanParameter>,
    #[serde(default)]
    pub(super) formdata: Vec<PostmanFormParameter>,
    #[serde(default)]
    pub(super) file: Option<PostmanFile>,
    #[serde(default)]
    pub(super) graphql: Option<Value>,
    #[serde(default)]
    pub(super) options: Value,
    #[serde(default)]
    pub(super) disabled: bool,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanFormParameter {
    #[serde(default)]
    pub(super) key: Value,
    #[serde(default)]
    pub(super) value: Value,
    #[serde(default)]
    pub(super) src: Value,
    #[serde(default, rename = "type")]
    pub(super) field_type: Option<String>,
    #[serde(default)]
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) disabled: bool,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct PostmanFile {
    #[serde(default)]
    pub(super) src: Value,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PostmanVariable {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) key: String,
    #[serde(default)]
    pub(super) value: Value,
    #[serde(default, rename = "type")]
    pub(super) variable_type: Option<String>,
    #[serde(default)]
    pub(super) description: Value,
    #[serde(default)]
    pub(super) system: bool,
    #[serde(default)]
    pub(super) disabled: bool,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}
