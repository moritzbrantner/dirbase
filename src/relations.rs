use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::query::filters::value_to_filter_string;

pub fn build_relation_lookup<'a>(
    target_items: &'a [Value],
    target_column: &str,
) -> HashMap<String, &'a Value> {
    let mut lookup = HashMap::new();
    for item in target_items {
        if let Some((_, key)) =
            item.as_object().and_then(|object| object.get(target_column).map(|key| (object, key)))
        {
            lookup.insert(value_to_filter_string(key), item);
        }
    }
    lookup
}

pub fn resolve_related_row_in_lookup(
    source_object: &Map<String, Value>,
    source_column: &str,
    lookup: &HashMap<String, &Value>,
) -> Option<Value> {
    let current_value = source_object.get(source_column)?;
    if current_value.is_object() || current_value.is_null() {
        return None;
    }

    let key = value_to_filter_string(current_value);
    lookup.get(&key).map(|row| (*row).clone())
}
