use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::schema::{
    DeclaredTableSchema, ForeignKey, ManyToManyRelation, Schema, TableKind, TableSchema,
};

pub(crate) fn build_table_aliases(
    tables: &BTreeMap<String, TableSchema>,
) -> HashMap<String, String> {
    let mut aliases = HashMap::new();

    for (table_name, table) in tables {
        if table.primary_key.is_none() {
            continue;
        }

        aliases.entry(table_name.clone()).or_insert_with(|| table_name.clone());
        aliases.entry(singularize_table_name(table_name)).or_insert_with(|| table_name.clone());
    }

    aliases
}

pub(crate) fn singularize_table_name(table_name: &str) -> String {
    if let Some(stem) = table_name.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if let Some(stem) = table_name.strip_suffix('s')
        && !stem.is_empty()
    {
        return stem.to_string();
    }
    table_name.to_string()
}

pub(crate) fn detect_foreign_keys(
    source_table_name: &str,
    tables: &BTreeMap<String, TableSchema>,
    aliases: &HashMap<String, String>,
    skip_columns: &BTreeSet<String>,
) -> BTreeMap<String, ForeignKey> {
    let mut foreign_keys = BTreeMap::new();
    let Some(table) = tables.get(source_table_name) else {
        return foreign_keys;
    };

    for (column_name, column) in &table.columns {
        if skip_columns.contains(column_name) {
            continue;
        }

        let Some(prefix) = column_name.strip_suffix("_id") else {
            continue;
        };
        let Some(target_table_name) = aliases.get(prefix) else {
            continue;
        };
        if target_table_name == source_table_name {
            continue;
        }
        let Some(target_table) = tables.get(target_table_name) else {
            continue;
        };
        let Some(target_column_name) = target_table.primary_key.as_deref() else {
            continue;
        };
        let Some(target_column) = target_table.columns.get(target_column_name) else {
            continue;
        };
        if !column.column_type.is_compatible_with(&target_column.column_type) {
            continue;
        }

        foreign_keys.insert(
            column_name.clone(),
            ForeignKey {
                target_table: target_table_name.clone(),
                target_column: target_column_name.to_string(),
            },
        );
    }

    foreign_keys
}

pub(crate) fn infer_table_kind(
    table_name: &str,
    table: &TableSchema,
    tables: &BTreeMap<String, TableSchema>,
) -> TableKind {
    if table.primary_key.is_some() {
        return TableKind::Object;
    }
    if is_strict_junction_table(table_name, table, tables) {
        return TableKind::Relation;
    }
    TableKind::Unknown
}

pub(crate) fn infer_declared_table_kind(
    table_name: &str,
    table: &DeclaredTableSchema,
    tables: &BTreeMap<String, DeclaredTableSchema>,
) -> Option<TableKind> {
    if table.primary_key.is_some() {
        return Some(TableKind::Object);
    }
    if is_strict_declared_junction_table(table_name, table, tables) {
        return Some(TableKind::Relation);
    }
    None
}

fn is_strict_junction_table(
    source_table_name: &str,
    table: &TableSchema,
    tables: &BTreeMap<String, TableSchema>,
) -> bool {
    if table.primary_key.is_some() || table.columns.len() != 2 || table.foreign_keys.len() != 2 {
        return false;
    }
    if !table.columns.keys().all(|column_name| table.foreign_keys.contains_key(column_name)) {
        return false;
    }

    let foreign_keys = table.foreign_keys.values().collect::<Vec<_>>();
    if foreign_keys[0].target_table == foreign_keys[1].target_table {
        return false;
    }

    foreign_keys.iter().all(|fk| {
        fk.target_table != source_table_name
            && tables
                .get(&fk.target_table)
                .and_then(|target| target.primary_key.as_deref())
                .is_some_and(|primary_key| fk.target_column == primary_key)
    })
}

fn is_strict_declared_junction_table(
    source_table_name: &str,
    table: &DeclaredTableSchema,
    tables: &BTreeMap<String, DeclaredTableSchema>,
) -> bool {
    if table.primary_key.is_some() || table.columns.len() != 2 || table.foreign_keys.len() != 2 {
        return false;
    }
    if !table.columns.keys().all(|column_name| table.foreign_keys.contains_key(column_name)) {
        return false;
    }

    let foreign_keys = table.foreign_keys.values().collect::<Vec<_>>();
    if foreign_keys[0].target_table == foreign_keys[1].target_table {
        return false;
    }

    foreign_keys.iter().all(|fk| {
        fk.target_table != source_table_name
            && tables
                .get(&fk.target_table)
                .and_then(|target| target.primary_key.as_deref())
                .is_some_and(|primary_key| fk.target_column == primary_key)
    })
}

pub(crate) fn derive_many_to_many_schema(mut schema: Schema) -> Schema {
    for table in schema.tables.values_mut() {
        table.many_to_many.clear();
    }

    let table_names = schema.tables.keys().cloned().collect::<Vec<_>>();
    for through_table_name in table_names {
        let Some(through_table) = schema.tables.get(&through_table_name).cloned() else {
            continue;
        };
        if !is_strict_junction_table(&through_table_name, &through_table, &schema.tables) {
            continue;
        }

        let mut foreign_keys = through_table
            .foreign_keys
            .iter()
            .map(|(column_name, fk)| (column_name.clone(), fk.clone()))
            .collect::<Vec<_>>();
        foreign_keys.sort_by(|left, right| left.0.cmp(&right.0));

        let [(left_column, left_fk), (right_column, right_fk)] = foreign_keys.as_slice() else {
            continue;
        };

        add_many_to_many_relation(
            &mut schema.tables,
            &left_fk.target_table,
            ManyToManyRelation {
                through_table: through_table_name.clone(),
                source_column: left_column.clone(),
                source_target_column: left_fk.target_column.clone(),
                target_table: right_fk.target_table.clone(),
                target_column: right_fk.target_column.clone(),
                through_target_column: right_column.clone(),
            },
        );
        add_many_to_many_relation(
            &mut schema.tables,
            &right_fk.target_table,
            ManyToManyRelation {
                through_table: through_table_name,
                source_column: right_column.clone(),
                source_target_column: right_fk.target_column.clone(),
                target_table: left_fk.target_table.clone(),
                target_column: left_fk.target_column.clone(),
                through_target_column: left_column.clone(),
            },
        );
    }

    schema
}

fn add_many_to_many_relation(
    tables: &mut BTreeMap<String, TableSchema>,
    source_table_name: &str,
    relation: ManyToManyRelation,
) {
    let Some(source_table) = tables.get_mut(source_table_name) else {
        return;
    };

    let preferred_name = relation.target_table.clone();
    let relation_name = if source_table.many_to_many.contains_key(&preferred_name) {
        format!("{}_via_{}", relation.target_table, relation.through_table)
    } else {
        preferred_name
    };
    source_table.many_to_many.insert(relation_name, relation);
}
