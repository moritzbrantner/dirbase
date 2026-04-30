use axum::{
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::app::{AppState, ResponseFormat};

pub async fn response_format_middleware(
    axum::extract::State(state): axum::extract::State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    if state.config.response_format != ResponseFormat::Xml || !response_is_json(&response) {
        return response;
    }

    let (mut parts, body) = response.into_parts();
    let body_bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CONTENT_TYPE, "application/xml; charset=utf-8")],
                json_to_xml(&serde_json::json!({
                    "error": format!("Failed to read response body: {err}"),
                    "code": "response_format_error",
                })),
            )
                .into_response();
        }
    };

    let value = match serde_json::from_slice::<Value>(&body_bytes) {
        Ok(value) => value,
        Err(_) => {
            return Response::from_parts(parts, Body::from(body_bytes));
        }
    };

    parts.headers.remove(CONTENT_LENGTH);
    parts
        .headers
        .insert(CONTENT_TYPE, "application/xml; charset=utf-8".parse().expect("content type"));
    Response::from_parts(parts, Body::from(json_to_xml(&value)))
}

fn response_is_json(response: &Response) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_json_content_type)
}

fn is_json_content_type(content_type: &str) -> bool {
    let media_type =
        content_type.split(';').next().map(str::trim).unwrap_or_default().to_ascii_lowercase();
    media_type == "application/json" || media_type.ends_with("+json")
}

fn json_to_xml(value: &Value) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    append_element(&mut xml, "response", &[], value);
    xml.push('\n');
    xml
}

fn append_json_value(xml: &mut String, value: &Value) {
    match value {
        Value::Null => {}
        Value::Bool(value) => xml.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => xml.push_str(&escape_xml_text(&value.to_string())),
        Value::String(value) => xml.push_str(&escape_xml_text(value)),
        Value::Array(items) => {
            for item in items {
                append_named_value(xml, "item", item);
            }
        }
        Value::Object(fields) => {
            for (name, value) in fields {
                append_named_value(xml, name, value);
            }
        }
    }
}

fn append_named_value(xml: &mut String, name: &str, value: &Value) {
    let tag = xml_tag_for_json_key(name);
    match tag {
        XmlTag::Named(name) => append_element(xml, name, &[], value),
        XmlTag::Field(name) => append_element(xml, "field", &[("name", name)], value),
    }
}

fn append_element(xml: &mut String, name: &str, attributes: &[(&str, &str)], value: &Value) {
    xml.push('<');
    xml.push_str(name);
    for (attribute_name, attribute_value) in attributes {
        append_xml_attribute(xml, attribute_name, attribute_value);
    }
    append_xml_attribute(xml, "type", json_type_name(value));
    xml.push('>');
    append_json_value(xml, value);
    xml.push_str("</");
    xml.push_str(name);
    xml.push('>');
}

fn append_xml_attribute(xml: &mut String, name: &str, value: &str) {
    xml.push(' ');
    xml.push_str(name);
    xml.push_str("=\"");
    xml.push_str(&escape_xml_attribute(value));
    xml.push('"');
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(value) if value.is_i64() || value.is_u64() => "integer",
        Value::Number(_) => "float",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

enum XmlTag<'a> {
    Named(&'a str),
    Field(&'a str),
}

fn xml_tag_for_json_key(key: &str) -> XmlTag<'_> {
    if is_xml_name(key) && !key.to_ascii_lowercase().starts_with("xml") {
        XmlTag::Named(key)
    } else {
        XmlTag::Field(key)
    }
}

fn is_xml_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, 'A'..='Z' | 'a'..='z' | '_') {
        return false;
    }
    chars.all(|ch| matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.'))
}

fn escape_xml_text(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_xml_attribute(value: &str) -> String {
    escape_xml_text(value).replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::{is_json_content_type, json_to_xml};

    #[test]
    fn recognizes_json_media_types() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("application/graphql-response+json; charset=utf-8"));
        assert!(!is_json_content_type("text/json"));
        assert!(!is_json_content_type("text/html"));
    }

    #[test]
    fn converts_json_values_to_xml() {
        let xml = json_to_xml(&serde_json::json!({
            "resources": ["users"],
            "count": 1,
            "ok": true,
            "score": 9.5,
            "nothing": null,
            "bad key": "escaped & <safe>",
        }));

        assert!(xml.contains("<response type=\"object\">"), "{xml}");
        assert!(
            xml.contains(
                "<resources type=\"array\"><item type=\"string\">users</item></resources>"
            ),
            "{xml}"
        );
        assert!(xml.contains("<count type=\"integer\">1</count>"), "{xml}");
        assert!(xml.contains("<ok type=\"boolean\">true</ok>"), "{xml}");
        assert!(xml.contains("<score type=\"float\">9.5</score>"), "{xml}");
        assert!(xml.contains("<nothing type=\"null\"></nothing>"), "{xml}");
        assert!(
            xml.contains(
                "<field name=\"bad key\" type=\"string\">escaped &amp; &lt;safe&gt;</field>"
            ),
            "{xml}"
        );
    }

    #[test]
    fn converts_root_scalar_and_array_types_to_xml() {
        assert_eq!(
            json_to_xml(&serde_json::json!("root & value")),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<response type=\"string\">root &amp; value</response>\n"
        );
        assert_eq!(
            json_to_xml(&serde_json::json!([1, 1.25, true, null, "text"])),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<response type=\"array\"><item type=\"integer\">1</item><item type=\"float\">1.25</item><item type=\"boolean\">true</item><item type=\"null\"></item><item type=\"string\">text</item></response>\n"
        );
    }

    #[test]
    fn uses_field_elements_for_keys_that_are_not_safe_xml_names() {
        let xml = json_to_xml(&serde_json::json!({
            "1 bad \"key\"": "quoted",
            "xmlValue": true,
        }));

        assert!(
            xml.contains("<field name=\"1 bad &quot;key&quot;\" type=\"string\">quoted</field>"),
            "{xml}"
        );
        assert!(xml.contains("<field name=\"xmlValue\" type=\"boolean\">true</field>"), "{xml}");
    }
}
