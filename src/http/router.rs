use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{MethodRouter, delete, get, post},
};

use crate::{
    app::AppState,
    graphql::{graphql_get, graphql_post},
    http::{
        assets::{get_overview_css, get_overview_js},
        html_forms::{get_create_item_form, get_item_editor, get_resource_editor},
        middleware::{
            auth_middleware, cors_middleware, log_requests_middleware, metrics_middleware,
            security_headers_middleware,
        },
        ops::{get_events, healthz, metrics, readyz},
        resource_routes::{
            create_item, create_resource, delete_item, delete_resource, get_collection, get_item,
            get_overview, list_resources, patch_item, patch_resource_object, replace_item,
            replace_resource_object,
        },
        response_format::response_format_middleware,
        schema_routes::{
            get_schema, get_schema_editor, infer_and_save_schema, save_declared_schema, save_schema,
        },
    },
    openapi::get_openapi,
    sql::{export_sql, sql_query, sql_query_post},
};

pub fn build_router(state: AppState) -> Router {
    let app = build_application_routes(state.config.readonly).with_state(state.clone());

    let mut app = app.layer(DefaultBodyLimit::max(state.config.max_body_bytes));
    app = app.layer(middleware::from_fn_with_state(state.clone(), metrics_middleware));
    app = app.layer(middleware::from_fn_with_state(state.clone(), cors_middleware));
    app = app.layer(middleware::from_fn_with_state(state.clone(), auth_middleware));
    app = app.layer(middleware::from_fn_with_state(state.clone(), response_format_middleware));
    app = app.layer(middleware::from_fn(security_headers_middleware));
    if state.config.enable_log {
        app = app.layer(middleware::from_fn_with_state(state.clone(), log_requests_middleware));
    }
    app
}

fn build_application_routes(readonly: bool) -> Router<AppState> {
    let app = Router::new()
        .route("/", get(list_resources))
        .route("/events", get(get_events))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/resources", resources_route(readonly))
        .route("/overview.json", get(get_overview))
        .route("/openapi.json", get(get_openapi))
        .route("/assets/overview.css", get(get_overview_css))
        .route("/assets/overview.js", get(get_overview_js))
        .route("/graphql", get(graphql_get).post(graphql_post))
        .route("/schema", schema_route(readonly))
        .route("/schema/editor", get(get_schema_editor))
        .route("/sql", sql_route(readonly))
        .route("/export.sql", get(export_sql))
        .route("/sql/export", get(export_sql))
        .route("/{resource}/edit", get(get_resource_editor))
        .route("/{resource}/create", get(get_create_item_form))
        .route("/{resource}/{id}/edit", get(get_item_editor))
        .route("/{resource}", collection_route(readonly))
        .route("/{resource}/{id}", item_route(readonly));

    if readonly {
        app
    } else {
        app.route("/schema/infer", post(infer_and_save_schema))
            .route("/resources/{resource}", delete(delete_resource))
    }
}

fn resources_route(readonly: bool) -> MethodRouter<AppState> {
    let route = get(list_resources);
    if readonly { route } else { route.post(create_resource) }
}

fn schema_route(readonly: bool) -> MethodRouter<AppState> {
    let route = get(get_schema);
    if readonly { route } else { route.post(save_schema).put(save_declared_schema) }
}

fn sql_route(readonly: bool) -> MethodRouter<AppState> {
    let route = get(sql_query);
    if readonly { route } else { route.post(sql_query_post) }
}

fn collection_route(readonly: bool) -> MethodRouter<AppState> {
    let route = get(get_collection);
    if readonly {
        route
    } else {
        route.post(create_item).put(replace_resource_object).patch(patch_resource_object)
    }
}

fn item_route(readonly: bool) -> MethodRouter<AppState> {
    let route = get(get_item);
    if readonly { route } else { route.put(replace_item).patch(patch_item).delete(delete_item) }
}
