use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{DeclaredSchema, Schema, TableKind};

use super::{
    relations::{
        build_table_aliases, derive_many_to_many_schema, detect_foreign_keys, infer_table_kind,
    },
    validation::{validate_declared_schema, validate_effective_schema},
};

pub fn merge_schemas(
    declared: Option<&DeclaredSchema>,
    inferred: &Schema,
) -> Result<Schema, String> {
    let mut tables = inferred.tables.clone();

    if let Some(declared) = declared {
        for (table_name, declared_table) in &declared.tables {
            let entry = tables.entry(table_name.clone()).or_default();

            for (column_name, column) in &declared_table.columns {
                entry.columns.insert(column_name.clone(), column.clone());
            }

            if let Some(primary_key) = &declared_table.primary_key {
                entry.primary_key = Some(primary_key.clone());
            }

            if let Some(kind) = &declared_table.kind {
                entry.kind = kind.clone();
            }
        }
    }

    let manual_foreign_keys = declared
        .map(|declared| {
            declared
                .tables
                .iter()
                .map(|(table_name, table)| {
                    (
                        table_name.clone(),
                        table
                            .foreign_keys
                            .keys()
                            .cloned()
                            .chain(table.suppressed_foreign_keys.iter().cloned())
                            .collect::<BTreeSet<_>>(),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();

    let aliases = build_table_aliases(&tables);
    let table_names = tables.keys().cloned().collect::<Vec<_>>();
    for table_name in table_names {
        let skip_columns = manual_foreign_keys.get(&table_name).cloned().unwrap_or_default();
        let mut foreign_keys = detect_foreign_keys(&table_name, &tables, &aliases, &skip_columns);

        if let Some(declared_table) = declared.and_then(|schema| schema.tables.get(&table_name)) {
            for (column_name, foreign_key) in &declared_table.foreign_keys {
                foreign_keys.insert(column_name.clone(), foreign_key.clone());
            }
        }

        let inferred_kind = declared
            .and_then(|schema| schema.tables.get(&table_name))
            .and_then(|table| table.kind.clone())
            .or_else(|| {
                tables.get(&table_name).map(|table| {
                    let mut next = table.clone();
                    next.foreign_keys = foreign_keys.clone();
                    infer_table_kind(&table_name, &next, &tables)
                })
            })
            .unwrap_or(TableKind::Unknown);

        if let Some(table) = tables.get_mut(&table_name) {
            table.foreign_keys = foreign_keys;
            table.many_to_many.clear();
            table.kind = inferred_kind;
        }
    }

    let schema = Schema { tables };
    validate_effective_schema(&schema)?;
    validate_declared_schema(declared, &schema)?;
    Ok(derive_many_to_many_schema(schema))
}
