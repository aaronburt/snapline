use serde_json::Value;
use serde_json_path::JsonPath;
use snapline_core::naming::{ThreadRandom, TokenSource};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("The server response is not valid JSON.")]
    InvalidJson,
    #[error("The JSON path in the uploader is not valid.")]
    InvalidPath,
    #[error("The uploader template is not valid.")]
    InvalidTemplate,
}

pub struct TemplateContext<'a> {
    pub file_name: &'a str,
    pub random: &'a str,
    pub response: Option<&'a str>,
    pub response_json: Option<&'a Value>,
}

pub fn expand(
    template: &str,
    context: &TemplateContext<'_>,
    encode_values: bool,
) -> Result<String, TemplateError> {
    let migrated = migrate_legacy(template);
    let mut output = String::with_capacity(migrated.len());
    let mut rest = migrated.as_str();
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let token = &after[..end];
        match placeholder(token, context)? {
            Some(value) => {
                if encode_values {
                    output.push_str(&encode_component(&value));
                } else {
                    output.push_str(&value);
                }
            }
            None => {
                output.push('{');
                output.push_str(token);
                output.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

pub fn validate(template: &str) -> Result<(), TemplateError> {
    let migrated = migrate_legacy(template);
    let mut rest = migrated.as_str();
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Ok(());
        };
        let token = &after[..end];
        if let Some(path) = json_token_path(token) {
            validate_path(path)?;
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

pub fn json_path(response: &str, path: &str) -> Result<Option<String>, TemplateError> {
    let value: Value = serde_json::from_str(response).map_err(|_| TemplateError::InvalidJson)?;
    json_path_value(&value, path)
}

pub fn json_path_value(value: &Value, path: &str) -> Result<Option<String>, TemplateError> {
    let normalized = normalized_path(path)?;
    let parsed = JsonPath::parse(&normalized).map_err(|_| TemplateError::InvalidPath)?;
    Ok(parsed.query(value).first().map(node_to_string))
}

pub fn migrate_legacy(template: &str) -> String {
    if !template.contains('$') {
        return template.to_owned();
    }
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let inner = &after[..end];
        if inner.eq_ignore_ascii_case("response") {
            output.push_str("{response}");
        } else if inner.eq_ignore_ascii_case("filename") {
            output.push_str("{filename}");
        } else if inner.eq_ignore_ascii_case("random") {
            output.push_str("{random}");
        } else if json_token_path(inner).is_some() {
            output.push('{');
            output.push_str(inner);
            output.push('}');
        } else {
            output.push('$');
            output.push_str(inner);
            output.push('$');
        }
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    output
}

pub fn random_token() -> String {
    ThreadRandom.random_chars()
}

fn placeholder(
    token: &str,
    context: &TemplateContext<'_>,
) -> Result<Option<String>, TemplateError> {
    if token.eq_ignore_ascii_case("filename") {
        return Ok(Some(context.file_name.to_owned()));
    }
    if token.eq_ignore_ascii_case("random") {
        return Ok(Some(context.random.to_owned()));
    }
    if token.eq_ignore_ascii_case("response") {
        return Ok(Some(context.response.unwrap_or("").to_owned()));
    }
    if let Some(path) = json_token_path(token) {
        let value = context.response_json.ok_or(TemplateError::InvalidJson)?;
        return json_path_value(value, path).map(|value| Some(value.unwrap_or_default()));
    }
    Ok(None)
}

fn json_token_path(token: &str) -> Option<&str> {
    let prefix = token.get(..5);
    if prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case("json:")) {
        Some(&token[5..])
    } else {
        None
    }
}

fn normalized_path(path: &str) -> Result<String, TemplateError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(TemplateError::InvalidPath);
    }
    if path.starts_with('$') {
        Ok(path.to_owned())
    } else if path.starts_with('[') {
        Ok(format!("${path}"))
    } else {
        Ok(format!("$.{path}"))
    }
}

fn validate_path(path: &str) -> Result<(), TemplateError> {
    let normalized = normalized_path(path)?;
    JsonPath::parse(&normalized).map_err(|_| TemplateError::InvalidPath)?;
    Ok(())
}

fn node_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn encode_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn context<'a>(
        file_name: &'a str,
        random: &'a str,
        response: Option<&'a str>,
        response_json: Option<&'a Value>,
    ) -> TemplateContext<'a> {
        TemplateContext {
            file_name,
            random,
            response,
            response_json,
        }
    }

    #[test]
    fn filename_and_random_expand() {
        let context = context("shot one.png", "a1b2", None, None);
        let value = expand("https://example.test/{filename}/{random}", &context, true);
        assert_eq!(
            value,
            Ok("https://example.test/shot%20one.png/a1b2".to_owned())
        );
    }

    #[test]
    fn response_placeholder_returns_the_body() {
        let body = r#"{"ok":true}"#;
        let context = context("shot.png", "abcd", Some(body), None);
        assert_eq!(expand("{response}", &context, false), Ok(body.to_owned()));
    }

    #[test]
    fn json_path_extracts_nested_values() {
        let value = json!({"data": {"url": "https://i.example/a.png", "n": 3}});
        let context = context("shot.png", "abcd", None, Some(&value));
        assert_eq!(
            expand("{json:$.data.url}", &context, false),
            Ok("https://i.example/a.png".to_owned())
        );
        assert_eq!(expand("{json:data.n}", &context, false), Ok("3".to_owned()));
        assert_eq!(expand("{json:[0]}", &context, false), Ok(String::new()));
    }

    #[test]
    fn json_path_accepts_a_dollar_less_path() {
        let value = json!({"data": {"url": "https://i.example/a.png"}});
        assert_eq!(
            json_path_value(&value, "data.url"),
            Ok(Some("https://i.example/a.png".to_owned()))
        );
    }

    #[test]
    fn json_path_rejects_an_empty_path() {
        let value = json!({"data": {}});
        assert_eq!(
            json_path_value(&value, " "),
            Err(TemplateError::InvalidPath)
        );
    }

    #[test]
    fn invalid_json_is_reported() {
        let context = context("shot.png", "abcd", Some("not json"), None);
        assert_eq!(
            expand("{json:$.url}", &context, false),
            Err(TemplateError::InvalidJson)
        );
    }

    #[test]
    fn legacy_sharex_syntax_is_migrated() {
        assert_eq!(migrate_legacy("$json:data.url$"), "{json:data.url}");
        assert_eq!(migrate_legacy("$response$"), "{response}");
        assert_eq!(
            migrate_legacy("https://example.test/$filename$"),
            "https://example.test/{filename}"
        );
        assert_eq!(migrate_legacy("cost $5"), "cost $5");
    }

    #[test]
    fn unknown_placeholders_are_left_alone() {
        let context = context("shot.png", "abcd", None, None);
        assert_eq!(
            expand("https://example.test/{unknown}", &context, true),
            Ok("https://example.test/{unknown}".to_owned())
        );
    }

    #[test]
    fn validation_accepts_good_paths_and_rejects_bad_ones() {
        assert_eq!(validate("{json:$.data.url}"), Ok(()));
        assert_eq!(validate("https://example.test/{filename}"), Ok(()));
        assert!(validate("{json:}").is_err());
        assert!(validate("{json:$.data[}").is_err());
    }

    #[test]
    fn random_tokens_are_four_lowercase_letters_or_digits() {
        for _ in 0..32 {
            let token = random_token();
            assert_eq!(token.len(), 4);
            assert!(token
                .chars()
                .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit()));
        }
    }
}
