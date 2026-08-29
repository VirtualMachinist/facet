use std::ops::Range;

use crate::{HttpRequest, QueryParameter};

enum PathSegmentAction {
    Keep,
    ReplaceValue(String),
    ReplaceName(String),
    Remove,
}

/// Ordered, deduplicated `:variableName` placeholders parsed from the URL path.
pub(crate) fn path_variable_names(url: &str) -> Vec<String> {
    path_variable_ranges(url)
        .into_iter()
        .map(|(_, name)| name)
        .fold(Vec::new(), |mut names, name| {
            if !names.iter().any(|existing| existing == &name) {
                names.push(name);
            }
            names
        })
}

/// Byte ranges and names for `:placeholder` segments in the URL path.
pub fn path_variable_ranges(url: &str) -> Vec<(Range<usize>, String)> {
    let bounds = path_bounds(url);
    let path = &url[bounds.clone()];
    let path_start = bounds.start;
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while let Some(relative_colon) = path[cursor..].find(':') {
        let colon = cursor + relative_colon;
        let name_start = colon + 1;
        let name_end = path[name_start..]
            .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .map_or(path.len(), |end| name_start + end);
        let name = &path[name_start..name_end];
        if is_valid_path_variable_name(name) {
            ranges.push((path_start + colon..path_start + name_end, name.to_owned()));
        }
        cursor = name_start.max(name_end);
    }
    ranges
}

/// Substitutes enabled path parameter values into `:name` placeholders.
pub fn apply_path_parameters(url: &str, parameters: &[QueryParameter]) -> String {
    rewrite_path_segments(url, |name| {
        if let Some(parameter) = parameters
            .iter()
            .find(|parameter| !parameter.disabled && parameter.name == name)
        {
            PathSegmentAction::ReplaceValue(encode_path_segment(&parameter.value))
        } else {
            PathSegmentAction::Keep
        }
    })
}

/// Aligns `path_parameters` with URL placeholders after the URL changes.
///
/// Enabled rows that no longer appear in the URL are removed.
pub fn synchronize_path_parameters(request: &mut HttpRequest) -> bool {
    synchronize_path_parameters_with_options(request, true)
}

/// Adds missing path-parameter rows for URL placeholders without removing extras.
pub fn ensure_path_parameters_from_url(request: &mut HttpRequest) -> bool {
    synchronize_path_parameters_with_options(request, false)
}

/// Renames a path parameter and updates matching URL placeholders.
pub fn rename_path_parameter_at(request: &mut HttpRequest, index: usize, new_name: &str) -> bool {
    let Some(old_name) = request
        .path_parameters
        .get(index)
        .map(|parameter| parameter.name.clone())
    else {
        return false;
    };
    if old_name == new_name {
        return false;
    }
    if let Some(url) = request.url.as_mut() {
        *url = rewrite_path_segments(url, |name| {
            if name == old_name.as_str() {
                PathSegmentAction::ReplaceName(new_name.to_owned())
            } else {
                PathSegmentAction::Keep
            }
        });
    }
    request.path_parameters[index].name = new_name.to_owned();
    synchronize_path_parameters_with_options(request, false)
}

/// Removes a path parameter and, when active, its URL placeholders.
pub fn remove_path_parameter_at(request: &mut HttpRequest, index: usize) -> bool {
    let Some(parameter) = request.path_parameters.get(index).cloned() else {
        return false;
    };
    request.path_parameters.remove(index);
    if !parameter.disabled && !parameter.name.is_empty() {
        if let Some(url) = request.url.as_mut() {
            *url = rewrite_path_segments(url, |name| {
                if name == parameter.name.as_str() {
                    PathSegmentAction::Remove
                } else {
                    PathSegmentAction::Keep
                }
            });
        }
        synchronize_path_parameters_with_options(request, true);
    }
    true
}

/// Appends a new `:param` placeholder to the URL and creates a matching row.
pub fn add_path_parameter(request: &mut HttpRequest) {
    let name = unique_path_parameter_name(request);
    if let Some(url) = request.url.as_mut() {
        *url = append_path_variable(url, &name);
    } else {
        request.url = Some(format!("/:{name}"));
    }
    synchronize_path_parameters_with_options(request, false);
}

fn synchronize_path_parameters_with_options(request: &mut HttpRequest, remove_stale: bool) -> bool {
    let before = request.path_parameters.clone();
    let names = request
        .url
        .as_deref()
        .map(path_variable_names)
        .unwrap_or_default();
    let mut existing = std::mem::take(&mut request.path_parameters);
    let mut synchronized = Vec::with_capacity(names.len() + existing.len());
    for name in names {
        if let Some(index) = existing.iter().position(|parameter| parameter.name == name) {
            synchronized.push(existing.remove(index));
        } else {
            synchronized.push(QueryParameter {
                name,
                value: String::new(),
                disabled: false,
            });
        }
    }
    if remove_stale {
        synchronized.extend(existing.into_iter().filter(|parameter| parameter.disabled));
    } else {
        synchronized.extend(existing);
    }
    request.path_parameters = synchronized;
    request.path_parameters != before
}

fn path_bounds(url: &str) -> Range<usize> {
    let path_end = url.find(['?', '#']).unwrap_or(url.len());
    let path_start = url.find("://").map_or(0, |scheme| {
        url[scheme + 3..path_end]
            .find('/')
            .map_or(path_end, |path| scheme + 3 + path)
    });
    path_start..path_end
}

fn is_valid_path_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
}

fn unique_path_parameter_name(request: &HttpRequest) -> String {
    let url_names = request
        .url
        .as_deref()
        .map(path_variable_names)
        .unwrap_or_default();
    for index in 1.. {
        let candidate = if index == 1 {
            "param".to_owned()
        } else {
            format!("param{index}")
        };
        if !request
            .path_parameters
            .iter()
            .any(|parameter| parameter.name == candidate)
            && !url_names.iter().any(|name| name == &candidate)
        {
            return candidate;
        }
    }
    unreachable!("path parameter names are bounded")
}

fn append_path_variable(url: &str, name: &str) -> String {
    let (without_fragment, fragment) = url.split_once('#').unwrap_or((url, ""));
    let (base, query) = without_fragment
        .split_once('?')
        .unwrap_or((without_fragment, ""));
    let mut path = base.to_owned();
    if path.ends_with('/') {
        path.push_str(&format!(":{name}"));
    } else {
        path.push_str(&format!("/:{name}"));
    }
    let query_part = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    let fragment_part = if fragment.is_empty() {
        String::new()
    } else {
        format!("#{fragment}")
    };
    format!("{path}{query_part}{fragment_part}")
}

fn rewrite_path_segments(
    url: &str,
    mut transform: impl FnMut(&str) -> PathSegmentAction,
) -> String {
    let bounds = path_bounds(url);
    let mut result = String::with_capacity(url.len());
    result.push_str(&url[..bounds.start]);
    let path = &url[bounds.clone()];
    let bytes = path.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let segment_start = cursor;
        let has_slash = bytes[cursor] == b'/';
        let colon = if has_slash { cursor + 1 } else { cursor };
        if colon < bytes.len() && bytes[colon] == b':' {
            let name_start = colon + 1;
            let name_end = path[name_start..]
                .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .map_or(path.len(), |end| name_start + end);
            let name = &path[name_start..name_end];
            if is_valid_path_variable_name(name) {
                match transform(name) {
                    PathSegmentAction::Keep => {
                        result.push_str(&path[segment_start..name_end]);
                    }
                    PathSegmentAction::ReplaceValue(value) => {
                        if has_slash {
                            result.push('/');
                        }
                        result.push_str(&value);
                    }
                    PathSegmentAction::ReplaceName(new_name) => {
                        if has_slash {
                            result.push('/');
                        }
                        result.push(':');
                        result.push_str(&new_name);
                    }
                    PathSegmentAction::Remove => {}
                }
                cursor = name_end;
                continue;
            }
        }
        let character = path[cursor..]
            .chars()
            .next()
            .expect("cursor must be within the path");
        result.push(character);
        cursor += character.len_utf8();
    }
    result.push_str(&url[bounds.end..]);
    result
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use crate::{HttpRequest, QueryParameter};

    use super::{
        add_path_parameter, apply_path_parameters, ensure_path_parameters_from_url,
        path_variable_names, path_variable_ranges, remove_path_parameter_at,
        rename_path_parameter_at, synchronize_path_parameters,
    };

    #[test]
    fn path_variable_ranges_ignore_ports_query_strings_and_invalid_names() {
        let value = "https://api.example.com:8443/users/:userId/posts/:post_id?next=:ignored";
        let ranges = path_variable_ranges(value);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&value[ranges[0].0.clone()], ":userId");
        assert_eq!(ranges[0].1, "userId");
        assert_eq!(&value[ranges[1].0.clone()], ":post_id");
        assert_eq!(ranges[1].1, "post_id");
        assert_eq!(
            path_variable_names(value),
            vec!["userId".to_owned(), "post_id".to_owned()]
        );
    }

    #[test]
    fn apply_path_parameters_substitutes_enabled_values() {
        let url = apply_path_parameters(
            "https://api.example.com/users/:userId",
            &[QueryParameter {
                name: "userId".to_owned(),
                value: "probe/user".to_owned(),
                disabled: false,
            }],
        );
        assert_eq!(url, "https://api.example.com/users/probe%2Fuser");
    }

    #[test]
    fn rewriting_path_parameters_preserves_unicode() {
        let url = apply_path_parameters(
            "https://api.example.com/café/:userId",
            &[QueryParameter {
                name: "userId".to_owned(),
                value: "probe".to_owned(),
                disabled: false,
            }],
        );
        assert_eq!(url, "https://api.example.com/café/probe");
    }

    #[test]
    fn synchronize_path_parameters_removes_stale_enabled_rows() {
        let mut request = HttpRequest {
            path_parameters: vec![QueryParameter {
                name: "stale".to_owned(),
                value: "old".to_owned(),
                disabled: false,
            }],
            ..HttpRequest::default()
        };
        request.url = Some("https://api.example.com/users/:userId".to_owned());
        synchronize_path_parameters(&mut request);
        assert_eq!(request.path_parameters.len(), 1);
        assert_eq!(request.path_parameters[0].name, "userId");
    }

    #[test]
    fn ensure_path_parameters_from_url_adds_placeholders_and_preserves_extra_yaml_rows() {
        let mut request = HttpRequest {
            url: Some("https://api.example.com/users/:userId".to_owned()),
            path_parameters: vec![QueryParameter {
                name: "ownerId".to_owned(),
                value: "42".to_owned(),
                disabled: false,
            }],
            ..HttpRequest::default()
        };
        ensure_path_parameters_from_url(&mut request);
        assert_eq!(request.path_parameters.len(), 2);
        assert_eq!(request.path_parameters[0].name, "userId");
        assert_eq!(request.path_parameters[1].name, "ownerId");
        assert_eq!(request.path_parameters[1].value, "42");
    }

    #[test]
    fn rename_path_parameter_at_updates_the_url_and_row() {
        let mut request = HttpRequest {
            url: Some("https://api.example.com/users/:userId/posts/:userId".to_owned()),
            path_parameters: vec![QueryParameter {
                name: "userId".to_owned(),
                value: "42".to_owned(),
                disabled: false,
            }],
            ..HttpRequest::default()
        };
        rename_path_parameter_at(&mut request, 0, "accountId");
        assert_eq!(
            request.url.as_deref(),
            Some("https://api.example.com/users/:accountId/posts/:accountId")
        );
        assert_eq!(request.path_parameters[0].name, "accountId");
    }

    #[test]
    fn remove_path_parameter_at_removes_url_segments() {
        let mut request = HttpRequest {
            url: Some("https://api.example.com/users/:userId/posts/:postId".to_owned()),
            path_parameters: vec![
                QueryParameter {
                    name: "userId".to_owned(),
                    value: "42".to_owned(),
                    disabled: false,
                },
                QueryParameter {
                    name: "postId".to_owned(),
                    value: "7".to_owned(),
                    disabled: false,
                },
            ],
            ..HttpRequest::default()
        };
        remove_path_parameter_at(&mut request, 0);
        assert_eq!(
            request.url.as_deref(),
            Some("https://api.example.com/users/posts/:postId")
        );
        assert_eq!(request.path_parameters.len(), 1);
        assert_eq!(request.path_parameters[0].name, "postId");
    }

    #[test]
    fn add_path_parameter_appends_a_unique_placeholder() {
        let mut request = HttpRequest {
            url: Some("https://api.example.com/users".to_owned()),
            ..HttpRequest::default()
        };
        add_path_parameter(&mut request);
        assert_eq!(
            request.url.as_deref(),
            Some("https://api.example.com/users/:param")
        );
        assert_eq!(request.path_parameters[0].name, "param");
    }
}
