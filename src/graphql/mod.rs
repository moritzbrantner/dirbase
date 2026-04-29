use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_graphql::{
    Error as GraphqlError, Request as GraphqlRequest, Value as GraphqlValue,
    dynamic::{
        Enum, EnumItem, Field, FieldFuture, FieldValue, InputObject, InputValue, Object, Scalar,
        Schema as DynamicSchema, TypeRef,
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
    query::filters::{
        FilterCondition, FilterOperator, Pagination, SortColumn, filter_collection_refs,
        paginate_collection_refs, sort_collection_refs, value_to_filter_string,
    },
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    schema::{
        ColumnSchema, ColumnType, DeclaredTableSchema, ManyToManyRelation, TableSchema,
        primary_key_name,
    },
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
    ManyToManyList { relation: ManyToManyRelation, target_type_name: String },
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
    CollectionQuery {
        resource: String,
        graphql_name: String,
        page_type_name: String,
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

#[derive(Clone, Debug)]
struct PageTypeSpec {
    type_name: String,
    row_type_name: String,
}

#[derive(Clone, Debug)]
struct GraphqlPageValue {
    object: JsonMap<String, JsonValue>,
}

#[derive(Default)]
struct GraphqlRelationCache {
    resources: tokio::sync::Mutex<BTreeMap<String, Arc<JsonValue>>>,
}

#[derive(Clone, Debug)]
struct LoadedResourceSchema {
    resource: String,
    value: JsonValue,
    table: Option<TableSchema>,
    declared_table: Option<DeclaredTableSchema>,
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
    let mut page_type_names = BTreeMap::new();
    let mut object_type_names = BTreeMap::new();
    let mut loaded_resources = Vec::with_capacity(resources.len());

    for resource in &resources {
        let value = load_resource(state, resource).await.map_err(|err| {
            format!("GraphQL schema build failed for resource '{resource}': {}", err.message)
        })?;
        let value = value.as_ref().clone();
        let table = schema.tables.get(resource).cloned();
        let declared_table = state.validation_schema_table(resource);

        if matches!(&value, JsonValue::Array(_))
            && table.as_ref().is_some_and(|table| !table.columns.is_empty())
        {
            let type_name = register_graphql_name(
                &mut type_name_registry,
                collection_type_name(resource),
                format!("collection type for resource '{resource}'"),
                "GraphQL type names",
            )?;
            let page_type_name = register_graphql_name(
                &mut type_name_registry,
                collection_page_type_name(resource),
                format!("collection page type for resource '{resource}'"),
                "GraphQL type names",
            )?;
            collection_type_names.insert(resource.clone(), type_name);
            page_type_names.insert(resource.clone(), page_type_name);
        } else if let JsonValue::Object(object) = &value
            && !object.is_empty()
        {
            let type_name = register_graphql_name(
                &mut type_name_registry,
                object_type_name(resource),
                format!("object type for resource '{resource}'"),
                "GraphQL type names",
            )?;
            object_type_names.insert(resource.clone(), type_name);
        }

        loaded_resources.push(LoadedResourceSchema {
            resource: resource.clone(),
            value,
            table,
            declared_table,
        });
    }

    let mut object_types = Vec::new();
    let mut page_types = Vec::new();
    let mut root_fields = Vec::new();

    for loaded in &loaded_resources {
        if let JsonValue::Array(_) = &loaded.value
            && let Some(table) = loaded.table.as_ref()
            && let Some(row_type_name) = collection_type_names.get(&loaded.resource).cloned()
        {
            let fields = build_collection_object_fields(
                &loaded.resource,
                table,
                &collection_type_names,
                &resource_set,
            )?;
            if !fields.is_empty() {
                let collection_field_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(&loaded.resource),
                    format!("resource '{}'", loaded.resource),
                    "GraphQL root fields",
                )?;
                root_fields.push(RootFieldSpec::Collection {
                    resource: loaded.resource.clone(),
                    graphql_name: collection_field_name,
                    row_type_name: row_type_name.clone(),
                });
                let query_field_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(&format!("{}Query", loaded.resource)),
                    format!("query field for resource '{}'", loaded.resource),
                    "GraphQL root fields",
                )?;
                let page_type_name =
                    page_type_names.get(&loaded.resource).cloned().ok_or_else(|| {
                        format!("Missing page type for resource '{}'", loaded.resource)
                    })?;
                root_fields.push(RootFieldSpec::CollectionQuery {
                    resource: loaded.resource.clone(),
                    graphql_name: query_field_name,
                    page_type_name: page_type_name.clone(),
                });

                if table.primary_key.is_some() {
                    let by_id_name = register_graphql_name(
                        &mut root_field_registry,
                        normalize_graphql_name(&format!("{}ById", loaded.resource)),
                        format!("single-item field for resource '{}'", loaded.resource),
                        "GraphQL root fields",
                    )?;
                    root_fields.push(RootFieldSpec::CollectionById {
                        resource: loaded.resource.clone(),
                        graphql_name: by_id_name,
                        row_type_name: row_type_name.clone(),
                        primary_key: primary_key_name(Some(table)).to_string(),
                    });
                }

                object_types.push(ObjectTypeSpec {
                    source_resource: loaded.resource.clone(),
                    type_name: row_type_name,
                    fields,
                });
                page_types.push(PageTypeSpec {
                    type_name: page_type_name,
                    row_type_name: collection_type_names
                        .get(&loaded.resource)
                        .cloned()
                        .expect("row type"),
                });
                continue;
            }
        }

        match &loaded.value {
            JsonValue::Object(object) if !object.is_empty() => {
                let type_name =
                    object_type_names.get(&loaded.resource).cloned().ok_or_else(|| {
                        format!("Missing object type for resource '{}'", loaded.resource)
                    })?;
                let fields = build_object_resource_fields(
                    &loaded.resource,
                    object,
                    loaded.declared_table.as_ref(),
                )?;
                if fields.is_empty() {
                    let graphql_name = register_graphql_name(
                        &mut root_field_registry,
                        normalize_graphql_name(&loaded.resource),
                        format!("resource '{}'", loaded.resource),
                        "GraphQL root fields",
                    )?;
                    root_fields.push(RootFieldSpec::Json {
                        resource: loaded.resource.clone(),
                        graphql_name,
                    });
                    continue;
                }

                let graphql_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(&loaded.resource),
                    format!("resource '{}'", loaded.resource),
                    "GraphQL root fields",
                )?;
                root_fields.push(RootFieldSpec::Object {
                    resource: loaded.resource.clone(),
                    graphql_name,
                    type_name: type_name.clone(),
                });
                object_types.push(ObjectTypeSpec {
                    source_resource: loaded.resource.clone(),
                    type_name,
                    fields,
                });
            }
            _ => {
                let graphql_name = register_graphql_name(
                    &mut root_field_registry,
                    normalize_graphql_name(&loaded.resource),
                    format!("resource '{}'", loaded.resource),
                    "GraphQL root fields",
                )?;
                root_fields
                    .push(RootFieldSpec::Json { resource: loaded.resource.clone(), graphql_name });
            }
        }
    }

    let mut query = Object::new("Query");
    for root_field in &root_fields {
        query = query.field(build_root_field(root_field));
    }

    let filter_operator = build_filter_operator_enum();
    let sort_direction = build_sort_direction_enum();
    let filter_input = build_filter_input(filter_operator.type_name());
    let sort_input = build_sort_input(sort_direction.type_name());

    let mut builder = DynamicSchema::build("Query", None, None)
        .data(state.clone())
        .register(Scalar::new("JSON"))
        .register(filter_operator)
        .register(sort_direction)
        .register(filter_input)
        .register(sort_input)
        .register(query);

    for object_type in &object_types {
        builder = builder.register(build_object_type(object_type));
    }
    for page_type in &page_types {
        builder = builder.register(build_page_type(page_type));
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

    for (relation_name, relation) in &table.many_to_many {
        if !resource_set.contains(&relation.target_table) {
            continue;
        }
        let Some(target_type_name) = target_type_names.get(&relation.target_table).cloned() else {
            continue;
        };
        let scope = format!("GraphQL fields for resource '{resource}'");
        let fallback_name = format!("{}_via_{}", relation.target_table, relation.through_table);
        let graphql_name = register_graphql_name(
            &mut seen,
            normalize_graphql_name(relation_name),
            format!("many-to-many field '{relation_name}'"),
            &scope,
        )
        .or_else(|_| {
            register_graphql_name(
                &mut seen,
                normalize_graphql_name(&fallback_name),
                format!("many-to-many field '{fallback_name}'"),
                &scope,
            )
        })?;
        fields.push(ObjectFieldSpec {
            graphql_name,
            json_key: relation_name.clone(),
            output: ObjectFieldOutput::ManyToManyList {
                relation: relation.clone(),
                target_type_name,
            },
            nullable: false,
        });
    }

    Ok(fields)
}

fn build_object_resource_fields(
    resource: &str,
    object: &JsonMap<String, JsonValue>,
    declared_table: Option<&DeclaredTableSchema>,
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
            output: ObjectFieldOutput::Scalar(
                declared_table
                    .and_then(|table| table.columns.get(json_key))
                    .map_or_else(|| scalar_kind_from_json_value(value), scalar_kind_from_column),
            ),
            nullable: declared_table
                .and_then(|table| table.columns.get(json_key))
                .is_none_or(|column| column.nullable),
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
        ObjectFieldOutput::ManyToManyList { target_type_name, .. } => {
            TypeRef::named_nn_list_nn(target_type_name.clone())
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
                let cache = ctx.data_unchecked::<GraphqlRelationCache>();
                let source_resource = source_resource.clone();
                let source_column = source_column.clone();
                let target_type_name = target_type_name.clone();
                FieldFuture::new(async move {
                    let parent = parent_object_value(&ctx)?;
                    let related = resolve_graphql_related_row(
                        &state,
                        cache,
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
        ObjectFieldOutput::ManyToManyList { relation, target_type_name } => {
            let relation = relation.clone();
            let target_type_name = target_type_name.clone();
            Field::new(field_name, type_ref, move |ctx| {
                let state = ctx.data_unchecked::<AppState>().clone();
                let cache = ctx.data_unchecked::<GraphqlRelationCache>();
                let relation = relation.clone();
                let target_type_name = target_type_name.clone();
                FieldFuture::new(async move {
                    let parent = parent_object_value(&ctx)?;
                    let values = resolve_graphql_many_to_many_rows(
                        &state,
                        cache,
                        &parent.object,
                        &relation,
                        &target_type_name,
                    )
                    .await?;
                    Ok(Some(FieldValue::list(values)))
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
        RootFieldSpec::CollectionQuery { resource, graphql_name, page_type_name } => {
            build_collection_query_field(resource, graphql_name, page_type_name)
        }
        RootFieldSpec::Object { resource, graphql_name, type_name } => {
            build_object_root_field(resource, graphql_name, type_name)
        }
        RootFieldSpec::Json { resource, graphql_name } => {
            build_json_root_field(resource, graphql_name)
        }
    }
}

fn build_filter_operator_enum() -> Enum {
    Enum::new("CollectionFilterOperator")
        .item(EnumItem::new("EQ"))
        .item(EnumItem::new("NE"))
        .item(EnumItem::new("LT"))
        .item(EnumItem::new("LTE"))
        .item(EnumItem::new("GT"))
        .item(EnumItem::new("GTE"))
        .item(EnumItem::new("IN"))
        .item(EnumItem::new("CONTAINS"))
        .item(EnumItem::new("STARTS_WITH"))
        .item(EnumItem::new("ENDS_WITH"))
        .item(EnumItem::new("IS_NULL"))
        .item(EnumItem::new("IS_NOT_NULL"))
}

fn build_sort_direction_enum() -> Enum {
    Enum::new("CollectionSortDirection").item(EnumItem::new("ASC")).item(EnumItem::new("DESC"))
}

fn build_filter_input(filter_operator_type: &str) -> InputObject {
    InputObject::new("CollectionFilterInput")
        .field(InputValue::new("field", TypeRef::named_nn(TypeRef::STRING)))
        .field(InputValue::new("operator", TypeRef::named(filter_operator_type)))
        .field(InputValue::new("value", TypeRef::named(TypeRef::STRING)))
}

fn build_sort_input(sort_direction_type: &str) -> InputObject {
    InputObject::new("CollectionSortInput")
        .field(InputValue::new("field", TypeRef::named_nn(TypeRef::STRING)))
        .field(InputValue::new("direction", TypeRef::named(sort_direction_type)))
}

fn build_page_type(spec: &PageTypeSpec) -> Object {
    let mut object = Object::new(spec.type_name.clone());
    for field_name in ["first", "prev", "next", "last", "page", "pages", "items"] {
        let field = field_name.to_string();
        object =
            object.field(Field::new(field.clone(), TypeRef::named_nn(TypeRef::INT), move |ctx| {
                let field = field.clone();
                FieldFuture::new(async move {
                    let parent = parent_page_value(&ctx)?;
                    let value = parent.object.get(&field).cloned().ok_or_else(|| {
                        GraphqlError::new(format!("Missing page field '{field}'"))
                    })?;
                    Ok(Some(FieldValue::value(json_to_graphql_value(value)?)))
                })
            }));
    }
    let row_type_name = spec.row_type_name.clone();
    object.field(Field::new(
        "data",
        TypeRef::named_nn_list_nn(spec.row_type_name.clone()),
        move |ctx| {
            let row_type_name = row_type_name.clone();
            FieldFuture::new(async move {
                let parent = parent_page_value(&ctx)?;
                let items = parent
                    .object
                    .get("data")
                    .and_then(JsonValue::as_array)
                    .ok_or_else(|| GraphqlError::new("Missing page field 'data'"))?;
                let values = items
                    .iter()
                    .map(|item| {
                        item.as_object()
                            .cloned()
                            .map(|object| typed_object_value(&row_type_name, object))
                            .ok_or_else(|| GraphqlError::new("Page data contains a non-object row"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Some(FieldValue::list(values)))
            })
        },
    ))
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

fn build_collection_query_field(resource: &str, graphql_name: &str, page_type_name: &str) -> Field {
    let resource = resource.to_string();
    Field::new(graphql_name.to_string(), TypeRef::named_nn(page_type_name), move |ctx| {
        let state = ctx.data_unchecked::<AppState>().clone();
        let resource = resource.clone();
        FieldFuture::new(async move {
            let args = parse_collection_query_arguments(&ctx)?;
            let _guard = state.read_lock_for_resource(&resource).await;
            let data = load_resource(&state, &resource).await.map_err(app_error_to_graphql)?;
            validate_resource_data(&state, &resource, data.as_ref())
                .map_err(app_error_to_graphql)?;
            let table = state.schema_table(&resource);
            let items = data.as_array().ok_or_else(|| {
                GraphqlError::new(format!("Resource '{resource}' is not a JSON array"))
            })?;
            let mut selected = if args.filters.is_empty() {
                items.iter().collect::<Vec<_>>()
            } else {
                filter_collection_refs(items, &args.filters, table.as_ref())
            };
            if !args.sort_columns.is_empty() {
                sort_collection_refs(selected.as_mut_slice(), &args.sort_columns);
            }
            let pagination =
                args.pagination.unwrap_or(Pagination { page: 1, per_page: selected.len().max(1) });
            if pagination.per_page > state.config.max_per_page {
                return Err(GraphqlError::new(format!(
                    "perPage exceeds configured max of {}",
                    state.config.max_per_page
                )));
            }
            let page = paginate_collection_refs(&selected, pagination);
            let object = page
                .as_object()
                .cloned()
                .ok_or_else(|| GraphqlError::new("Invalid paginated result"))?;
            Ok(Some(FieldValue::owned_any(GraphqlPageValue { object })))
        })
    })
    .argument(InputValue::new("filter", TypeRef::named_list("CollectionFilterInput")))
    .argument(InputValue::new("sort", TypeRef::named_list("CollectionSortInput")))
    .argument(InputValue::new("page", TypeRef::named(TypeRef::INT)))
    .argument(InputValue::new("perPage", TypeRef::named(TypeRef::INT)))
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
            validate_resource_data(&state, &resource, data.as_ref())
                .map_err(app_error_to_graphql)?;
            let object = data.as_object().cloned().ok_or_else(|| {
                GraphqlError::new(format!("Resource '{resource}' is not a JSON object"))
            })?;
            Ok(Some(typed_object_value(&type_name, object)))
        })
    })
}

#[derive(Debug)]
struct GraphqlCollectionArgs {
    filters: Vec<FilterCondition>,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
}

fn parse_collection_query_arguments(
    ctx: &async_graphql::dynamic::ResolverContext<'_>,
) -> Result<GraphqlCollectionArgs, GraphqlError> {
    let mut filters = Vec::new();
    if let Some(filter_arg) = ctx.args.get("filter") {
        for item in filter_arg.list()?.iter() {
            let filter = item.object()?;
            let field = filter.try_get("field")?.string()?.to_string();
            let operator = filter
                .get("operator")
                .map(|value| parse_filter_operator_enum(value.enum_name()?))
                .transpose()?
                .unwrap_or(FilterOperator::Eq);
            let value = filter
                .get("value")
                .map(|value| value.string().map(str::to_string))
                .transpose()?
                .unwrap_or_default();
            filters.push(FilterCondition::new(field, operator, value));
        }
    }

    let mut sort_columns = Vec::new();
    if let Some(sort_arg) = ctx.args.get("sort") {
        for item in sort_arg.list()?.iter() {
            let sort = item.object()?;
            let field = sort.try_get("field")?.string()?.to_string();
            let descending = sort
                .get("direction")
                .map(|value| value.enum_name().map(|name| name == "DESC"))
                .transpose()?
                .unwrap_or(false);
            sort_columns.push(SortColumn { field_path: field, descending });
        }
    }

    let page = ctx.args.get("page").map(|value| value.i64()).transpose()?;
    let per_page = ctx.args.get("perPage").map(|value| value.i64()).transpose()?;
    let pagination = match (page, per_page) {
        (None, None) => None,
        (Some(page), Some(per_page)) => {
            Some(Pagination { page: page.max(1) as usize, per_page: per_page.max(1) as usize })
        }
        (Some(page), None) => Some(Pagination { page: page.max(1) as usize, per_page: 10 }),
        (None, Some(per_page)) => Some(Pagination { page: 1, per_page: per_page.max(1) as usize }),
    };

    Ok(GraphqlCollectionArgs { filters, sort_columns, pagination })
}

fn parse_filter_operator_enum(value: &str) -> Result<FilterOperator, GraphqlError> {
    match value {
        "EQ" => Ok(FilterOperator::Eq),
        "NE" => Ok(FilterOperator::Ne),
        "LT" => Ok(FilterOperator::Lt),
        "LTE" => Ok(FilterOperator::Lte),
        "GT" => Ok(FilterOperator::Gt),
        "GTE" => Ok(FilterOperator::Gte),
        "IN" => Ok(FilterOperator::In),
        "CONTAINS" => Ok(FilterOperator::Contains),
        "STARTS_WITH" => Ok(FilterOperator::StartsWith),
        "ENDS_WITH" => Ok(FilterOperator::EndsWith),
        "IS_NULL" => Ok(FilterOperator::IsNull),
        "IS_NOT_NULL" => Ok(FilterOperator::IsNotNull),
        _ => Err(GraphqlError::new(format!("Unsupported filter operator '{value}'"))),
    }
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

fn parent_page_value<'a>(
    ctx: &'a async_graphql::dynamic::ResolverContext<'a>,
) -> Result<&'a GraphqlPageValue, GraphqlError> {
    ctx.parent_value
        .try_downcast_ref::<GraphqlPageValue>()
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
        ColumnType::String
        | ColumnType::Date
        | ColumnType::DateTime
        | ColumnType::Uuid
        | ColumnType::BigInteger
        | ColumnType::Decimal => ScalarKind::String,
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

fn collection_page_type_name(resource: &str) -> String {
    normalize_graphql_type_name(&format!("{}Page", pascalize(resource)))
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

    let request = request.data(GraphqlRelationCache::default());
    GraphQLResponse::from(schema.execute(request).await).into_response()
}

async fn resolve_graphql_related_row(
    state: &AppState,
    cache: &GraphqlRelationCache,
    resource: &str,
    source_object: &JsonMap<String, JsonValue>,
    source_column: &str,
) -> Result<Option<JsonValue>, AppError> {
    let table = state.schema_table(resource).ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "Relation lookup requires schema metadata with foreign key definitions",
        )
    })?;
    let fk = table.foreign_keys.get(source_column).ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Cannot resolve relation '{source_column}' for resource '{resource}'"),
        )
    })?;
    let target_resource = load_cached_graphql_resource(cache, state, &fk.target_table).await?;
    let target_items = target_resource.as_array().ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Embedded resource '{}' is not a JSON array", fk.target_table),
        )
    })?;
    let lookup = build_relation_lookup(target_items, &fk.target_column);
    Ok(resolve_related_row_in_lookup(source_object, source_column, &lookup))
}

async fn load_cached_graphql_resource(
    cache: &GraphqlRelationCache,
    state: &AppState,
    resource: &str,
) -> Result<Arc<JsonValue>, AppError> {
    if let Some(value) = cache.resources.lock().await.get(resource).cloned() {
        return Ok(value);
    }

    let value = load_resource(state, resource).await?;
    cache.resources.lock().await.insert(resource.to_string(), value.clone());
    Ok(value)
}

async fn resolve_graphql_many_to_many_rows(
    state: &AppState,
    cache: &GraphqlRelationCache,
    parent: &JsonMap<String, JsonValue>,
    relation: &ManyToManyRelation,
    target_type_name: &str,
) -> Result<Vec<FieldValue<'static>>, GraphqlError> {
    let source_value = match parent.get(&relation.source_target_column) {
        Some(value) if value.is_null() || value.is_object() || value.is_array() => {
            return Ok(Vec::new());
        }
        Some(value) => value_to_filter_string(value),
        None => return Ok(Vec::new()),
    };

    let through_resource = load_cached_graphql_resource(cache, state, &relation.through_table)
        .await
        .map_err(app_error_to_graphql)?;
    validate_resource_data(state, &relation.through_table, through_resource.as_ref())
        .map_err(app_error_to_graphql)?;
    let through_items = through_resource.as_array().ok_or_else(|| {
        GraphqlError::new(format!("Resource '{}' is not a JSON array", relation.through_table))
    })?;

    let mut target_ids = BTreeSet::new();
    for row in through_items {
        let Some(object) = row.as_object() else {
            continue;
        };
        let Some(candidate) = object.get(&relation.source_column) else {
            continue;
        };
        if candidate.is_null() || candidate.is_object() || candidate.is_array() {
            continue;
        }
        if value_to_filter_string(candidate) != source_value {
            continue;
        }
        let Some(target_value) = object.get(&relation.through_target_column) else {
            continue;
        };
        if target_value.is_null() || target_value.is_object() || target_value.is_array() {
            continue;
        }
        target_ids.insert(value_to_filter_string(target_value));
    }

    if target_ids.is_empty() {
        return Ok(Vec::new());
    }

    let target_resource = load_cached_graphql_resource(cache, state, &relation.target_table)
        .await
        .map_err(app_error_to_graphql)?;
    validate_resource_data(state, &relation.target_table, target_resource.as_ref())
        .map_err(app_error_to_graphql)?;
    let target_items = target_resource.as_array().ok_or_else(|| {
        GraphqlError::new(format!("Resource '{}' is not a JSON array", relation.target_table))
    })?;

    let values = target_items
        .iter()
        .filter_map(|item| item.as_object())
        .filter_map(|object| {
            let id = object.get(&relation.target_column)?;
            (!id.is_null()
                && !id.is_object()
                && !id.is_array()
                && target_ids.contains(&value_to_filter_string(id)))
            .then(|| typed_object_value(target_type_name, object.clone()))
        })
        .collect::<Vec<_>>();

    Ok(values)
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
