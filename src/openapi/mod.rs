use std::collections::BTreeMap;

use axum::{Json, extract::State};
use serde_json::{Map, Value, json};

use crate::{
    app::AppState,
    error::{AppError, ERROR_CODE_UNAUTHORIZED},
    schema::{ColumnSchema, ColumnType, Schema, TableSchema, primary_key_name},
    storage::load_resource,
};

enum ResourceShape {
    Array,
    Object,
    Other,
}

struct ResourceDocumentSpec {
    name: String,
    operation_stem: String,
    row_schema_name: String,
    collection_schema_name: String,
    schema: Value,
    shape: ResourceShape,
    primary_key: String,
}

pub(crate) async fn get_openapi(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(build_openapi_document(&state).await?))
}

pub(crate) async fn build_openapi_document(state: &AppState) -> Result<Value, AppError> {
    let resources = state.resource_names_sorted().await;
    let _guards = state.read_locks_for_resources(&resources).await;
    let schema = state.schema_snapshot();
    let mut paths = Map::new();
    let mut component_schemas = common_component_schemas();
    let mut used_resource_stems = BTreeMap::new();
    let local_readonly = state.config.readonly || state.config.clone_proxy.is_some();

    add_static_paths(&mut paths, local_readonly);

    for resource in resources {
        let operation_stem =
            unique_pascal_identifier(&pascal_case_identifier(&resource), &mut used_resource_stems);
        let spec = resource_document_spec(state, &schema, &resource, operation_stem).await?;
        component_schemas.insert(spec.row_schema_name.clone(), spec.schema.clone());
        component_schemas.insert(
            spec.collection_schema_name.clone(),
            json!({
                "type": "array",
                "items": schema_ref(&spec.row_schema_name),
            }),
        );
        add_resource_paths(&mut paths, &spec, local_readonly);
    }

    Ok(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "dirbase",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "servers": [{ "url": "/" }],
        "paths": Value::Object(paths),
        "components": {
            "schemas": Value::Object(component_schemas),
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                }
            }
        },
    }))
}

async fn resource_document_spec(
    state: &AppState,
    schema: &Schema,
    resource: &str,
    operation_stem: String,
) -> Result<ResourceDocumentSpec, AppError> {
    let data = load_resource(state, resource).await?;
    let table = schema.tables.get(resource);
    let shape = if data.is_array() {
        ResourceShape::Array
    } else if data.is_object() {
        ResourceShape::Object
    } else {
        ResourceShape::Other
    };
    let row_schema_name = format!("{operation_stem}Resource");
    let collection_schema_name = format!("{operation_stem}Collection");
    let schema = match shape {
        ResourceShape::Array | ResourceShape::Object => object_schema(table),
        ResourceShape::Other => json!(true),
    };
    let primary_key = primary_key_name(table).to_string();

    Ok(ResourceDocumentSpec {
        name: resource.to_string(),
        operation_stem,
        row_schema_name,
        collection_schema_name,
        schema,
        shape,
        primary_key,
    })
}

fn add_static_paths(paths: &mut Map<String, Value>, readonly: bool) {
    insert_operation(
        paths,
        "/",
        "get",
        operation(
            "listRootResources",
            "List resources or render the overview UI",
            "Returns resource names for API clients and the overview HTML for browser requests.",
            Vec::new(),
            None,
            json_response("Available resources.", schema_ref("ResourceList")),
        ),
    );
    insert_operation(
        paths,
        "/resources",
        "get",
        operation(
            "listResources",
            "List resources",
            "Returns the available resource names.",
            Vec::new(),
            None,
            json_response("Available resources.", schema_ref("ResourceList")),
        ),
    );
    if !readonly {
        insert_operation(
            paths,
            "/resources",
            "post",
            operation(
                "createResource",
                "Create a resource",
                "Creates a JSON resource in folder mode or a top-level key in file mode.",
                Vec::new(),
                Some(json_request_body(schema_ref("ResourceCreateRequest"))),
                created_json_response("Resource created.", json!(true)),
            ),
        );
        insert_operation(
            paths,
            "/resources/{resource}",
            "delete",
            operation(
                "deleteResource",
                "Delete a resource",
                "Deletes a JSON resource in folder mode or a top-level key in file mode.",
                vec![
                    json!({
                        "name": "resource",
                        "in": "path",
                        "required": true,
                        "description": "Resource name.",
                        "schema": { "type": "string" },
                    }),
                    query_parameter(
                        "confirm",
                        "Must be true to delete a resource.",
                        json!({ "type": "boolean" }),
                    ),
                ],
                None,
                no_content_response("Resource deleted."),
            ),
        );
    }

    insert_operation(
        paths,
        "/overview.json",
        "get",
        operation(
            "getOverview",
            "Get overview metadata",
            "Returns the data used by the embedded overview UI.",
            Vec::new(),
            None,
            json_response("Overview metadata.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/openapi.json",
        "get",
        operation(
            "getOpenApiDocument",
            "Get OpenAPI document",
            "Returns the schema-derived OpenAPI 3.1 description for this server.",
            Vec::new(),
            None,
            json_response("OpenAPI document.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/schema",
        "get",
        operation(
            "getSchema",
            "Get effective schema",
            "Returns inferred schema metadata merged with any declared overlay.",
            Vec::new(),
            None,
            json_response("Effective schema.", json!(true)),
        ),
    );
    if !readonly {
        insert_operation(
            paths,
            "/schema",
            "post",
            operation(
                "saveEffectiveSchema",
                "Save effective schema",
                "Persists the current effective schema as schema.json.",
                Vec::new(),
                None,
                json_response("Schema saved.", json!(true)),
            ),
        );
        insert_operation(
            paths,
            "/schema",
            "put",
            operation(
                "saveDeclaredSchema",
                "Save declared schema",
                "Validates and persists a declared schema overlay.",
                Vec::new(),
                Some(json_request_body(json!(true))),
                json_response("Declared schema saved.", json!(true)),
            ),
        );
        insert_operation(
            paths,
            "/schema/infer",
            "post",
            operation(
                "inferAndSaveSchema",
                "Infer and save schema",
                "Re-infers schema metadata from data and persists it as schema.json.",
                Vec::new(),
                None,
                json_response("Inferred schema saved.", json!(true)),
            ),
        );
    }
    insert_operation(
        paths,
        "/schema/editor",
        "get",
        operation(
            "getSchemaEditor",
            "Get schema editor state",
            "Returns inferred, declared, and effective schema documents for the overview UI.",
            Vec::new(),
            None,
            json_response("Schema editor state.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/graphql",
        "get",
        operation(
            "getGraphql",
            "Open GraphiQL",
            "Serves GraphiQL for browser requests and accepts GraphQL query parameters.",
            Vec::new(),
            None,
            text_response("GraphiQL HTML or GraphQL response."),
        ),
    );
    insert_operation(
        paths,
        "/graphql",
        "post",
        operation(
            "postGraphql",
            "Execute GraphQL",
            "Executes a GraphQL JSON request body.",
            Vec::new(),
            Some(json_request_body(schema_ref("GraphqlRequest"))),
            json_response("GraphQL response.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/sql",
        "get",
        operation(
            "querySql",
            "Run SQL query",
            "Runs a read-only SQL SELECT query from the q query parameter.",
            vec![query_parameter("q", "SQL SELECT query.", json!({ "type": "string" }))],
            None,
            json_response("SQL query result.", json!(true)),
        ),
    );
    if !readonly {
        insert_operation(
            paths,
            "/sql",
            "post",
            operation(
                "postSqlQuery",
                "Run SQL query",
                "Runs a read-only SQL SELECT query from a JSON request body.",
                Vec::new(),
                Some(json_request_body(schema_ref("SqlRequest"))),
                json_response("SQL query result.", json!(true)),
            ),
        );
    }
    insert_operation(
        paths,
        "/export.sql",
        "get",
        operation(
            "exportSql",
            "Export SQL",
            "Exports the current data as SQL.",
            vec![query_parameter(
                "dialect",
                "SQL dialect, either postgres or sqlite.",
                json!({ "type": "string", "enum": ["postgres", "sqlite"] }),
            )],
            None,
            sql_response("SQL export."),
        ),
    );
    insert_operation(
        paths,
        "/sql/export",
        "get",
        operation(
            "exportSqlAlias",
            "Export SQL",
            "Alias for /export.sql.",
            vec![query_parameter(
                "dialect",
                "SQL dialect, either postgres or sqlite.",
                json!({ "type": "string", "enum": ["postgres", "sqlite"] }),
            )],
            None,
            sql_response("SQL export."),
        ),
    );
    insert_operation(
        paths,
        "/events",
        "get",
        operation(
            "getEvents",
            "Subscribe to events",
            "Streams overview_changed, resource_changed, and schema_changed server-sent events.",
            Vec::new(),
            None,
            event_stream_response("Server-sent event stream."),
        ),
    );
    insert_operation(
        paths,
        "/healthz",
        "get",
        operation(
            "healthz",
            "Health check",
            "Reports process liveness.",
            Vec::new(),
            None,
            json_response("Health status.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/readyz",
        "get",
        operation(
            "readyz",
            "Readiness check",
            "Reports whether the current data source is readable.",
            Vec::new(),
            None,
            json_response("Readiness status.", json!(true)),
        ),
    );
    insert_operation(
        paths,
        "/metrics",
        "get",
        operation(
            "metrics",
            "Prometheus metrics",
            "Returns Prometheus text metrics.",
            Vec::new(),
            None,
            text_response("Prometheus metrics."),
        ),
    );
}

fn add_resource_paths(paths: &mut Map<String, Value>, spec: &ResourceDocumentSpec, readonly: bool) {
    let resource_path = format!("/{}", spec.name);
    match spec.shape {
        ResourceShape::Array => {
            insert_operation(
                paths,
                &resource_path,
                "get",
                operation(
                    &format!("get{}Collection", spec.operation_stem),
                    &format!("List {}", spec.name),
                    "Returns the resource collection. Filtering, sorting, pagination, and embeds are available through query parameters.",
                    collection_query_parameters(),
                    None,
                    json_response(
                        "Collection response.",
                        json!({
                            "oneOf": [
                                schema_ref(&spec.collection_schema_name),
                                schema_ref("PaginationEnvelope"),
                            ]
                        }),
                    ),
                ),
            );
            if !readonly {
                insert_operation(
                    paths,
                    &resource_path,
                    "post",
                    operation(
                        &format!("create{}Item", spec.operation_stem),
                        &format!("Create {}", spec.name),
                        "Appends an object to the resource collection and persists it.",
                        Vec::new(),
                        Some(json_request_body(schema_ref(&spec.row_schema_name))),
                        created_json_response("Created item.", schema_ref(&spec.row_schema_name)),
                    ),
                );
            }

            let item_path = format!("{resource_path}/{{id}}");
            let id_parameter = id_path_parameter(&spec.primary_key);
            insert_operation(
                paths,
                &item_path,
                "get",
                operation(
                    &format!("get{}Item", spec.operation_stem),
                    &format!("Get {} item", spec.name),
                    "Returns one item from the collection.",
                    vec![id_parameter.clone()],
                    None,
                    json_response("Resource item.", schema_ref(&spec.row_schema_name)),
                ),
            );
            if !readonly {
                insert_operation(
                    paths,
                    &item_path,
                    "put",
                    operation(
                        &format!("replace{}Item", spec.operation_stem),
                        &format!("Replace {} item", spec.name),
                        "Replaces one item in the collection and persists it.",
                        vec![id_parameter.clone()],
                        Some(json_request_body(schema_ref(&spec.row_schema_name))),
                        json_response("Replaced item.", schema_ref(&spec.row_schema_name)),
                    ),
                );
                insert_operation(
                    paths,
                    &item_path,
                    "patch",
                    operation(
                        &format!("patch{}Item", spec.operation_stem),
                        &format!("Patch {} item", spec.name),
                        "Merges fields into one item in the collection and persists it.",
                        vec![id_parameter.clone()],
                        Some(json_request_body(schema_ref(&spec.row_schema_name))),
                        json_response("Patched item.", schema_ref(&spec.row_schema_name)),
                    ),
                );
                insert_operation(
                    paths,
                    &item_path,
                    "delete",
                    operation(
                        &format!("delete{}Item", spec.operation_stem),
                        &format!("Delete {} item", spec.name),
                        "Deletes one item from the collection and persists it.",
                        vec![id_parameter],
                        None,
                        no_content_response("Deleted item."),
                    ),
                );
            }
        }
        ResourceShape::Object => {
            insert_operation(
                paths,
                &resource_path,
                "get",
                operation(
                    &format!("get{}Object", spec.operation_stem),
                    &format!("Get {}", spec.name),
                    "Returns the object resource.",
                    Vec::new(),
                    None,
                    json_response("Object resource.", schema_ref(&spec.row_schema_name)),
                ),
            );
            if !readonly {
                insert_operation(
                    paths,
                    &resource_path,
                    "put",
                    operation(
                        &format!("replace{}Object", spec.operation_stem),
                        &format!("Replace {}", spec.name),
                        "Replaces the object resource and persists it.",
                        Vec::new(),
                        Some(json_request_body(schema_ref(&spec.row_schema_name))),
                        json_response(
                            "Replaced object resource.",
                            schema_ref(&spec.row_schema_name),
                        ),
                    ),
                );
                insert_operation(
                    paths,
                    &resource_path,
                    "patch",
                    operation(
                        &format!("patch{}Object", spec.operation_stem),
                        &format!("Patch {}", spec.name),
                        "Merges fields into the object resource and persists it.",
                        Vec::new(),
                        Some(json_request_body(schema_ref(&spec.row_schema_name))),
                        json_response(
                            "Patched object resource.",
                            schema_ref(&spec.row_schema_name),
                        ),
                    ),
                );
            }
        }
        ResourceShape::Other => {
            insert_operation(
                paths,
                &resource_path,
                "get",
                operation(
                    &format!("get{}Resource", spec.operation_stem),
                    &format!("Get {}", spec.name),
                    "Returns the raw JSON resource.",
                    Vec::new(),
                    None,
                    json_response("Raw JSON resource.", schema_ref(&spec.row_schema_name)),
                ),
            );
        }
    }
}

fn common_component_schemas() -> Map<String, Value> {
    let mut schemas = Map::new();
    schemas.insert(
        "ErrorResponse".to_string(),
        json!({
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": { "type": "string" },
                "code": { "type": "string" },
            },
            "additionalProperties": false,
        }),
    );
    schemas.insert(
        "ResourceList".to_string(),
        json!({
            "type": "object",
            "required": ["resources"],
            "properties": {
                "resources": {
                    "type": "array",
                    "items": { "type": "string" },
                }
            },
            "additionalProperties": false,
        }),
    );
    schemas.insert(
        "ResourceCreateRequest".to_string(),
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "initial": true,
            },
            "additionalProperties": false,
        }),
    );
    schemas.insert(
        "PaginationEnvelope".to_string(),
        json!({
            "type": "object",
            "required": ["first", "prev", "next", "last", "pages", "items", "data"],
            "properties": {
                "first": { "type": "integer" },
                "prev": { "type": ["integer", "null"] },
                "next": { "type": ["integer", "null"] },
                "last": { "type": "integer" },
                "pages": { "type": "integer" },
                "items": { "type": "integer" },
                "data": { "type": "array", "items": true },
            },
            "additionalProperties": true,
        }),
    );
    schemas.insert(
        "GraphqlRequest".to_string(),
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" },
                "variables": { "type": "object", "additionalProperties": true },
                "operationName": { "type": ["string", "null"] },
            },
            "additionalProperties": true,
        }),
    );
    schemas.insert(
        "SqlRequest".to_string(),
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" },
            },
            "additionalProperties": false,
        }),
    );
    schemas
}

fn object_schema(table: Option<&TableSchema>) -> Value {
    let Some(table) = table else {
        return permissive_object_schema();
    };
    if table.columns.is_empty() {
        return permissive_object_schema();
    }

    let mut properties = Map::new();
    let mut required = Vec::new();
    for (column_name, column) in &table.columns {
        properties.insert(column_name.clone(), column_schema(column));
        if !column.nullable {
            required.push(Value::String(column_name.clone()));
        }
    }

    json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": required,
        "additionalProperties": true,
    })
}

fn permissive_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
    })
}

fn column_schema(column: &ColumnSchema) -> Value {
    let mut schema = Map::new();
    match column.column_type {
        ColumnType::Json => {
            return json!(true);
        }
        ColumnType::Integer => {
            schema.insert("type".to_string(), type_schema("integer", column.nullable));
        }
        ColumnType::Float | ColumnType::Decimal => {
            schema.insert("type".to_string(), type_schema("number", column.nullable));
        }
        ColumnType::Boolean => {
            schema.insert("type".to_string(), type_schema("boolean", column.nullable));
        }
        ColumnType::String => {
            schema.insert("type".to_string(), type_schema("string", column.nullable));
        }
        ColumnType::Date => {
            schema.insert("type".to_string(), type_schema("string", column.nullable));
            schema.insert("format".to_string(), json!("date"));
        }
        ColumnType::DateTime => {
            schema.insert("type".to_string(), type_schema("string", column.nullable));
            schema.insert("format".to_string(), json!("date-time"));
        }
        ColumnType::Uuid => {
            schema.insert("type".to_string(), type_schema("string", column.nullable));
            schema.insert("format".to_string(), json!("uuid"));
        }
        ColumnType::BigInteger => {
            schema.insert("type".to_string(), type_schema("string", column.nullable));
            schema.insert(
                "description".to_string(),
                json!("Big integer encoded as string-compatible value"),
            );
        }
    }

    if let Some(values) = &column.enum_values {
        schema.insert("enum".to_string(), json!(values));
    }
    if let Some(min) = &column.min
        && is_numeric_column(&column.column_type)
    {
        schema.insert("minimum".to_string(), min.clone());
    }
    if let Some(max) = &column.max
        && is_numeric_column(&column.column_type)
    {
        schema.insert("maximum".to_string(), max.clone());
    }
    if let Some(min_length) = column.min_length {
        schema.insert("minLength".to_string(), json!(min_length));
    }
    if let Some(max_length) = column.max_length {
        schema.insert("maxLength".to_string(), json!(max_length));
    }
    if let Some(pattern) = &column.pattern {
        schema.insert("pattern".to_string(), json!(pattern));
    }

    Value::Object(schema)
}

fn type_schema(name: &str, nullable: bool) -> Value {
    if nullable { json!([name, "null"]) } else { json!(name) }
}

fn is_numeric_column(column_type: &ColumnType) -> bool {
    matches!(column_type, ColumnType::Integer | ColumnType::Float | ColumnType::Decimal)
}

fn collection_query_parameters() -> Vec<Value> {
    vec![
        query_parameter(
            "field",
            "Any resource field, nested field, or field:operator pair may be supplied as a query parameter. Values are interpreted as REST filters.",
            json!({
                "type": "object",
                "additionalProperties": { "type": "string" },
            }),
        ),
        query_parameter(
            "sort",
            "Comma-separated sort columns. Prefix a column with '-' for descending order.",
            json!({ "type": "string" }),
        ),
        query_parameter("_sort", "Alias for sort.", json!({ "type": "string" })),
        query_parameter("page", "Page number.", json!({ "type": "integer", "minimum": 1 })),
        query_parameter("_page", "Alias for page.", json!({ "type": "integer", "minimum": 1 })),
        query_parameter("per_page", "Items per page.", json!({ "type": "integer", "minimum": 1 })),
        query_parameter(
            "_per_page",
            "Alias for per_page.",
            json!({ "type": "integer", "minimum": 1 }),
        ),
        query_parameter(
            "embed",
            "Foreign-key column to embed. May be repeated.",
            json!({ "type": "string" }),
        ),
        query_parameter("_embed", "Alias for embed.", json!({ "type": "string" })),
    ]
}

fn id_path_parameter(primary_key: &str) -> Value {
    json!({
        "name": "id",
        "in": "path",
        "required": true,
        "description": format!("Resource item identifier. Uses primary key '{primary_key}'."),
        "schema": { "type": "string" },
    })
}

fn query_parameter(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "in": "query",
        "required": false,
        "description": description,
        "schema": schema,
    })
}

fn operation(
    operation_id: &str,
    summary: &str,
    description: &str,
    parameters: Vec<Value>,
    request_body: Option<Value>,
    responses: Value,
) -> Value {
    let mut operation = Map::new();
    operation.insert("operationId".to_string(), json!(operation_id));
    operation.insert("summary".to_string(), json!(summary));
    operation.insert("description".to_string(), json!(description));
    operation.insert("parameters".to_string(), Value::Array(parameters));
    if let Some(request_body) = request_body {
        operation.insert("requestBody".to_string(), request_body);
    }
    operation.insert("responses".to_string(), responses);
    operation.insert(
        "security".to_string(),
        json!([
            { "bearerAuth": [] },
            {},
        ]),
    );
    Value::Object(operation)
}

fn json_request_body(schema: Value) -> Value {
    json!({
        "required": true,
        "content": {
            "application/json": {
                "schema": schema,
            }
        }
    })
}

fn json_response(description: &str, schema: Value) -> Value {
    let mut responses = standard_error_responses();
    responses.insert(
        "200".to_string(),
        json!({
            "description": description,
            "content": {
                "application/json": {
                    "schema": schema,
                }
            }
        }),
    );
    Value::Object(responses)
}

fn created_json_response(description: &str, schema: Value) -> Value {
    let mut responses = standard_error_responses();
    responses.insert(
        "201".to_string(),
        json!({
            "description": description,
            "content": {
                "application/json": {
                    "schema": schema,
                }
            }
        }),
    );
    Value::Object(responses)
}

fn no_content_response(description: &str) -> Value {
    let mut responses = standard_error_responses();
    responses.insert("204".to_string(), json!({ "description": description }));
    Value::Object(responses)
}

fn text_response(description: &str) -> Value {
    let mut responses = standard_error_responses();
    responses.insert(
        "200".to_string(),
        json!({
            "description": description,
            "content": {
                "text/plain": {
                    "schema": { "type": "string" },
                },
                "text/html": {
                    "schema": { "type": "string" },
                },
            }
        }),
    );
    Value::Object(responses)
}

fn sql_response(description: &str) -> Value {
    let mut responses = standard_error_responses();
    responses.insert(
        "200".to_string(),
        json!({
            "description": description,
            "content": {
                "text/sql": {
                    "schema": { "type": "string" },
                }
            }
        }),
    );
    Value::Object(responses)
}

fn event_stream_response(description: &str) -> Value {
    let mut responses = standard_error_responses();
    responses.insert(
        "200".to_string(),
        json!({
            "description": description,
            "content": {
                "text/event-stream": {
                    "schema": { "type": "string" },
                }
            }
        }),
    );
    Value::Object(responses)
}

fn standard_error_responses() -> Map<String, Value> {
    Map::from_iter([
        (
            "400".to_string(),
            json!({
                "description": "Bad request.",
                "content": {
                    "application/json": {
                        "schema": schema_ref("ErrorResponse"),
                    }
                }
            }),
        ),
        (
            "401".to_string(),
            json!({
                "description": "Missing or invalid bearer token.",
                "content": {
                    "application/json": {
                        "schema": schema_ref("ErrorResponse"),
                        "example": {
                            "error": "Missing or invalid bearer token",
                            "code": ERROR_CODE_UNAUTHORIZED,
                        },
                    }
                }
            }),
        ),
        (
            "404".to_string(),
            json!({
                "description": "Resource or item not found.",
                "content": {
                    "application/json": {
                        "schema": schema_ref("ErrorResponse"),
                    }
                }
            }),
        ),
    ])
}

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

fn insert_operation(paths: &mut Map<String, Value>, path: &str, method: &str, operation: Value) {
    let entry = paths.entry(path.to_string()).or_insert_with(|| Value::Object(Map::new()));
    let path_item = entry.as_object_mut().expect("OpenAPI path item");
    path_item.insert(method.to_string(), operation);
}

fn pascal_case_identifier(raw: &str) -> String {
    let mut output = String::new();
    let mut capitalize = true;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if capitalize {
                output.push(ch.to_ascii_uppercase());
                capitalize = false;
            } else {
                output.push(ch);
            }
        } else {
            capitalize = true;
        }
    }

    if output.is_empty() {
        output.push_str("Resource");
    }
    if output.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        output.insert_str(0, "Resource");
    }
    output
}

fn unique_pascal_identifier(base: &str, used: &mut BTreeMap<String, usize>) -> String {
    let count = used.entry(base.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 { base.to_string() } else { format!("{base}{count}") }
}
