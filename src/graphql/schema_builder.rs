use std::collections::{BTreeMap, BTreeSet};

use async_graphql::dynamic::{Object, Scalar, Schema as DynamicSchema};
use serde_json::Value as JsonValue;

use crate::{app::AppState, schema::primary_key_name, storage::load_resource};

use super::{
    fields::{
        build_collection_object_fields, build_filter_input, build_filter_operator_enum,
        build_object_resource_fields, build_object_type, build_page_type, build_root_field,
        build_sort_direction_enum, build_sort_input,
    },
    naming::{
        collection_page_type_name, collection_type_name, normalize_graphql_name, object_type_name,
        register_graphql_name,
    },
    types::{LoadedResourceSchema, ObjectTypeSpec, PageTypeSpec, RootFieldSpec},
};

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
