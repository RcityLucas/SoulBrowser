use axum::{routing::get, routing::post, Router};

use crate::{self_heal_list_handler, self_heal_update_handler};

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route("/api/self-heal/strategies", get(self_heal_list_handler))
        .route(
            "/api/self-heal/strategies/:strategy_id",
            post(self_heal_update_handler),
        )
}
