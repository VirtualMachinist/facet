# OpenCollection 1.0 contract pack with Facet secret_ref overlay.
# Schema: https://schema.opencollection.com/opencollection/v1.0.0.json
#
# secret_ref: allowed on environment variables (Facet machine-store refs).
# Plaintext credential fields (token, password, api_key, authorization, and
# OC secret values) are | not_exported — omitted from eval_full_for_export.

let OC_VERSION | force = "1.0.0"

# Plaintext secrets: author-time only; never appear in frozen export.
let PlainSecret | String | not_exported | optional

# Facet machine-store reference (kr:<user> or enc:v1:<blob>).
let SecretRef | String | optional

let Description = String | {
  content | String,
  type | String,
  ..
}

let HttpHeader = {
  name | String,
  value | String | optional,
  disabled | Bool | optional,
  ..
}

let AuthBearer = {
  type | force = "bearer",
  token | PlainSecret,
  authorization | PlainSecret,
  secret_ref | SecretRef,
  ..
}

let AuthBasic = {
  type | force = "basic",
  username | String | optional,
  password | PlainSecret,
  secret_ref | SecretRef,
  ..
}

let AuthApiKey = {
  type | force = "apikey",
  key | String | optional,
  value | PlainSecret,
  api_key | PlainSecret,
  placement | String | optional,
  secret_ref | SecretRef,
  ..
}

let AuthDigest = {
  type | force = "digest",
  username | String | optional,
  password | PlainSecret,
  secret_ref | SecretRef,
  ..
}

let AuthWsse = {
  type | force = "wsse",
  username | String | optional,
  password | PlainSecret,
  secret_ref | SecretRef,
  ..
}

let AuthAwsV4 = {
  type | force = "awsv4",
  accessKeyId | String | optional,
  secretAccessKey | PlainSecret,
  sessionToken | PlainSecret,
  service | String | optional,
  region | String | optional,
  profileName | String | optional,
  secret_ref | SecretRef,
  ..
}

let AuthOAuth1 = {
  type | force = "oauth1",
  consumerKey | String | optional,
  consumerSecret | PlainSecret,
  accessToken | PlainSecret,
  accessTokenSecret | PlainSecret,
  secret_ref | SecretRef,
  ..
}

let AuthOAuth2 = {
  type | force = "oauth2",
  flow | Dyn,
  secret_ref | SecretRef,
  ..
}

let Auth = AuthBearer
  | AuthBasic
  | AuthApiKey
  | AuthDigest
  | AuthWsse
  | AuthAwsV4
  | AuthOAuth1
  | AuthOAuth2
  | "inherit"

let SecretVariable = {
  secret | force = true,
  name | String,
  secret_ref | SecretRef,
  value | PlainSecret,
  description | Description | optional,
  disabled | Bool | optional,
  type | String | optional,
  ..
}

let PlainVariable = {
  name | String,
  value | Dyn | optional,
  secret_ref | SecretRef,
  description | Description | optional,
  disabled | Bool | optional,
  ..
}

let EnvironmentVariable = SecretVariable | PlainVariable

let Environment = {
  name | String,
  color | String | optional,
  description | Description | optional,
  variables | Array EnvironmentVariable | optional,
  extends | String | optional,
  dotEnvFilePath | String | optional,
  ..
}

let HttpRequest = {
  method | String,
  url | String,
  headers | Array HttpHeader | optional,
  auth | Auth | optional,
  ..
}

let ItemInfo = {
  name | String,
  type | String,
  seq | Number | optional,
  ..
}

let Item = {
  info | ItemInfo,
  http | HttpRequest | optional,
  items | Array Item | optional,
  ..
}

let CollectionInfo = {
  name | String,
  summary | String | optional,
  version | String | optional,
  ..
}

let CollectionConfig = {
  environments | Array Environment | optional,
  ..
}

let OpenCollection = {
  opencollection | force = OC_VERSION,
  info | CollectionInfo | optional,
  config | CollectionConfig | optional,
  items | Array Item | optional,
  bundled | Bool | optional,
  ..
}

in {
  OpenCollection,
  OC_VERSION,
  PlainSecret,
  SecretRef,
}
