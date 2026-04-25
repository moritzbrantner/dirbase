use std::collections::HashMap;

use serde_json::Value;

use crate::schema::{ColumnType, TableSchema, primary_key_name};

pub fn find_item_by_key<'a>(items: &'a [Value], key_name: &str, id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, key_name, id))
}

pub fn find_item_index_by_key(items: &[Value], key_name: &str, id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, key_name, id))
}

pub fn build_id_index(
    value: &Value,
    table: Option<&TableSchema>,
) -> Option<HashMap<String, usize>> {
    let items = value.as_array()?;
    let key_name = primary_key_name(table);
    let mut index = HashMap::with_capacity(items.len());
    let mut has_any_id = false;

    for (position, item) in items.iter().enumerate() {
        let Some(id_value) = item.as_object().and_then(|obj| obj.get(key_name)) else {
            continue;
        };
        match id_value {
            Value::Number(number) => {
                index.insert(number.to_string(), position);
                has_any_id = true;
            }
            Value::String(text) => {
                index.insert(text.clone(), position);
                has_any_id = true;
            }
            Value::Bool(value) => {
                index.insert(value.to_string(), position);
                has_any_id = true;
            }
            _ => {}
        }
    }

    has_any_id.then_some(index)
}

pub fn next_numeric_id(items: &[Value], key_name: &str) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get(key_name)))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

pub fn coerce_id_value(id: &str, table: Option<&TableSchema>) -> Value {
    let key_name = primary_key_name(table);
    match table.and_then(|table| table.columns.get(key_name)) {
        Some(column) if matches!(column.column_type, ColumnType::String) => {
            Value::String(id.to_string())
        }
        _ => id.parse::<i64>().map_or_else(|_| Value::String(id.to_string()), Value::from),
    }
}

fn id_matches(item: &Value, key_name: &str, expected: &str) -> bool {
    item.as_object().and_then(|obj| obj.get(key_name)).is_some_and(|id| match id {
        Value::Number(n) => n.to_string() == expected,
        Value::String(s) => s == expected,
        Value::Bool(value) => value.to_string() == expected,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::schema::{ColumnSchema, TableKind};

    fn table_with_pk(pk_name: &str, column_type: Option<ColumnType>) -> TableSchema {
        let mut columns = BTreeMap::new();
        if let Some(column_type) = column_type {
            columns.insert(pk_name.to_string(), ColumnSchema { column_type, nullable: false });
        }
        TableSchema {
            kind: TableKind::Object,
            primary_key: Some(pk_name.to_string()),
            columns,
            foreign_keys: BTreeMap::new(),
            many_to_many: BTreeMap::new(),
        }
    }

    #[test]
    fn find_item_by_key_matches_numeric_id() {
        let items = json!([{"id": 1}, {"id": 2}]);
        let found = find_item_by_key(items.as_array().expect("array"), "id", "2").expect("item");
        assert_eq!(found["id"], 2);
    }

    #[test]
    fn find_item_by_key_matches_string_id() {
        let items = json!([{"id": "user-1"}, {"id": "user-2"}]);
        let found =
            find_item_by_key(items.as_array().expect("array"), "id", "user-2").expect("item");
        assert_eq!(found["id"], "user-2");
    }

    #[test]
    fn find_item_by_key_matches_bool_id() {
        let items = json!([{"id": false}, {"id": true}]);
        let found = find_item_by_key(items.as_array().expect("array"), "id", "true").expect("item");
        assert_eq!(found["id"], true);
    }

    #[test]
    fn find_item_by_key_returns_none_for_missing_id() {
        let items = json!([{"id": 1}]);
        assert_eq!(find_item_by_key(items.as_array().expect("array"), "id", "99"), None);
    }

    #[test]
    fn find_item_index_by_key_returns_expected_position() {
        let items = json!([{"id": 1}, {"id": 2}, {"id": 3}]);
        assert_eq!(find_item_index_by_key(items.as_array().expect("array"), "id", "2"), Some(1));
    }

    #[test]
    fn find_item_index_by_key_returns_none_for_missing_id() {
        let items = json!([{"id": 1}]);
        assert_eq!(find_item_index_by_key(items.as_array().expect("array"), "id", "2"), None);
    }

    #[test]
    fn build_id_index_uses_default_id_key() {
        let items = json!([{"id": 1}, {"id": "two"}, {"id": true}]);
        let index = build_id_index(&items, None).expect("index");
        assert_eq!(index.get("1"), Some(&0));
        assert_eq!(index.get("two"), Some(&1));
        assert_eq!(index.get("true"), Some(&2));
    }

    #[test]
    fn build_id_index_uses_declared_primary_key() {
        let items = json!([{"slug": "ada"}, {"slug": "grace"}]);
        let table = table_with_pk("slug", Some(ColumnType::String));
        let index = build_id_index(&items, Some(&table)).expect("index");
        assert_eq!(index.get("ada"), Some(&0));
        assert_eq!(index.get("grace"), Some(&1));
    }

    #[test]
    fn build_id_index_skips_items_without_supported_ids() {
        let items = json!([
            {"id": {"nested": true}},
            {"id": null},
            {"id": 7},
            {"name": "Ada"}
        ]);
        let index = build_id_index(&items, None).expect("index");
        assert_eq!(index.len(), 1);
        assert_eq!(index.get("7"), Some(&2));
    }

    #[test]
    fn build_id_index_returns_none_when_no_supported_ids_exist() {
        let items = json!([{"id": null}, {"id": {"nested": true}}, {"name": "Ada"}]);
        assert_eq!(build_id_index(&items, None), None);
    }

    #[test]
    fn next_numeric_id_returns_one_for_empty_array() {
        assert_eq!(next_numeric_id(&[], "id"), 1);
    }

    #[test]
    fn next_numeric_id_returns_max_plus_one() {
        let items = json!([{"id": 2}, {"id": 9}, {"id": -4}]);
        assert_eq!(next_numeric_id(items.as_array().expect("array"), "id"), 10);
    }

    #[test]
    fn coerce_id_value_keeps_string_for_declared_string_pk() {
        let table = table_with_pk("slug", Some(ColumnType::String));
        assert_eq!(coerce_id_value("42", Some(&table)), Value::String("42".to_string()));
    }

    #[test]
    fn coerce_id_value_parses_numeric_id_when_not_string_pk() {
        let table = table_with_pk("id", Some(ColumnType::Integer));
        assert_eq!(coerce_id_value("42", Some(&table)), Value::from(42));
    }

    #[test]
    fn coerce_id_value_falls_back_to_string_for_non_numeric_input() {
        let table = table_with_pk("id", Some(ColumnType::Integer));
        assert_eq!(coerce_id_value("user-42", Some(&table)), Value::String("user-42".to_string()));
    }
}
