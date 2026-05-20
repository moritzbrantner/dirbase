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
    let mut tables = BTreeMap::new();

    for (table_name, value) in values {
        let Some(rows) = value.as_array() else {
            continue;
        };
        if !rows.iter().all(Value::is_object) {
            continue;
        }

        let mut table = infer_table_schema(rows);
        table.primary_key = infer_primary_key(table_name, rows);
        table.kind =
            if table.primary_key.is_some() { TableKind::Object } else { TableKind::Unknown };
        tables.insert(table_name.clone(), table);
    }

    let aliases = build_table_aliases(&tables);
    let table_names = tables.keys().cloned().collect::<Vec<_>>();
    for table_name in table_names {
        let foreign_keys = detect_foreign_keys(&table_name, &tables, &aliases, &BTreeSet::new());
        let inferred_kind = tables
            .get(&table_name)
            .map(|table| {
                let mut next = table.clone();
                next.foreign_keys = foreign_keys.clone();
                infer_table_kind(&table_name, &next, &tables)
            })
            .unwrap_or(TableKind::Unknown);
        if let Some(table) = tables.get_mut(&table_name) {
            table.foreign_keys = foreign_keys;
            table.kind = inferred_kind;
        }
    }

    derive_many_to_many_schema(Schema { tables })
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
