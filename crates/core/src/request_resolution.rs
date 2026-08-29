use crate::{
    AuthenticationValue, Body, EnvironmentResolutionError, HttpRequest, MultipartValue,
    RequestBody, ResolvedEnvironment,
};

/// Clones a request and interpolates every currently supported request-value field.
pub fn resolve_request(
    request: &HttpRequest,
    environment: &ResolvedEnvironment,
) -> Result<HttpRequest, EnvironmentResolutionError> {
    let mut request = request.clone();
    interpolate_optional(&mut request.method, environment)?;
    interpolate_optional(&mut request.url, environment)?;
    for header in &mut request.headers {
        header.name = environment.interpolate(&header.name)?;
        header.value = environment.interpolate(&header.value)?;
    }
    for parameter in request
        .query_parameters
        .iter_mut()
        .chain(&mut request.path_parameters)
    {
        parameter.name = environment.interpolate(&parameter.name)?;
        parameter.value = environment.interpolate(&parameter.value)?;
    }
    if let Some(body) = &mut request.body {
        resolve_body(body, environment)?;
    }
    if let Some(authentication) = &mut request.authentication {
        for value in authentication.properties.values_mut() {
            resolve_authentication_value(value, environment)?;
        }
    }
    Ok(request)
}

fn interpolate_optional(
    value: &mut Option<String>,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    if let Some(value) = value {
        *value = environment.interpolate(value)?;
    }
    Ok(())
}

fn resolve_body(
    body: &mut RequestBody,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match body {
        RequestBody::Single(body) => resolve_body_value(body, environment),
        RequestBody::Variants(variants) => {
            for variant in variants {
                resolve_body_value(&mut variant.body, environment)?;
            }
            Ok(())
        }
    }
}

fn resolve_body_value(
    body: &mut Body,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match body {
        Body::Raw(body) => body.data = environment.interpolate(&body.data)?,
        Body::FormUrlEncoded(fields) => {
            for field in fields {
                field.name = environment.interpolate(&field.name)?;
                field.value = environment.interpolate(&field.value)?;
            }
        }
        Body::Multipart(parts) => {
            for part in parts {
                part.name = environment.interpolate(&part.name)?;
                match &mut part.value {
                    MultipartValue::Single(value) => *value = environment.interpolate(value)?,
                    MultipartValue::Multiple(values) => {
                        for value in values {
                            *value = environment.interpolate(value)?;
                        }
                    }
                }
                interpolate_optional(&mut part.content_type, environment)?;
            }
        }
        Body::File(files) => {
            for file in files {
                file.file_path = environment.interpolate(&file.file_path)?;
                file.content_type = environment.interpolate(&file.content_type)?;
            }
        }
    }
    Ok(())
}

fn resolve_authentication_value(
    value: &mut AuthenticationValue,
    environment: &ResolvedEnvironment,
) -> Result<(), EnvironmentResolutionError> {
    match value {
        AuthenticationValue::String(value) | AuthenticationValue::Number(value) => {
            *value = environment.interpolate(value)?;
        }
        AuthenticationValue::Sequence(values) => {
            for value in values {
                resolve_authentication_value(value, environment)?;
            }
        }
        AuthenticationValue::Object(values) => {
            for value in values.values_mut() {
                resolve_authentication_value(value, environment)?;
            }
        }
        AuthenticationValue::Boolean(_) | AuthenticationValue::Null => {}
    }
    Ok(())
}
