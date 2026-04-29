use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::DataSource;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Schema {
    #[serde(default)]
    pub tables: BTreeMap<String, TableSchema>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TableSchema {
    #[serde(default)]
    pub kind: TableKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<String>,
    #[serde(default)]
    pub columns: BTreeMap<String, ColumnSchema>,
    #[serde(default)]
    pub foreign_keys: BTreeMap<String, ForeignKey>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub many_to_many: BTreeMap<String, ManyToManyRelation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeclaredSchema {
    #[serde(default)]
    pub tables: BTreeMap<String, DeclaredTableSchema>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeclaredTableSchema {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TableKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<String>,
    #[serde(default)]
    pub columns: BTreeMap<String, ColumnSchema>,
    #[serde(default)]
    pub foreign_keys: BTreeMap<String, ForeignKey>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub suppressed_foreign_keys: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unique: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    Object,
    Relation,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForeignKey {
    pub target_table: String,
    pub target_column: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManyToManyRelation {
    pub through_table: String,
    pub source_column: String,
    pub source_target_column: String,
    pub target_table: String,
    pub target_column: String,
    pub through_target_column: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnSchema {
    pub column_type: ColumnType,
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    Integer,
    Float,
    Boolean,
    String,
    Json,
    Date,
    #[serde(rename = "datetime")]
    DateTime,
    Uuid,
    BigInteger,
    Decimal,
}

impl ColumnSchema {
    pub fn new(column_type: ColumnType, nullable: bool) -> Self {
        Self {
            column_type,
            nullable,
            enum_values: None,
            min: None,
            max: None,
            min_length: None,
            max_length: None,
            pattern: None,
        }
    }
}

impl ColumnType {
    pub fn label(&self) -> &'static str {
        match self {
            ColumnType::Integer => "integer",
            ColumnType::Float => "float",
            ColumnType::Boolean => "boolean",
            ColumnType::String => "string",
            ColumnType::Json => "json",
            ColumnType::Date => "date",
            ColumnType::DateTime => "datetime",
            ColumnType::Uuid => "uuid",
            ColumnType::BigInteger => "big_integer",
            ColumnType::Decimal => "decimal",
        }
    }

    pub fn from_dbml_type(raw: &str) -> Self {
        match raw.split('(').next().unwrap_or(raw).to_ascii_lowercase().as_str() {
            "int" | "integer" | "smallint" | "serial" => ColumnType::Integer,
            "bigint" | "bigserial" => ColumnType::BigInteger,
            "float" | "double" | "real" => ColumnType::Float,
            "decimal" | "numeric" => ColumnType::Decimal,
            "bool" | "boolean" => ColumnType::Boolean,
            "json" | "jsonb" => ColumnType::Json,
            "date" => ColumnType::Date,
            "datetime" | "timestamp" | "timestamptz" => ColumnType::DateTime,
            "uuid" => ColumnType::Uuid,
            _ => ColumnType::String,
        }
    }

    pub fn infer_json(value: &Value) -> Option<Self> {
        if value.is_i64() || value.is_u64() {
            return Some(ColumnType::Integer);
        }
        if value.is_number() {
            return Some(ColumnType::Float);
        }
        if value.is_boolean() {
            return Some(ColumnType::Boolean);
        }
        if value.is_array() || value.is_object() {
            return Some(ColumnType::Json);
        }
        if value.is_null() {
            return None;
        }
        Some(ColumnType::String)
    }

    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self == other
            || matches!(
                (self, other),
                (ColumnType::Integer, ColumnType::Float)
                    | (ColumnType::Float, ColumnType::Integer)
                    | (ColumnType::Integer, ColumnType::BigInteger)
                    | (ColumnType::BigInteger, ColumnType::Integer)
                    | (ColumnType::Integer, ColumnType::Decimal)
                    | (ColumnType::Decimal, ColumnType::Integer)
                    | (ColumnType::Float, ColumnType::Decimal)
                    | (ColumnType::Decimal, ColumnType::Float)
                    | (ColumnType::BigInteger, ColumnType::Decimal)
                    | (ColumnType::Decimal, ColumnType::BigInteger)
            )
    }

    pub fn is_string_backed(&self) -> bool {
        matches!(
            self,
            ColumnType::String
                | ColumnType::Date
                | ColumnType::DateTime
                | ColumnType::Uuid
                | ColumnType::BigInteger
                | ColumnType::Decimal
        )
    }

    pub fn is_numeric_like(&self) -> bool {
        matches!(
            self,
            ColumnType::Integer | ColumnType::Float | ColumnType::BigInteger | ColumnType::Decimal
        )
    }

    pub fn is_orderable_with_bounds(&self) -> bool {
        self.is_numeric_like() || matches!(self, ColumnType::Date | ColumnType::DateTime)
    }
}

pub fn is_valid_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn primary_key_name(table: Option<&TableSchema>) -> &str {
    table.and_then(|table| table.primary_key.as_deref()).unwrap_or("id")
}

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

    let dbml = folder.join("schema.dbml");
    if dbml.exists() {
        return Ok(Some(dbml));
    }

    Ok(None)
}

fn parse_schema_file(path: &Path, raw: &str) -> Result<DeclaredSchema, String> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => parse_json_schema(raw),
        _ => parse_dbml_schema(raw),
    }
}

pub fn parse_json_schema(input: &str) -> Result<DeclaredSchema, String> {
    serde_json::from_str(input).map_err(|err| format!("invalid schema json: {err}"))
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

fn build_table_aliases(tables: &BTreeMap<String, TableSchema>) -> HashMap<String, String> {
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

fn singularize_table_name(table_name: &str) -> String {
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

fn detect_foreign_keys(
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
        if !column_types_are_compatible(&column.column_type, &target_column.column_type) {
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

fn infer_table_kind(
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

fn infer_declared_table_kind(
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

fn derive_many_to_many_schema(mut schema: Schema) -> Schema {
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

fn validate_effective_schema(schema: &Schema) -> Result<(), String> {
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

fn validate_declared_schema(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert_eq!(users.kind, Some(TableKind::Object));
        assert_eq!(users.primary_key.as_deref(), Some("id"));
    }

    #[test]
    fn parses_extended_dbml_column_types() {
        let schema = parse_dbml_schema(
            r#"
            Table events {
              id uuid [pk]
              starts_on date
              starts_at timestamptz
              amount numeric
              counter bigint
            }
            "#,
        )
        .expect("parse schema");

        let events = schema.tables.get("events").expect("events table");
        assert_eq!(events.columns["id"].column_type, ColumnType::Uuid);
        assert_eq!(events.columns["starts_on"].column_type, ColumnType::Date);
        assert_eq!(events.columns["starts_at"].column_type, ColumnType::DateTime);
        assert_eq!(events.columns["amount"].column_type, ColumnType::Decimal);
        assert_eq!(events.columns["counter"].column_type, ColumnType::BigInteger);
    }

    #[test]
    fn validates_declared_column_constraints() {
        let inferred = infer_schema_from_values(&BTreeMap::from([(
            "posts".to_string(),
            json!([{"id": 1, "status": "draft", "slug": "hello", "score": 3, "published_on": "2026-04-29"}]),
        )]));
        let mut status = ColumnSchema::new(ColumnType::String, false);
        status.enum_values = Some(vec!["draft".to_string(), "published".to_string()]);
        let mut slug = ColumnSchema::new(ColumnType::String, false);
        slug.min_length = Some(3);
        slug.max_length = Some(20);
        slug.pattern = Some("^[a-z0-9-]+$".to_string());
        let mut score = ColumnSchema::new(ColumnType::Integer, false);
        score.min = Some(Value::from(1));
        score.max = Some(Value::from(5));
        let mut published_on = ColumnSchema::new(ColumnType::Date, false);
        published_on.min = Some(Value::from("2026-01-01"));
        published_on.max = Some(Value::from("2026-12-31"));

        let declared = DeclaredSchema {
            tables: BTreeMap::from([(
                "posts".to_string(),
                DeclaredTableSchema {
                    columns: BTreeMap::from([
                        ("status".to_string(), status),
                        ("slug".to_string(), slug),
                        ("score".to_string(), score),
                        ("published_on".to_string(), published_on),
                    ]),
                    unique: vec![vec!["slug".to_string()]],
                    ..DeclaredTableSchema::default()
                },
            )]),
        };

        merge_schemas(Some(&declared), &inferred).expect("valid constraints");
    }

    #[test]
    fn rejects_invalid_declared_constraints() {
        let inferred = infer_schema_from_values(&BTreeMap::from([(
            "posts".to_string(),
            json!([{"id": 1, "status": "draft"}]),
        )]));
        let mut status = ColumnSchema::new(ColumnType::String, false);
        status.enum_values = Some(vec!["draft".to_string(), "draft".to_string()]);
        let declared = DeclaredSchema {
            tables: BTreeMap::from([(
                "posts".to_string(),
                DeclaredTableSchema {
                    columns: BTreeMap::from([("status".to_string(), status)]),
                    ..DeclaredTableSchema::default()
                },
            )]),
        };

        let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid constraints");
        assert!(err.contains("duplicate enum value"));
    }

    #[test]
    fn rejects_invalid_unique_constraints() {
        let inferred = infer_schema_from_values(&BTreeMap::from([(
            "posts".to_string(),
            json!([{"id": 1, "slug": "hello"}]),
        )]));
        let declared = DeclaredSchema {
            tables: BTreeMap::from([(
                "posts".to_string(),
                DeclaredTableSchema {
                    unique: vec![vec!["missing".to_string()]],
                    ..DeclaredTableSchema::default()
                },
            )]),
        };

        let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid unique");
        assert!(err.contains("unique constraint references unknown column 'missing'"));
    }

    #[test]
    fn rejects_invalid_identifiers() {
        let err = parse_dbml_schema(
            r#"
            Table users {
              bad$name int
            }
            "#,
        )
        .expect_err("invalid schema");
        assert!(err.contains("invalid column name"));
    }

    #[test]
    fn parses_inline_and_top_level_refs() {
        let schema = parse_dbml_schema(
            r#"
            Table users {
              id int [pk]
              name varchar
            }

            Table posts {
              id int [pk]
              user_id int [ref: > users.id]
            }

            Ref: posts.user_id > users.id
            "#,
        )
        .expect("parse schema");

        let posts = schema.tables.get("posts").expect("posts table");
        let fk = posts.foreign_keys.get("user_id").expect("user_id foreign key");
        assert_eq!(fk.target_table, "users");
        assert_eq!(fk.target_column, "id");
    }

    #[test]
    fn infers_object_and_relation_tables() {
        let schema = infer_schema_from_values(&BTreeMap::from([
            (
                "students".to_string(),
                json!([
                    {"id": 1, "name": "Ada"},
                    {"id": 2, "name": "Grace"}
                ]),
            ),
            (
                "courses".to_string(),
                json!([
                    {"id": 10, "title": "Math"},
                    {"id": 11, "title": "CS"}
                ]),
            ),
            (
                "student_courses".to_string(),
                json!([
                    {"student_id": 1, "course_id": 10},
                    {"student_id": 2, "course_id": 11}
                ]),
            ),
        ]));

        let students = schema.tables.get("students").expect("students table");
        assert_eq!(students.kind, TableKind::Object);
        assert_eq!(students.primary_key.as_deref(), Some("id"));

        let relation = schema.tables.get("student_courses").expect("relation table");
        assert_eq!(relation.kind, TableKind::Relation);
        assert_eq!(relation.foreign_keys["student_id"].target_table, "students");
        assert_eq!(relation.foreign_keys["course_id"].target_table, "courses");
        assert_eq!(students.many_to_many["courses"].through_table, "student_courses");
        assert_eq!(students.many_to_many["courses"].source_column, "student_id");
        assert_eq!(students.many_to_many["courses"].through_target_column, "course_id");
        assert_eq!(
            schema.tables["courses"].many_to_many["students"].through_table,
            "student_courses"
        );
    }

    #[test]
    fn strict_junction_detection_avoids_false_positives() {
        let schema = infer_schema_from_values(&BTreeMap::from([
            (
                "students".to_string(),
                json!([
                    {"id": 1, "name": "Ada"},
                    {"id": 2, "name": "Grace"}
                ]),
            ),
            (
                "courses".to_string(),
                json!([
                    {"id": 10, "title": "Math"},
                    {"id": 11, "title": "CS"}
                ]),
            ),
            (
                "student_courses".to_string(),
                json!([
                    {"student_id": 1, "course_id": 10, "role": "lead"},
                    {"student_id": 2, "course_id": 11, "role": "assistant"}
                ]),
            ),
            (
                "labels".to_string(),
                json!([
                    {"student_id": 1, "label": "mentor"},
                    {"student_id": 2, "label": "helper"}
                ]),
            ),
        ]));

        assert_eq!(schema.tables["student_courses"].kind, TableKind::Unknown);
        assert!(schema.tables["student_courses"].many_to_many.is_empty());
        assert_eq!(schema.tables["labels"].kind, TableKind::Unknown);
        assert!(schema.tables["labels"].foreign_keys.contains_key("student_id"));
        assert!(schema.tables["labels"].many_to_many.is_empty());
    }

    #[test]
    fn infers_non_id_primary_keys() {
        let schema = infer_schema_from_values(&BTreeMap::from([(
            "users".to_string(),
            json!([
                {"user_id": 1, "name": "Ada"},
                {"user_id": 2, "name": "Grace"}
            ]),
        )]));

        assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("user_id"));
    }

    #[test]
    fn merges_declared_foreign_key_over_inferred_data() {
        let inferred = infer_schema_from_values(&BTreeMap::from([
            (
                "users".to_string(),
                json!([
                    {"user_id": 1, "name": "Ada"},
                    {"user_id": 2, "name": "Grace"}
                ]),
            ),
            (
                "posts".to_string(),
                json!([
                    {"id": 1, "author_id": 1}
                ]),
            ),
        ]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
        let posts = merged.tables.get("posts").expect("posts table");
        assert_eq!(posts.foreign_keys["author_id"].target_column, "user_id");
    }

    #[test]
    fn partial_declared_schema_preserves_inferred_columns() {
        let inferred = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"user_id": 1, "name": "Ada"}])),
            (
                "posts".to_string(),
                json!([
                    {"id": 1, "author_id": 1, "title": "Hello"}
                ]),
            ),
        ]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
        let posts = merged.tables.get("posts").expect("posts table");
        assert!(posts.columns.contains_key("title"));
        assert!(posts.columns.contains_key("author_id"));
    }

    #[test]
    fn suppressed_foreign_keys_remove_inferred_relations() {
        let inferred = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"id": 1, "name": "Ada"}])),
            ("posts".to_string(), json!([{"id": 1, "user_id": 1}])),
        ]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "posts": {
                  "suppressed_foreign_keys": ["user_id"]
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let merged = merge_schemas(Some(&declared), &inferred).expect("merge schema");
        let posts = merged.tables.get("posts").expect("posts table");
        assert!(posts.foreign_keys.get("user_id").is_none(), "{posts:?}");
    }

    #[test]
    fn export_declared_snapshot_preserves_suppressed_foreign_keys() {
        let effective = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"id": 1, "name": "Ada"}])),
            ("posts".to_string(), json!([{"id": 1, "user_id": 1}])),
        ]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "posts": {
                  "suppressed_foreign_keys": ["user_id"]
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let snapshot = export_declared_schema_snapshot(Some(&declared), &effective);
        assert_eq!(
            snapshot.tables["posts"].suppressed_foreign_keys,
            BTreeSet::from(["user_id".to_string()])
        );
    }

    #[test]
    fn parses_schema_json() {
        let raw = r#"
        {
          "tables": {
            "users": {
              "kind": "object",
              "primary_key": "id",
              "columns": {
                "id": {"column_type": "integer", "nullable": false}
              },
              "foreign_keys": {}
            }
          }
        }
        "#;

        let schema = parse_json_schema(raw).expect("parse schema json");
        assert_eq!(schema.tables["users"].kind, Some(TableKind::Object));
        assert_eq!(schema.tables["users"].primary_key.as_deref(), Some("id"));
    }

    #[test]
    fn validates_foreign_key_targets() {
        let inferred = infer_schema_from_values(&BTreeMap::from([(
            "posts".to_string(),
            json!([{"id": 1, "author_id": 1}]),
        )]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "id"}
                  }
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid fk");
        assert!(err.contains("targets unknown table"));
    }

    #[test]
    fn validates_foreign_key_type_compatibility() {
        let inferred = infer_schema_from_values(&BTreeMap::from([
            ("users".to_string(), json!([{"user_id": "user-1"}])),
            ("posts".to_string(), json!([{"author_id": 1}])),
        ]));
        let declared = parse_json_schema(
            r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_id": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
        )
        .expect("parse schema");

        let err = merge_schemas(Some(&declared), &inferred).expect_err("invalid fk type");
        assert!(err.contains("incompatible"));
    }

    #[test]
    fn dbml_and_json_yield_same_effective_schema() {
        let inferred = infer_schema_from_values(&BTreeMap::from([
            (
                "users".to_string(),
                json!([
                    {"user_id": 1, "name": "Ada"},
                    {"user_id": 2, "name": "Grace"}
                ]),
            ),
            (
                "posts".to_string(),
                json!([
                    {"id": 1, "author_ref": 1, "title": "Hello"}
                ]),
            ),
        ]));
        let dbml = parse_dbml_schema(
            r#"
            Table users {
              user_id int [pk]
              name varchar
            }

            Table posts {
              id int [pk]
              author_ref int
            }

            Ref: posts.author_ref > users.user_id
            "#,
        )
        .expect("parse dbml");
        let json = parse_json_schema(
            r#"
            {
              "tables": {
                "users": {
                  "primary_key": "user_id"
                },
                "posts": {
                  "foreign_keys": {
                    "author_ref": {"target_table": "users", "target_column": "user_id"}
                  }
                }
              }
            }
            "#,
        )
        .expect("parse json");

        let from_dbml = merge_schemas(Some(&dbml), &inferred).expect("merge dbml");
        let from_json = merge_schemas(Some(&json), &inferred).expect("merge json");
        assert_eq!(from_dbml.tables["users"].primary_key, from_json.tables["users"].primary_key);
        assert_eq!(from_dbml.tables["posts"].foreign_keys, from_json.tables["posts"].foreign_keys);
    }
}
