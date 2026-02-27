use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Schema {
    pub tables: BTreeMap<String, TableSchema>,
}

#[derive(Clone, Debug)]
pub struct TableSchema {
    pub columns: BTreeMap<String, ColumnSchema>,
}

#[derive(Clone, Debug)]
pub struct ColumnSchema {
    pub column_type: ColumnType,
    pub nullable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnType {
    Integer,
    Float,
    Boolean,
    String,
    Json,
}

pub fn load_schema(folder: &Path, schema_path: Option<&Path>) -> Result<Option<Schema>, String> {
    let resolved_path = resolve_schema_path(folder, schema_path)?;
    let Some(path) = resolved_path else {
        return Ok(None);
    };

    let raw = fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse_dbml_schema(&raw).map(Some)
}

fn resolve_schema_path(
    folder: &Path,
    schema_path: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = schema_path {
        return Ok(Some(path.to_path_buf()));
    }

    let default = folder.join("schema.dbml");
    if default.exists() {
        Ok(Some(default))
    } else {
        Ok(None)
    }
}

pub fn parse_dbml_schema(input: &str) -> Result<Schema, String> {
    let mut tables = BTreeMap::new();
    let mut current_table: Option<(String, TableSchema)> = None;

    for (index, raw_line) in input.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }

        if let Some((table_name, table)) = current_table.as_mut() {
            if line == "}" {
                let (name, table) = current_table.take().expect("present table");
                tables.insert(name, table);
                continue;
            }

            if line.starts_with("indexes") || line.starts_with("note") {
                continue;
            }

            let (column_name, column_schema) = parse_column_line(line)
                .map_err(|err| format!("line {line_number} (table {table_name}): {err}"))?;
            table.columns.insert(column_name, column_schema);
            continue;
        }

        if line.starts_with("Table ") {
            let remainder = line.trim_start_matches("Table ").trim();
            let name = remainder
                .trim_end_matches('{')
                .trim()
                .trim_matches('"')
                .to_string();
            if name.is_empty() {
                return Err(format!("line {line_number}: table name is missing"));
            }

            current_table = Some((
                name,
                TableSchema {
                    columns: BTreeMap::new(),
                },
            ));
        }
    }

    if let Some((name, _)) = current_table {
        return Err(format!("Table '{name}' is missing closing '}}'"));
    }

    Ok(Schema { tables })
}

fn parse_column_line(line: &str) -> Result<(String, ColumnSchema), String> {
    let mut parts = line
        .splitn(3, char::is_whitespace)
        .filter(|s| !s.is_empty());
    let name = parts
        .next()
        .ok_or_else(|| "column name is missing".to_string())?;
    let raw_type = parts
        .next()
        .ok_or_else(|| format!("column type is missing for '{name}'"))?;

    let attrs = parts.next().unwrap_or_default().to_ascii_lowercase();
    let nullable = !attrs.contains("not null") && !attrs.contains("pk");

    let normalized_type = raw_type
        .split('(')
        .next()
        .unwrap_or(raw_type)
        .to_ascii_lowercase();
    let column_type = match normalized_type.as_str() {
        "int" | "integer" | "smallint" | "bigint" | "serial" | "bigserial" => ColumnType::Integer,
        "float" | "double" | "decimal" | "real" | "numeric" => ColumnType::Float,
        "bool" | "boolean" => ColumnType::Boolean,
        "json" | "jsonb" => ColumnType::Json,
        _ => ColumnType::String,
    };

    Ok((
        name.to_string(),
        ColumnSchema {
            column_type,
            nullable,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dbml_schema() {
        let schema = parse_dbml_schema(
            r#"
            Table users {
              id int [pk]
              name varchar [not null]
              active bool
            }
            "#,
        )
        .expect("parse schema");

        let users = schema.tables.get("users").expect("users table");
        assert_eq!(users.columns["id"].column_type, ColumnType::Integer);
        assert_eq!(users.columns["name"].column_type, ColumnType::String);
        assert_eq!(users.columns["active"].column_type, ColumnType::Boolean);
        assert!(!users.columns["id"].nullable);
    }
}
