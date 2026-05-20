use std::collections::BTreeMap;

use crate::schema::{
    ColumnSchema, ColumnType, DeclaredSchema, DeclaredTableSchema, ForeignKey, TableKind,
    is_valid_identifier,
};

use super::relations::infer_declared_table_kind;

fn normalize_identifier(name: &str) -> String {
    name.trim().trim_matches('"').to_string()
}

fn validate_identifier(name: &str, kind: &str, line_number: usize) -> Result<String, String> {
    let normalized = normalize_identifier(name);
    if !is_valid_identifier(&normalized) {
        return Err(format!("line {line_number}: invalid {kind} '{normalized}'"));
    }

    Ok(normalized)
}
pub fn parse_dbml_schema(input: &str) -> Result<DeclaredSchema, String> {
    let mut tables = BTreeMap::new();
    let mut current_table: Option<(String, DeclaredTableSchema)> = None;

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
            if let Some(reference) = parse_inline_reference(line) {
                table.foreign_keys.insert(column_name.clone(), reference);
            }
            if line.to_ascii_lowercase().contains("pk") {
                table.primary_key = Some(column_name.clone());
                table.kind = Some(TableKind::Object);
            }
            table.columns.insert(column_name, column_schema);
            continue;
        }

        if line.starts_with("Ref:") {
            parse_ref_line(line, line_number, &mut tables, current_table.as_mut())?;
            continue;
        }

        if line.starts_with("Table ") {
            let remainder = line.trim_start_matches("Table ").trim();
            let name = remainder.trim_end_matches('{').trim();
            let name = validate_identifier(name, "table name", line_number)?;

            current_table = Some((name, DeclaredTableSchema::default()));
        }
    }

    if let Some((name, _)) = current_table {
        return Err(format!("Table '{name}' is missing closing '}}'"));
    }

    let inferred_kinds = tables
        .iter()
        .map(|(table_name, table)| {
            (
                table_name.clone(),
                table
                    .kind
                    .clone()
                    .or_else(|| infer_declared_table_kind(table_name, table, &tables)),
            )
        })
        .collect::<Vec<_>>();
    for (table_name, kind) in inferred_kinds {
        if let Some(table) = tables.get_mut(&table_name)
            && table.kind.is_none()
        {
            table.kind = kind;
        }
    }

    Ok(DeclaredSchema { tables })
}
fn parse_column_line(line: &str) -> Result<(String, ColumnSchema), String> {
    let mut parts = line.splitn(3, char::is_whitespace).filter(|s| !s.is_empty());
    let name = parts.next().ok_or_else(|| "column name is missing".to_string())?;
    if !is_valid_identifier(name.trim_matches('"')) {
        return Err(format!("invalid column name '{name}'"));
    }
    let raw_type = parts.next().ok_or_else(|| format!("column type is missing for '{name}'"))?;

    let attrs = parts.next().unwrap_or_default().to_ascii_lowercase();
    let nullable = !attrs.contains("not null") && !attrs.contains("pk");

    let column_type = ColumnType::from_dbml_type(raw_type);

    Ok((normalize_identifier(name), ColumnSchema::new(column_type, nullable)))
}

fn parse_inline_reference(line: &str) -> Option<ForeignKey> {
    let ref_pos = line.find("ref:")?;
    let after_ref = &line[ref_pos + 4..];
    let arrow_pos = after_ref.find('>')?;
    let target = after_ref[arrow_pos + 1..].trim().trim_matches(']').trim();
    let (target_table, target_column) = target.split_once('.')?;

    let target_table = normalize_identifier(target_table);
    let target_column = normalize_identifier(target_column);
    if !is_valid_identifier(&target_table) || !is_valid_identifier(&target_column) {
        return None;
    }

    Some(ForeignKey { target_table, target_column })
}

fn parse_ref_line(
    line: &str,
    line_number: usize,
    tables: &mut BTreeMap<String, DeclaredTableSchema>,
    current_table: Option<&mut (String, DeclaredTableSchema)>,
) -> Result<(), String> {
    let expression = line.trim_start_matches("Ref:").trim();
    let (left, right) = expression
        .split_once('>')
        .ok_or_else(|| format!("line {line_number}: invalid Ref expression '{line}'"))?;

    let (source_table, source_column) = parse_ref_side(left.trim(), line_number)?;
    let (target_table, target_column) = parse_ref_side(right.trim(), line_number)?;

    match current_table {
        Some((name, table)) if *name == source_table => {
            table.foreign_keys.insert(source_column, ForeignKey { target_table, target_column });
            return Ok(());
        }
        _ => {}
    }

    let Some(table) = tables.get_mut(&source_table) else {
        return Err(format!("line {line_number}: Ref source table '{source_table}' not found"));
    };

    table.foreign_keys.insert(source_column, ForeignKey { target_table, target_column });
    Ok(())
}

fn parse_ref_side(side: &str, line_number: usize) -> Result<(String, String), String> {
    let cleaned = side.trim().trim_matches('"');
    let (table, column) = cleaned
        .split_once('.')
        .ok_or_else(|| format!("line {line_number}: invalid Ref side '{side}'"))?;
    let table = validate_identifier(table, "table name", line_number)?;
    let column = validate_identifier(column, "column name", line_number)?;
    Ok((table, column))
}
