use crate::schema::DeclaredSchema;

pub fn parse_json_schema(input: &str) -> Result<DeclaredSchema, String> {
    serde_json::from_str(input).map_err(|err| format!("invalid schema json: {err}"))
}
