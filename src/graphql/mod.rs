use std::collections::{BTreeMap, BTreeSet};

use async_graphql::{
    Error as GraphqlError, Request as GraphqlRequest, Value as GraphqlValue,
    dynamic::{
        Field, FieldFuture, FieldValue, InputValue, Object, Scalar, Schema as DynamicSchema,
        TypeRef,
    },
    http::{GraphiQLSource, parse_query_string},
};
use async_graphql_axum::GraphQLResponse;
use axum::{
    Json,
    extract::State,
    http::{
        HeaderMap, StatusCode, Uri,
        header::{ACCEPT, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Response},
};
use serde_json::{Map as JsonMap, Value as JsonValue, json};

use crate::{
    app::AppState,
    error::AppError,
    relations::resolve_related_row,
    schema::{ColumnSchema, ColumnType, TableSchema, primary_key_name},
    storage::{find_item_by_key, load_resource, validate_resource_data},
};

#[derive(Clone, Debug)]
struct ObjectTypeSpec {
    source_resource: String,
    type_name: String,
    fields: Vec<ObjectFieldSpec>,
}

#[derive(Clone, Debug)]
struct ObjectFieldSpec {
    graphql_name: String,
    json_key: String,
    output: ObjectFieldOutput,
    nullable: bool,
}

#[derive(Clone, Debug)]
enum ObjectFieldOutput {
    Scalar(ScalarKind),
    Relation { source_column: String, target_type_name: String },
}

#[derive(Clone, Debug)]
enum RootFieldSpec {
    Collection {
        resource: String,
        graphql_name: String,
        row_type_name: String,
    },
    CollectionById {
        resource: String,
        graphql_name: String,
        row_type_name: String,
        primary_key: String,
    },
    Object {
        resource: String,
        graphql_name: String,
        type_name: String,
    },
    Json {
        resource: String,
        graphql_name: String,
    },
}

#[derive(Clone, Debug)]
struct GraphqlObjectValue {
    object: JsonMap<String, JsonValue>,
}

#[derive(Clone, Copy, Debug)]
enum ScalarKind {
    Int,
    Float,
    Boolean,
    String,
    Json,
}

pub async fn build_schema(state: &AppState) -> Result<DynamicSchema, String> {
    let resources = state.resource_names_sorted().await;
    let _guards = state.read_locks_for_resources(&resources).await;
    let resource_set = resources.iter().cloned().collect::<BTreeSet<_>>();
    let schema = state.schema_snapshot();

    let mut type_name_registry = BTreeMap::new();
    let mut root_field_registry = BTreeMap::new();
    let mut collection_type_names = BTreeMap::new();

    for resource in &resources {
        let Some(table) = schema.tables.get(resource) else {
            continue;
        };
        if table.columns.is_empty() {
            continue;
        }
        let type_name = register_graphql_name(
            &mut type_name_registry,
            collection_type_name(resource),
            format!("collection type for resource '{resource}'"),
            "GraphQL type names",
        )?;
        collection_type_names.insert(resource.clone(), type_name);
    }

    let mut object_types = Vec::new();
    let mut root_fields = Vec::new();

    for resource in &resources {
        if let Some(table) = schema.tables.get(resource)
            && let Some(row_type_name) = collection_type_names.get(resource).cloned()
        {
            let fields = build_collection_object_fields(
                resource,
                table,
                &collection_type_names,
                &resource_set,
            )?;
            if !fields.is_empty() {
                let collection_field_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(resource),
                    format!("resource '{resource}'"),
                    "GraphQL root fields",
                )?;
                root_fields.push(RootFieldSpec::Collection {
                    resource: resource.clone(),
                    graphql_name: collection_field_name,
                    row_type_name: row_type_name.clone(),
                });

                if table.primary_key.is_some() {
                    let by_id_name = register_graphql_name(
                        &mut root_field_registry,
                        normalize_graphql_name(&format!("{resource}ById")),
                        format!("single-item field for resource '{resource}'"),
                        "GraphQL root fields",
                    )?;
                    root_fields.push(RootFieldSpec::CollectionById {
                        resource: resource.clone(),
                        graphql_name: by_id_name,
                        row_type_name: row_type_name.clone(),
                        primary_key: primary_key_name(Some(table)).to_string(),
                    });
                }

                object_types.push(ObjectTypeSpec {
                    source_resource: resource.clone(),
                    type_name: row_type_name,
                    fields,
                });
                continue;
            }
        }

        let value = load_resource(state, resource).await.map_err(|err| {
            format!("GraphQL schema build failed for resource '{resource}': {}", err.message)
        })?;

        match value.as_ref() {
            JsonValue::Object(object) if !object.is_empty() => {
                let type_name = register_graphql_name(
                    &mut type_name_registry,
                    object_type_name(resource),
                    format!("object type for resource '{resource}'"),
                    "GraphQL type names",
                )?;
                let fields = build_object_resource_fields(resource, object)?;
                if fields.is_empty() {
                    let graphql_name = register_graphql_name(
                        &mut root_field_registry,
                        normalize_graphql_name(resource),
                        format!("resource '{resource}'"),
                        "GraphQL root fields",
                    )?;
                    root_fields
                        .push(RootFieldSpec::Json { resource: resource.clone(), graphql_name });
                    continue;
                }

                let graphql_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(resource),
                    format!("resource '{resource}'"),
                    "GraphQL root fields",
                )?;
                root_fields.push(RootFieldSpec::Object {
                    resource: resource.clone(),
                    graphql_name,
                    type_name: type_name.clone(),
                });
                object_types.push(ObjectTypeSpec {
                    source_resource: resource.clone(),
                    type_name,
                    fields,
                });
            }
            _ => {
                let graphql_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(resource),
                    format!("resource '{resource}'"),
                    "GraphQL root fields",
                )?;
                root_fields.push(RootFieldSpec::Json { resource: resource.clone(), graphql_name });
            }
        }
    }

    let mut query = Object::new("Query");
    for root_field in &root_fields {
        query = query.field(build_root_field(root_field));
    }

    let mut builder = DynamicSchema::build("Query", None, None)
        .data(state.clone())
        .register(Scalar::new("JSON"))
        .register(query);

    for object_type in &object_types {
        builder = builder.register(build_object_type(object_type));
    }

    builder.finish().map_err(|err| err.to_string())
}

pub async fn graphql_get(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.is_empty() && request_prefers_html(&headers) {
        return Html(GraphiQLSource::build().endpoint("/graphql").finish()).into_response();
    }

    let request = match parse_query_string(raw_query) {
        Ok(request) if !request.query.trim().is_empty() => request,
        Ok(_) => return graphql_error_response(StatusCode::BAD_REQUEST, "Missing GraphQL query"),
        Err(err) => return graphql_error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    execute_graphql_request(&state, request).await
}

pub async fn graphql_post(
    State(state): State<AppState>,
    Json(request): Json<GraphqlRequest>,
) -> Response {
    execute_graphql_request(&state, request).await
}

fn build_collection_object_fields(
    resource: &str,
    table: &TableSchema,
    target_type_names: &BTreeMap<String, String>,
    resource_set: &BTreeSet<String>,
) -> Result<Vec<ObjectFieldSpec>, String> {
    let mut seen = BTreeMap::new();
    let mut fields = Vec::new();

    for (column_name, column) in &table.columns {
        let graphql_name = register_graphql_name(
            &mut seen,
            normalize_graphql_name(column_name),
            format!("column '{column_name}'"),
            &format!("GraphQL fields for resource '{resource}'"),
        )?;
        fields.push(ObjectFieldSpec {
            graphql_name,
            json_key: column_name.clone(),
            output: ObjectFieldOutput::Scalar(scalar_kind_from_column(column)),
            nullable: column.nullable,
        });
    }

    for (source_column, fk) in &table.foreign_keys {
        if !resource_set.contains(&fk.target_table) {
            continue;
        }
        let Some(target_type_name) = target_type_names.get(&fk.target_table).cloned() else {
            continue;
        };
        let relation_name = relation_field_name(source_column);
        let graphql_name = register_graphql_name(
            &mut seen,
            normalize_graphql_name(&relation_name),
            format!("relation field derived from column '{source_column}'"),
            &format!("GraphQL fields for resource '{resource}'"),
        )?;
        fields.push(ObjectFieldSpec {
            graphql_name,
            json_key: source_column.clone(),
            output: ObjectFieldOutput::Relation {
                source_column: source_column.clone(),
                target_type_name,
            },
            nullable: true,
        });
    }

    Ok(fields)
}

fn build_object_resource_fields(
    resource: &str,
    object: &JsonMap<String, JsonValue>,
) -> Result<Vec<ObjectFieldSpec>, String> {
    let mut seen = BTreeMap::new();
    let mut fields = Vec::new();

    for (json_key, value) in object {
        let graphql_name = register_graphql_name(
            &mut seen,
            normalize_graphql_name(json_key),
            format!("object key '{json_key}'"),
            &format!("GraphQL fields for object resource '{resource}'"),
        )?;
        fields.push(ObjectFieldSpec {
            graphql_name,
            json_key: json_key.clone(),
            output: ObjectFieldOutput::Scalar(scalar_kind_from_json_value(value)),
            nullable: true,
        });
    }

    Ok(fields)
}

fn build_object_type(spec: &ObjectTypeSpec) -> Object {
    let mut object = Object::new(spec.type_name.clone());
    for field in &spec.fields {
        object = object.field(build_object_field(&spec.source_resource, &spec.type_name, field));
    }
    object
}

fn build_object_field(
    source_resource: &str,
    object_type_name: &str,
    spec: &ObjectFieldSpec,
) -> Field {
    let type_ref = match &spec.output {
        ObjectFieldOutput::Scalar(kind) => scalar_type_ref(*kind, spec.nullable),
        ObjectFieldOutput::Relation { target_type_name, .. } => {
            named_type_ref(target_type_name, spec.nullable)
        }
    };
    let field_name = spec.graphql_name.clone();
    let json_key = spec.json_key.clone();
    let json_key_description = json_key.clone();

    match &spec.output {
        ObjectFieldOutput::Scalar(_) => Field::new(field_name, type_ref, move |ctx| {
            let json_key = json_key.clone();
            FieldFuture::new(async move {
                let parent = parent_object_value(&ctx)?;
                let Some(value) = parent.object.get(&json_key).cloned() else {
                    return Ok(FieldValue::NONE);
                };
                Ok(Some(FieldValue::value(json_to_graphql_value(value)?)))
            })
        }),
        ObjectFieldOutput::Relation { source_column, target_type_name } => {
            let source_resource = source_resource.to_string();
            let source_column = source_column.clone();
            let target_type_name = target_type_name.clone();
            Field::new(field_name, type_ref, move |ctx| {
                let state = ctx.data_unchecked::<AppState>().clone();
                let source_resource = source_resource.clone();
                let source_column = source_column.clone();
                let target_type_name = target_type_name.clone();
                FieldFuture::new(async move {
                    let parent = parent_object_value(&ctx)?;
                    let related = resolve_related_row(
                        &state,
                        &source_resource,
                        &parent.object,
                        &source_column,
                    )
                    .await
                    .map_err(app_error_to_graphql)?;
                    Ok(related.and_then(|value| {
                        value
                            .as_object()
                            .cloned()
                            .map(|object| typed_object_value(&target_type_name, object))
                    }))
                })
            })
        }
    }
    .description(format!("Field on {object_type_name} backed by JSON key '{json_key_description}'"))
}

fn build_root_field(spec: &RootFieldSpec) -> Field {
    match spec {
        RootFieldSpec::Collection { resource, graphql_name, row_type_name } => {
            build_collection_root_field(resource, graphql_name, row_type_name)
        }
        RootFieldSpec::CollectionById { resource, graphql_name, row_type_name, primary_key } => {
            build_collection_by_id_field(resource, graphql_name, row_type_name, primary_key)
        }
        RootFieldSpec::Object { resource, graphql_name, type_name } => {
            build_object_root_field(resource, graphql_name, type_name)
        }
        RootFieldSpec::Json { resource, graphql_name } => {
            build_json_root_field(resource, graphql_name)
        }
    }
}

fn build_collection_root_field(resource: &str, graphql_name: &str, row_type_name: &str) -> Field {
    let resource = resource.to_string();
    let row_type_name = row_type_name.to_string();
    Field::new(
        graphql_name.to_string(),
        TypeRef::named_nn_list_nn(row_type_name.clone()),
        move |ctx| {
            let state = ctx.data_unchecked::<AppState>().clone();
            let resource = resource.clone();
            let row_type_name = row_type_name.clone();
            FieldFuture::new(async move {
                let _guard = state.read_lock_for_resource(&resource).await;
                let data = load_resource(&state, &resource).await.map_err(app_error_to_graphql)?;
                validate_resource_data(&state, &resource, data.as_ref())
                    .map_err(app_error_to_graphql)?;
                let items = data.as_array().ok_or_else(|| {
                    GraphqlError::new(format!("Resource '{resource}' is not a JSON array"))
                })?;
                let values = items
                    .iter()
                    .map(|item| {
                        item.as_object()
                            .cloned()
                            .map(|object| typed_object_value(&row_type_name, object))
                            .ok_or_else(|| {
                                GraphqlError::new(format!(
                                    "Resource '{resource}' contains a non-object row"
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Some(FieldValue::list(values)))
            })
        },
    )
}

fn build_collection_by_id_field(
    resource: &str,
    graphql_name: &str,
    row_type_name: &str,
    primary_key: &str,
) -> Field {
    let resource = resource.to_string();
    let row_type_name = row_type_name.to_string();
    let primary_key = primary_key.to_string();
    Field::new(graphql_name.to_string(), TypeRef::named(row_type_name.clone()), move |ctx| {
        let state = ctx.data_unchecked::<AppState>().clone();
        let resource = resource.clone();
        let row_type_name = row_type_name.clone();
        let primary_key = primary_key.clone();
        FieldFuture::new(async move {
            let id = graphql_argument_to_lookup_string(ctx.args.try_get("id")?.as_value())?;
            let _guard = state.read_lock_for_resource(&resource).await;
            let data = load_resource(&state, &resource).await.map_err(app_error_to_graphql)?;
            validate_resource_data(&state, &resource, data.as_ref())
                .map_err(app_error_to_graphql)?;
            let items = data.as_array().ok_or_else(|| {
                GraphqlError::new(format!("Resource '{resource}' is not a JSON array"))
            })?;
            let related = find_item_by_key(items, &primary_key, &id)
                .and_then(|item| item.as_object().cloned())
                .map(|object| typed_object_value(&row_type_name, object));
            Ok(related)
        })
    })
    .argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)))
}

fn build_object_root_field(resource: &str, graphql_name: &str, type_name: &str) -> Field {
    let resource = resource.to_string();
    let type_name = type_name.to_string();
    Field::new(graphql_name.to_string(), TypeRef::named(type_name.clone()), move |ctx| {
        let state = ctx.data_unchecked::<AppState>().clone();
        let resource = resource.clone();
        let type_name = type_name.clone();
        FieldFuture::new(async move {
            let _guard = state.read_lock_for_resource(&resource).await;
            let data = load_resource(&state, &resource).await.map_err(app_error_to_graphql)?;
            let object = data.as_object().cloned().ok_or_else(|| {
                GraphqlError::new(format!("Resource '{resource}' is not a JSON object"))
            })?;
            Ok(Some(typed_object_value(&type_name, object)))
        })
    })
}

fn build_json_root_field(resource: &str, graphql_name: &str) -> Field {
    let resource = resource.to_string();
    Field::new(graphql_name.to_string(), TypeRef::named("JSON"), move |ctx| {
        let state = ctx.data_unchecked::<AppState>().clone();
        let resource = resource.clone();
        FieldFuture::new(async move {
            let _guard = state.read_lock_for_resource(&resource).await;
            let data = load_resource(&state, &resource).await.map_err(app_error_to_graphql)?;
            Ok(Some(FieldValue::value(json_to_graphql_value(data.as_ref().clone())?)))
        })
    })
}

fn parent_object_value<'a>(
    ctx: &'a async_graphql::dynamic::ResolverContext<'a>,
) -> Result<&'a GraphqlObjectValue, GraphqlError> {
    ctx.parent_value
        .try_downcast_ref::<GraphqlObjectValue>()
        .map_err(|err| GraphqlError::new(err.message))
}

fn typed_object_value(_type_name: &str, object: JsonMap<String, JsonValue>) -> FieldValue<'static> {
    FieldValue::owned_any(GraphqlObjectValue { object })
}

fn json_to_graphql_value(value: JsonValue) -> Result<GraphqlValue, GraphqlError> {
    GraphqlValue::from_json(value)
        .map_err(|err| GraphqlError::new(format!("Failed to convert JSON value: {err}")))
}

fn graphql_argument_to_lookup_string(value: &GraphqlValue) -> Result<String, GraphqlError> {
    match value {
        GraphqlValue::String(text) => Ok(text.clone()),
        GraphqlValue::Number(number) => Ok(number.to_string()),
        GraphqlValue::Boolean(value) => Ok(value.to_string()),
        _ => Err(GraphqlError::new("GraphQL id argument must be a scalar value")),
    }
}

fn app_error_to_graphql(error: AppError) -> GraphqlError {
    GraphqlError::new(error.message)
}

fn scalar_kind_from_column(column: &ColumnSchema) -> ScalarKind {
    match column.column_type {
        ColumnType::Integer => ScalarKind::Int,
        ColumnType::Float => ScalarKind::Float,
        ColumnType::Boolean => ScalarKind::Boolean,
        ColumnType::String => ScalarKind::String,
        ColumnType::Json => ScalarKind::Json,
    }
}

fn scalar_kind_from_json_value(value: &JsonValue) -> ScalarKind {
    if value.is_i64() || value.is_u64() {
        return ScalarKind::Int;
    }
    if value.is_number() {
        return ScalarKind::Float;
    }
    if value.is_boolean() {
        return ScalarKind::Boolean;
    }
    if value.is_string() {
        return ScalarKind::String;
    }
    ScalarKind::Json
}

fn scalar_type_ref(kind: ScalarKind, nullable: bool) -> TypeRef {
    let type_name = match kind {
        ScalarKind::Int => TypeRef::INT,
        ScalarKind::Float => TypeRef::FLOAT,
        ScalarKind::Boolean => TypeRef::BOOLEAN,
        ScalarKind::String => TypeRef::STRING,
        ScalarKind::Json => "JSON",
    };
    named_type_ref(type_name, nullable)
}

fn named_type_ref(type_name: &str, nullable: bool) -> TypeRef {
    if nullable { TypeRef::named(type_name) } else { TypeRef::named_nn(type_name) }
}

fn relation_field_name(source_column: &str) -> String {
    let mut candidate = source_column.to_string();
    for suffix in ["_id", "Id", "ID"] {
        if let Some(stripped) = candidate.strip_suffix(suffix) {
            candidate = stripped.to_string();
            break;
        }
    }
    for suffix in ["_ref", "Ref"] {
        if let Some(stripped) = candidate.strip_suffix(suffix) {
            candidate = stripped.to_string();
            break;
        }
    }
    if candidate.is_empty() || candidate == source_column {
        return format!("{source_column}Ref");
    }
    candidate
}

fn normalize_graphql_name(raw: &str) -> String {
    let mut normalized = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' })
        .collect::<String>();

    if normalized.is_empty() {
        normalized.push('x');
    }
    if normalized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        normalized = format!("n_{normalized}");
    }
    if normalized.starts_with("__") {
        normalized = format!("x_{normalized}");
    }
    normalized
}

fn collection_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Record", pascalize(resource)))
}

fn object_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Object", pascalize(resource)))
}

fn normalize_graphql_type_name(raw: &str) -> String {
    let mut normalized = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' { ch } else { '_' })
        .collect::<String>();

    if normalized.is_empty() {
        normalized.push('X');
    }
    if normalized.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        normalized = format!("N{normalized}");
    }
    if normalized.starts_with("__") {
        normalized = format!("X{normalized}");
    }
    normalized
}

fn pascalize(raw: &str) -> String {
    let mut out = String::new();
    let mut uppercase_next = true;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if uppercase_next {
                out.push(ch.to_ascii_uppercase());
                uppercase_next = false;
            } else {
                out.push(ch);
            }
        } else {
            uppercase_next = true;
        }
    }

    if out.is_empty() {
        out.push('X');
    }
    out
}

fn register_graphql_name(
    seen: &mut BTreeMap<String, String>,
    normalized: String,
    origin: String,
    scope: &str,
) -> Result<String, String> {
    if let Some(existing) = seen.get(&normalized) {
        return Err(format!(
            "{scope}: GraphQL name '{normalized}' conflicts between {existing} and {origin}"
        ));
    }
    seen.insert(normalized.clone(), origin);
    Ok(normalized)
}

fn request_prefers_html(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept.split(',').map(str::trim).any(|value| {
                value.starts_with("text/html") || value.starts_with("application/xhtml+xml")
            })
        })
        .unwrap_or(false)
}

async fn execute_graphql_request(state: &AppState, request: GraphqlRequest) -> Response {
    let schema = match state.graphql_schema().await {
        Ok(schema) => schema,
        Err(error) => {
            return graphql_error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    };

    GraphQLResponse::from(schema.execute(request).await).into_response()
}

fn graphql_error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/graphql-response+json")],
        Json(json!({
            "errors": [{"message": message.into()}]
        })),
    )
        .into_response()
}
