use std::collections::BTreeMap;

pub(crate) fn relation_field_name(source_column: &str) -> String {
    let mut candidate = source_column.to_string();
    for suffix in ["_id", "Id", "ID"] {
        if let Some(stripped) = candidate.strip_suffix(suffix) {
            candidate = stripped.to_string();
            break;
        }
    }
    for suffix in ["_ref", "Ref"] {
        if let Some(stripped) = candidate.strip_suffix(suffix) {
            candidate = stripped.to_string();
            break;
        }
    }
    if candidate.is_empty() || candidate == source_column {
        return format!("{source_column}Ref");
    }
    candidate
}

pub(crate) fn normalize_graphql_name(raw: &str) -> String {
    let mut normalized = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' })
        .collect::<String>();

    if normalized.is_empty() {
        normalized.push('x');
    }
    if normalized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        normalized = format!("n_{normalized}");
    }
    if normalized.starts_with("__") {
        normalized = format!("x_{normalized}");
    }
    normalized
}

pub(crate) fn collection_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Record", pascalize(resource)))
}

pub(crate) fn collection_page_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Page", pascalize(resource)))
}

pub(crate) fn object_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Object", pascalize(resource)))
}

pub(crate) fn normalize_graphql_type_name(raw: &str) -> String {
    let mut normalized = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' })
        .collect::<String>();

    if normalized.is_empty() {
        normalized.push('X');
    }
    if normalized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        normalized = format!("N{normalized}");
    }
    if normalized.starts_with("__") {
        normalized = format!("X{normalized}");
    }
    normalized
}

pub(crate) fn pascalize(raw: &str) -> String {
    let mut out = String::new();
    let mut uppercase_next = true;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if uppercase_next {
                out.push(ch.to_ascii_uppercase());
                uppercase_next = false;
            } else {
                out.push(ch);
            }
        } else {
            uppercase_next = true;
        }
    }

    if out.is_empty() {
        out.push('X');
    }
    out
}

pub(crate) fn register_graphql_name(
    seen: &mut BTreeMap<String, String>,
    normalized: String,
    origin: String,
    scope: &str,
) -> Result<String, String> {
    if let Some(existing) = seen.get(&normalized) {
        return Err(format!(
            "{scope}: GraphQL name '{normalized}' conflicts between {existing} and {origin}"
        ));
    }
    seen.insert(normalized.clone(), origin);
    Ok(normalized)
}
