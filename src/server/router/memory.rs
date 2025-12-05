use axum::{routing::delete, routing::get, Router};

use crate::{
    memory_create_handler, memory_delete_handler, memory_list_handler, memory_stats_handler,
    memory_update_handler,
};

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route(
            "/api/memory",
            get(memory_list_handler).post(memory_create_handler),
        )
        .route(
            "/api/memory/:record_id",
            delete(memory_delete_handler).put(memory_update_handler),
        )
        .route("/api/memory/stats", get(memory_stats_handler))
}
