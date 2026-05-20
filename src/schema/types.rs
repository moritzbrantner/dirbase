use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

    pub fn from_xsd_type(raw: &str) -> Self {
        match normalize_qname(raw).to_ascii_lowercase().as_str() {
            "int" | "integer" | "short" | "byte" | "nonnegativeinteger" | "nonpositiveinteger"
            | "positiveinteger" | "negativeinteger" | "unsignedbyte" | "unsignedshort" => {
                ColumnType::Integer
            }
            "long" | "unsignedint" | "unsignedlong" => ColumnType::BigInteger,
            "decimal" => ColumnType::Decimal,
            "float" | "double" => ColumnType::Float,
            "boolean" => ColumnType::Boolean,
            "date" => ColumnType::Date,
            "datetime" => ColumnType::DateTime,
            "string" | "normalizedstring" | "token" | "id" | "idref" | "language" | "name"
            | "ncname" | "nmtoken" | "anyuri" => ColumnType::String,
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
                    | (ColumnType::Float, ColumnType::BigInteger)
                    | (ColumnType::BigInteger, ColumnType::Float)
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

fn normalize_qname(name: &str) -> &str {
    name.rsplit_once(':').map(|(_, local)| local).unwrap_or(name)
}
