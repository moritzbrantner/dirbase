use std::collections::BTreeSet;

use serde_json::Value;

use crate::schema::{ColumnSchema, ColumnType, DeclaredSchema, Schema};

pub(crate) fn validate_effective_schema(schema: &Schema) -> Result<(), String> {
    for (table_name, table) in &schema.tables {
        if let Some(primary_key) = &table.primary_key
            && !table.columns.contains_key(primary_key)
        {
            return Err(format!(
                "table '{table_name}' declares primary key '{primary_key}' but no such column exists"
            ));
        }

        for (source_column, foreign_key) in &table.foreign_keys {
            let Some(source_column_schema) = table.columns.get(source_column) else {
                return Err(format!(
                    "table '{table_name}' declares foreign key '{source_column}' but no such column exists"
                ));
            };
            let Some(target_table) = schema.tables.get(&foreign_key.target_table) else {
                return Err(format!(
                    "table '{table_name}' foreign key '{source_column}' targets unknown table '{}'",
                    foreign_key.target_table
                ));
            };
            let Some(target_column_schema) = target_table.columns.get(&foreign_key.target_column)
            else {
                return Err(format!(
                    "table '{table_name}' foreign key '{source_column}' targets unknown column '{}.{}'",
                    foreign_key.target_table, foreign_key.target_column
                ));
            };
            if !column_types_are_compatible(
                &source_column_schema.column_type,
                &target_column_schema.column_type,
            ) {
                return Err(format!(
                    "table '{table_name}' foreign key '{source_column}' is incompatible with '{}.{}'",
                    foreign_key.target_table, foreign_key.target_column
                ));
            }
        }

        for (column_name, column) in &table.columns {
            validate_column_constraints(table_name, column_name, column)?;
        }
    }

    Ok(())
}

fn column_types_are_compatible(left: &ColumnType, right: &ColumnType) -> bool {
    left.is_compatible_with(right)
}

pub(crate) fn validate_declared_schema(
    declared: Option<&DeclaredSchema>,
    effective: &Schema,
) -> Result<(), String> {
    let Some(declared) = declared else {
        return Ok(());
    };

    for (table_name, declared_table) in &declared.tables {
        let Some(effective_table) = effective.tables.get(table_name) else {
            continue;
        };
        let mut seen_constraints = BTreeSet::new();
        for unique in &declared_table.unique {
            if unique.is_empty() {
                return Err(format!("table '{table_name}' declares an empty unique constraint"));
            }
            let mut seen_columns = BTreeSet::new();
            for column_name in unique {
                if !seen_columns.insert(column_name.clone()) {
                    return Err(format!(
                        "table '{table_name}' unique constraint contains duplicate column '{column_name}'"
                    ));
                }
                if !effective_table.columns.contains_key(column_name) {
                    return Err(format!(
                        "table '{table_name}' unique constraint references unknown column '{column_name}'"
                    ));
                }
            }
            let mut normalized = unique.clone();
            normalized.sort();
            let key = normalized.join("\x1f");
            if !seen_constraints.insert(key) {
                return Err(format!("table '{table_name}' declares duplicate unique constraints"));
            }
        }
    }

    Ok(())
}

fn validate_column_constraints(
    table_name: &str,
    column_name: &str,
    column: &ColumnSchema,
) -> Result<(), String> {
    if let Some(values) = &column.enum_values {
        if !matches!(column.column_type, ColumnType::String) {
            return Err(format!(
                "table '{table_name}' column '{column_name}' declares enum_values on non-string type"
            ));
        }
        if values.is_empty() {
            return Err(format!(
                "table '{table_name}' column '{column_name}' declares empty enum_values"
            ));
        }
        let mut seen = BTreeSet::new();
        for value in values {
            if !seen.insert(value) {
                return Err(format!(
                    "table '{table_name}' column '{column_name}' declares duplicate enum value '{value}'"
                ));
            }
        }
    }

    if (column.min.is_some() || column.max.is_some())
        && !column.column_type.is_orderable_with_bounds()
    {
        return Err(format!(
            "table '{table_name}' column '{column_name}' declares min/max on unsupported type '{}'",
            column.column_type.label()
        ));
    }
    if let Some(min) = &column.min {
        validate_bound_value(table_name, column_name, &column.column_type, "min", min)?;
    }
    if let Some(max) = &column.max {
        validate_bound_value(table_name, column_name, &column.column_type, "max", max)?;
    }
    if let (Some(min), Some(max)) = (&column.min, &column.max)
        && compare_bound_values(&column.column_type, min, max)
            .is_some_and(|ordering| ordering.is_gt())
    {
        return Err(format!(
            "table '{table_name}' column '{column_name}' declares min greater than max"
        ));
    }

    if (column.min_length.is_some() || column.max_length.is_some())
        && !column.column_type.is_string_backed()
    {
        return Err(format!(
            "table '{table_name}' column '{column_name}' declares length constraints on unsupported type '{}'",
            column.column_type.label()
        ));
    }
    if let (Some(min), Some(max)) = (column.min_length, column.max_length)
        && min > max
    {
        return Err(format!(
            "table '{table_name}' column '{column_name}' declares min_length greater than max_length"
        ));
    }

    if let Some(pattern) = &column.pattern {
        if !column.column_type.is_string_backed() {
            return Err(format!(
                "table '{table_name}' column '{column_name}' declares pattern on unsupported type '{}'",
                column.column_type.label()
            ));
        }
        regex::Regex::new(pattern).map_err(|err| {
            format!("table '{table_name}' column '{column_name}' declares invalid pattern: {err}")
        })?;
    }

    Ok(())
}

fn validate_bound_value(
    table_name: &str,
    column_name: &str,
    column_type: &ColumnType,
    bound_name: &str,
    value: &Value,
) -> Result<(), String> {
    match column_type {
        ColumnType::Integer | ColumnType::Float | ColumnType::BigInteger | ColumnType::Decimal => {
            if value.as_f64().is_some() {
                Ok(())
            } else {
                Err(format!(
                    "table '{table_name}' column '{column_name}' declares non-numeric {bound_name}"
                ))
            }
        }
        ColumnType::Date => {
            let Some(text) = value.as_str() else {
                return Err(format!(
                    "table '{table_name}' column '{column_name}' declares non-string {bound_name}"
                ));
            };
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|_| ()).map_err(|err| {
                format!(
                    "table '{table_name}' column '{column_name}' declares invalid {bound_name}: {err}"
                )
            })
        }
        ColumnType::DateTime => {
            let Some(text) = value.as_str() else {
                return Err(format!(
                    "table '{table_name}' column '{column_name}' declares non-string {bound_name}"
                ));
            };
            chrono::DateTime::parse_from_rfc3339(text).map(|_| ()).map_err(|err| {
                format!(
                    "table '{table_name}' column '{column_name}' declares invalid {bound_name}: {err}"
                )
            })
        }
        _ => Ok(()),
    }
}

fn compare_bound_values(
    column_type: &ColumnType,
    left: &Value,
    right: &Value,
) -> Option<std::cmp::Ordering> {
    match column_type {
        ColumnType::Integer | ColumnType::Float | ColumnType::BigInteger | ColumnType::Decimal => {
            left.as_f64()?.partial_cmp(&right.as_f64()?)
        }
        ColumnType::Date => {
            let left = chrono::NaiveDate::parse_from_str(left.as_str()?, "%Y-%m-%d").ok()?;
            let right = chrono::NaiveDate::parse_from_str(right.as_str()?, "%Y-%m-%d").ok()?;
            Some(left.cmp(&right))
        }
        ColumnType::DateTime => {
            let left = chrono::DateTime::parse_from_rfc3339(left.as_str()?).ok()?;
            let right = chrono::DateTime::parse_from_rfc3339(right.as_str()?).ok()?;
            Some(left.cmp(&right))
        }
        _ => None,
    }
}
