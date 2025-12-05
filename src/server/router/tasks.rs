use axum::{routing::get, routing::post, Router};

use crate::{
    cancel_task_handler, create_task_annotation_handler, create_task_handler, execute_task_handler,
    get_task_handler, list_tasks_handler, recordings_detail_handler, recordings_list_handler,
    task_annotations_handler, task_artifact_download_handler, task_artifacts_handler,
    task_events_sse_handler, task_logs_handler, task_observations_handler, task_status_handler,
    task_stream_handler,
};

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new()
        .route(
            "/api/tasks",
            get(list_tasks_handler).post(create_task_handler),
        )
        .route("/api/tasks/:task_id", get(get_task_handler))
        .route("/api/tasks/:task_id/status", get(task_status_handler))
        .route("/api/tasks/:task_id/logs", get(task_logs_handler))
        .route(
            "/api/tasks/:task_id/observations",
            get(task_observations_handler),
        )
        .route("/api/recordings", get(recordings_list_handler))
        .route(
            "/api/recordings/:session_id",
            get(recordings_detail_handler),
        )
        .route("/api/tasks/:task_id/artifacts", get(task_artifacts_handler))
        .route(
            "/api/tasks/:task_id/artifacts/:artifact",
            get(task_artifact_download_handler),
        )
        .route(
            "/api/tasks/:task_id/annotations",
            get(task_annotations_handler).post(create_task_annotation_handler),
        )
        .route("/api/tasks/:task_id/events", get(task_events_sse_handler))
        .route("/api/tasks/:task_id/stream", get(task_stream_handler))
        .route("/api/tasks/:task_id/execute", post(execute_task_handler))
        .route("/api/tasks/:task_id/cancel", post(cancel_task_handler))
}
