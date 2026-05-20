use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::app::DataSource;

mod infer;
mod merge;
mod parse_dbml;
mod parse_json;
mod parse_xsd;
mod relations;
#[cfg(test)]
mod tests;
mod types;
mod validation;

pub use infer::infer_schema_from_data_source;
#[cfg(test)]
pub use infer::infer_schema_from_values;
pub use merge::merge_schemas;
pub use parse_dbml::parse_dbml_schema;
pub use parse_json::parse_json_schema;
pub use parse_xsd::parse_xsd_schema;
pub use types::{
    ColumnSchema, ColumnType, DeclaredSchema, DeclaredTableSchema, ForeignKey, ManyToManyRelation,
    Schema, TableKind, TableSchema, is_valid_identifier, primary_key_name,
};

pub fn load_schema(
    folder: &Path,
    schema_path: Option<&Path>,
) -> Result<Option<DeclaredSchema>, String> {
    let resolved_path = resolve_schema_path(folder, schema_path)?;
    let Some(path) = resolved_path else {
        return Ok(None);
    };

    let raw = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse_schema_file(&path, &raw).map(Some)
}

pub fn export_declared_schema_snapshot(
    declared: Option<&DeclaredSchema>,
    effective: &Schema,
) -> DeclaredSchema {
    let tables = effective
        .tables
        .iter()
        .map(|(table_name, table)| {
            let suppressed_foreign_keys = declared
                .and_then(|schema| schema.tables.get(table_name))
                .map(|table| table.suppressed_foreign_keys.clone())
                .unwrap_or_default();
            let unique = declared
                .and_then(|schema| schema.tables.get(table_name))
                .map(|table| table.unique.clone())
                .unwrap_or_default();

            (
                table_name.clone(),
                DeclaredTableSchema {
                    kind: Some(table.kind.clone()),
                    primary_key: table.primary_key.clone(),
                    columns: table.columns.clone(),
                    foreign_keys: table.foreign_keys.clone(),
                    suppressed_foreign_keys,
                    unique,
                },
            )
        })
        .collect();

    DeclaredSchema { tables }
}

pub fn default_schema_output_path(data_source: &DataSource) -> PathBuf {
    match data_source {
        DataSource::Folder(folder) => folder.join("schema.json"),
        DataSource::File(file) => file
            .parent()
            .map(|parent| parent.join("schema.json"))
            .unwrap_or_else(|| PathBuf::from("schema.json")),
    }
}

fn resolve_schema_path(
    folder: &Path,
    schema_path: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = schema_path {
        return Ok(Some(path.to_path_buf()));
    }

    let json = folder.join("schema.json");
    if json.exists() {
        return Ok(Some(json));
    }

    let xsd = folder.join("schema.xsd");
    if xsd.exists() {
        return Ok(Some(xsd));
    }

    let dbml = folder.join("schema.dbml");
    if dbml.exists() {
        return Ok(Some(dbml));
    }

    Ok(None)
}

fn parse_schema_file(path: &Path, raw: &str) -> Result<DeclaredSchema, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => parse_json_schema(raw),
        Some("xsd") => parse_xsd_schema(raw),
        _ => parse_dbml_schema(raw),
    }
}
