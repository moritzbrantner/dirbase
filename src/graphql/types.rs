use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{
    query::filters::{FilterCondition, Pagination, SortColumn},
    schema::{DeclaredTableSchema, ManyToManyRelation, TableSchema},
};

#[derive(Clone, Debug)]
pub(crate) struct ObjectTypeSpec {
    pub(crate) source_resource: String,
    pub(crate) type_name: String,
    pub(crate) fields: Vec<ObjectFieldSpec>,
}

#[derive(Clone, Debug)]
pub(crate) struct ObjectFieldSpec {
    pub(crate) graphql_name: String,
    pub(crate) json_key: String,
    pub(crate) output: ObjectFieldOutput,
    pub(crate) nullable: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum ObjectFieldOutput {
    Scalar(ScalarKind),
    Relation { source_column: String, target_type_name: String },
    ManyToManyList { relation: ManyToManyRelation, target_type_name: String },
}

#[derive(Clone, Debug)]
pub(crate) enum RootFieldSpec {
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
pub(crate) struct GraphqlObjectValue {
    pub(crate) object: JsonMap<String, JsonValue>,
}

#[derive(Clone, Debug)]
pub(crate) struct PageTypeSpec {
    pub(crate) type_name: String,
    pub(crate) row_type_name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct GraphqlPageValue {
    pub(crate) object: JsonMap<String, JsonValue>,
}

#[derive(Default)]
pub(crate) struct GraphqlRelationCache {
    pub(crate) resources: tokio::sync::Mutex<BTreeMap<String, Arc<JsonValue>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedResourceSchema {
    pub(crate) resource: String,
    pub(crate) value: JsonValue,
    pub(crate) table: Option<TableSchema>,
    pub(crate) declared_table: Option<DeclaredTableSchema>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ScalarKind {
    Int,
    Float,
    Boolean,
    String,
    Json,
}
#[derive(Debug)]
pub(crate) struct GraphqlCollectionArgs {
    pub(crate) filters: Vec<FilterCondition>,
    pub(crate) sort_columns: Vec<SortColumn>,
    pub(crate) pagination: Option<Pagination>,
}
