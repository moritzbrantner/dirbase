use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use serde_json::Value;

use crate::{
    app::DataSource,
    schema::{ColumnSchema, ColumnType, Schema, TableKind, TableSchema},
};

use super::relations::{
    build_table_aliases, derive_many_to_many_schema, detect_foreign_keys, infer_table_kind,
    singularize_table_name,
};

pub fn infer_schema_from_data_source(
    data_source: &DataSource,
    resources: &BTreeSet<String>,
) -> Result<Schema, String> {
    match data_source {
        DataSource::Folder(folder) => {
            let mut values = BTreeMap::new();
            for resource in resources {
                let path = folder.join(format!("{resource}.json"));
                let raw = fs::read_to_string(&path)
                    .map_err(|err| format!("{}: {err}", path.display()))?;
                let value = serde_json::from_str(&raw)
                    .map_err(|err| format!("{}: invalid json: {err}", path.display()))?;
                values.insert(resource.clone(), value);
            }
            Ok(infer_schema_from_values(&values))
        }
        DataSource::File(file) => {
            let raw =
                fs::read_to_string(file).map_err(|err| format!("{}: {err}", file.display()))?;
            let root: Value = serde_json::from_str(&raw)
                .map_err(|err| format!("{}: invalid json: {err}", file.display()))?;
            let mut values = BTreeMap::new();
            if let Some(object) = root.as_object() {
                for resource in resources {
                    if let Some(value) = object.get(resource) {
                        values.insert(resource.clone(), value.clone());
                    }
                }
            }
            Ok(infer_schema_from_values(&values))
        }
    }
}

pub fn infer_schema_from_values(values: &BTreeMap<String, Value>) -> Schema {
    let tables = values
        .iter()
        .filter_map(|(table_name, value)| {
            infer_table_from_value(table_name, value).map(|table| (table_name.clone(), table))
        })
        .collect();

    rebuild_inferred_relations(Schema { tables })
}

pub(crate) fn replace_inferred_table(
    mut schema: Schema,
    table_name: &str,
    value: &Value,
) -> Schema {
    if let Some(table) = infer_table_from_value(table_name, value) {
        schema.tables.insert(table_name.to_string(), table);
    } else {
        schema.tables.remove(table_name);
    }
    rebuild_inferred_relations(schema)
}

fn infer_table_from_value(table_name: &str, value: &Value) -> Option<TableSchema> {
    let rows = value.as_array()?;
    if !rows.iter().all(Value::is_object) {
        return None;
    }

    let mut table = infer_table_schema(rows);
    table.primary_key = infer_primary_key(table_name, rows);
    table.kind = if table.primary_key.is_some() { TableKind::Object } else { TableKind::Unknown };
    Some(table)
}

fn rebuild_inferred_relations(mut schema: Schema) -> Schema {
    for table in schema.tables.values_mut() {
        table.foreign_keys.clear();
        table.many_to_many.clear();
        table.kind = if table.primary_key.is_some() { TableKind::Object } else { TableKind::Unknown };
    }

    let aliases = build_table_aliases(&schema.tables);
    let table_names = schema.tables.keys().cloned().collect::<Vec<_>>();
    for table_name in table_names {
        let foreign_keys =
            detect_foreign_keys(&table_name, &schema.tables, &aliases, &BTreeSet::new());
        let inferred_kind = schema
            .tables
            .get(&table_name)
            .map(|table| {
                let mut next = table.clone();
                next.foreign_keys = foreign_keys.clone();
                infer_table_kind(&table_name, &next, &schema.tables)
            })
            .unwrap_or(TableKind::Unknown);
        if let Some(table) = schema.tables.get_mut(&table_name) {
            table.foreign_keys = foreign_keys;
            table.kind = inferred_kind;
        }
    }

    derive_many_to_many_schema(schema)
}

fn infer_table_schema(rows: &[Value]) -> TableSchema {
    let mut columns = BTreeMap::<String, ColumnSchema>::new();

    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };

        for (column_name, value) in object {
            let inferred_type = ColumnType::infer_json(value);
            let entry = columns.entry(column_name.clone()).or_insert(ColumnSchema {
                column_type: inferred_type.clone().unwrap_or(ColumnType::String),
                nullable: false,
                enum_values: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
            });

            if let Some(inferred_type) = inferred_type
                && entry.column_type != inferred_type
            {
                let has_json = matches!(entry.column_type, ColumnType::Json)
                    || matches!(inferred_type, ColumnType::Json);
                entry.column_type = if has_json { ColumnType::Json } else { ColumnType::String };
            }

            if value.is_null() {
                entry.nullable = true;
            }
        }
    }

    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        for (column_name, column) in &mut columns {
            if !object.contains_key(column_name) {
                column.nullable = true;
            }
        }
    }

    TableSchema {
        kind: TableKind::Unknown,
        primary_key: None,
        columns,
        foreign_keys: BTreeMap::new(),
        many_to_many: BTreeMap::new(),
    }
}

fn infer_primary_key(table_name: &str, rows: &[Value]) -> Option<String> {
    let singular = singularize_table_name(table_name);
    let candidates = ["id".to_string(), format!("{singular}_id"), format!("{table_name}_id")];

    candidates.into_iter().find(|candidate| is_unique_scalar_column(rows, candidate))
}

fn is_unique_scalar_column(rows: &[Value], column_name: &str) -> bool {
    let mut values = BTreeSet::new();
    let mut has_rows = false;

    for row in rows {
        let Some(object) = row.as_object() else {
            return false;
        };
        let Some(value) = object.get(column_name) else {
            return false;
        };
        let Some(key) = scalar_key(value) else {
            return false;
        };
        has_rows = true;
        if !values.insert(key) {
            return false;
        }
    }

    has_rows
}

fn scalar_key(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(format!("s:{value}")),
        Value::Number(value) => Some(format!("n:{value}")),
        Value::Bool(value) => Some(format!("b:{value}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replacing_one_table_recomputes_foreign_keys_on_other_tables() {
        let initial = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"id": 1}, {"id": 2}])),
            (
                "posts".to_string(),
                json!([
                    {"id": 10, "user_id": 1},
                    {"id": 11, "user_id": 2}
                ]),
            ),
        ]));
        assert_eq!(
            initial.tables["posts"].foreign_keys["user_id"].target_table,
            "users"
        );

        let updated = replace_inferred_table(
            initial,
            "users",
            &json!([{"id": "ada"}, {"id": "grace"}]),
        );

        assert!(
            !updated.tables["posts"].foreign_keys.contains_key("user_id"),
            "changing the target key type must invalidate another table's inferred foreign key"
        );
    }

    #[test]
    fn replacing_one_table_rebuilds_many_to_many_metadata() {
        let initial = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"id": 1}, {"id": 2}])),
            ("teams".to_string(), json!([{"id": 10}, {"id": 11}])),
            (
                "memberships".to_string(),
                json!([
                    {"user_id": 1, "team_id": 10},
                    {"user_id": 2, "team_id": 11}
                ]),
            ),
        ]));
        assert!(initial.tables["users"].many_to_many.contains_key("teams"));

        let updated = replace_inferred_table(
            initial,
            "memberships",
            &json!([
                {"user_id": 1, "label": "owner"},
                {"user_id": 2, "label": "member"}
            ]),
        );

        assert!(
            !updated.tables["users"].many_to_many.contains_key("teams"),
            "changing the junction shape must clear stale many-to-many metadata"
        );
    }
}
