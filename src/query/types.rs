#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperator {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    In,
    Contains,
    StartsWith,
    EndsWith,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone)]
pub struct FilterCondition {
    pub field_path: String,
    pub(super) field_segments: Vec<String>,
    pub operator: FilterOperator,
    pub value: String,
    pub(super) value_lower: String,
    pub(super) prepared_value: ComparableValue,
    pub(super) prepared_in_values: Vec<ComparableValue>,
}

impl FilterCondition {
    pub fn new(field_path: String, operator: FilterOperator, value: String) -> Self {
        Self {
            field_segments: field_path.split('.').map(str::to_string).collect(),
            field_path,
            operator,
            value_lower: value.to_lowercase(),
            prepared_value: prepare_expected_value(&value),
            prepared_in_values: prepare_in_values(&value),
            value,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SortColumn {
    pub field_path: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Pagination {
    pub page: usize,
    pub per_page: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PaginationWindow {
    pub first: usize,
    pub prev: Option<usize>,
    pub next: Option<usize>,
    pub last: usize,
    pub page: usize,
    pub pages: usize,
    pub items: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Default)]
pub struct ParsedCollectionQuery {
    pub filters: Vec<FilterCondition>,
    pub sort_columns: Vec<SortColumn>,
    pub pagination: Option<Pagination>,
    pub embeds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum ComparableValue {
    Null,
    Number(f64),
    Bool(bool),
    String(String),
}

fn prepare_expected_value(expected: &str) -> ComparableValue {
    if expected.eq_ignore_ascii_case("null") {
        return ComparableValue::Null;
    }
    if let Ok(number) = expected.parse::<f64>() {
        return ComparableValue::Number(number);
    }
    if let Ok(boolean) = expected.parse::<bool>() {
        return ComparableValue::Bool(boolean);
    }
    ComparableValue::String(expected.to_string())
}

fn prepare_in_values(value: &str) -> Vec<ComparableValue> {
    value.split(',').map(str::trim).filter(|v| !v.is_empty()).map(prepare_expected_value).collect()
}
