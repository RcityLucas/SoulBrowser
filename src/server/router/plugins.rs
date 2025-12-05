use axum::{routing::get, routing::post, routing::put, Router};

use crate::{
    plugin_helper_create_handler, plugin_helper_delete_handler, plugin_helper_scaffold_handler,
    plugin_helper_update_handler, plugin_registry_helpers_handler, plugin_registry_list_handler,
    plugin_registry_update_handler,
};

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/plugins/registry", get(plugin_registry_list_handler))
        .route(
            "/api/plugins/registry/:plugin_id",
            post(plugin_registry_update_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers",
            get(plugin_registry_helpers_handler).post(plugin_helper_create_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers/:helper_id",
            put(plugin_helper_update_handler).delete(plugin_helper_delete_handler),
        )
        .route(
            "/api/plugins/registry/:plugin_id/helpers/scaffold",
            get(plugin_helper_scaffold_handler),
        )
}
