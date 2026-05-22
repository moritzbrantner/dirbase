use std::collections::{BTreeMap, BTreeSet};

use async_graphql::{
    Error as GraphqlError,
    dynamic::{
        Enum, EnumItem, Field, FieldFuture, FieldValue, InputObject, InputValue, Object, TypeRef,
    },
};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{
    app::AppState,
    query::filters::{
        FilterCondition, FilterOperator, Pagination, SortColumn, filter_collection_refs,
        paginate_collection_refs, sort_collection_refs,
    },
    schema::{DeclaredTableSchema, TableSchema},
    storage::{find_item_by_key, load_resource, validate_resource_data},
};

use super::{
    naming::{normalize_graphql_name, register_graphql_name, relation_field_name},
    resolvers::{resolve_graphql_many_to_many_rows, resolve_graphql_related_row},
    types::{
        GraphqlCollectionArgs, GraphqlPageValue, GraphqlRelationCache, ObjectFieldOutput,
        ObjectFieldSpec, ObjectTypeSpec, PageTypeSpec, RootFieldSpec,
    },
    values::{
        app_error_to_graphql, graphql_argument_to_lookup_string, json_to_graphql_value,
        named_type_ref, parent_object_value, parent_page_value, scalar_kind_from_column,
        scalar_kind_from_json_value, scalar_type_ref, typed_object_value,
    },
};

pub(crate) fn build_collection_object_fields(
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

pub(crate) fn build_object_resource_fields(
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

pub(crate) fn build_object_type(spec: &ObjectTypeSpec) -> Object {
    let mut object = Object::new(spec.type_name.clone());
    for field in &spec.fields {
        object = object.field(build_object_field(&spec.source_resource, &spec.type_name, field));
    }
    object
}

pub(crate) fn build_object_field(
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

pub(crate) fn build_root_field(spec: &RootFieldSpec) -> Field {
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

pub(crate) fn build_filter_operator_enum() -> Enum {
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

pub(crate) fn build_sort_direction_enum() -> Enum {
    Enum::new("CollectionSortDirection").item(EnumItem::new("ASC")).item(EnumItem::new("DESC"))
}

pub(crate) fn build_filter_input(filter_operator_type: &str) -> InputObject {
    InputObject::new("CollectionFilterInput")
        .field(InputValue::new("field", TypeRef::named_nn(TypeRef::STRING)))
        .field(InputValue::new("operator", TypeRef::named(filter_operator_type)))
        .field(InputValue::new("value", TypeRef::named(TypeRef::STRING)))
}

pub(crate) fn build_sort_input(sort_direction_type: &str) -> InputObject {
    InputObject::new("CollectionSortInput")
        .field(InputValue::new("field", TypeRef::named_nn(TypeRef::STRING)))
        .field(InputValue::new("direction", TypeRef::named(sort_direction_type)))
}

pub(crate) fn build_page_type(spec: &PageTypeSpec) -> Object {
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

pub(crate) fn build_collection_root_field(
    resource: &str,
    graphql_name: &str,
    row_type_name: &str,
) -> Field {
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

pub(crate) fn build_collection_by_id_field(
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

pub(crate) fn build_collection_query_field(
    resource: &str,
    graphql_name: &str,
    page_type_name: &str,
) -> Field {
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

pub(crate) fn build_object_root_field(
    resource: &str,
    graphql_name: &str,
    type_name: &str,
) -> Field {
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

pub(crate) fn parse_collection_query_arguments(
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

pub(crate) fn parse_filter_operator_enum(value: &str) -> Result<FilterOperator, GraphqlError> {
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

pub(crate) fn build_json_root_field(resource: &str, graphql_name: &str) -> Field {
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
