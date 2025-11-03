use std::sync::Arc;

use axum::{
    routing::{delete, get, post},
    Router,
};

use crate::dispatcher::{NoopDispatcher, ToolDispatcher};
use crate::handlers;
use crate::policy::WebDriverBridgePolicyHandle;
use crate::state::SessionStore;
use crate::trace::BridgeTracer;

#[derive(Clone)]
pub struct WebDriverBridge {
    policy: WebDriverBridgePolicyHandle,
    state: Arc<SessionStore>,
    tracer: BridgeTracer,
    dispatcher: Arc<dyn ToolDispatcher>,
}

impl WebDriverBridge {
    pub fn new(policy: WebDriverBridgePolicyHandle) -> Self {
        Self {
            policy,
            state: Arc::new(SessionStore::default()),
            tracer: BridgeTracer::default(),
            dispatcher: Arc::new(NoopDispatcher),
        }
    }

    pub fn with_tracer(mut self, tracer: BridgeTracer) -> Self {
        self.tracer = tracer;
        self
    }

    pub fn with_dispatcher(mut self, dispatcher: Arc<dyn ToolDispatcher>) -> Self {
        self.dispatcher = dispatcher;
        self
    }

    pub fn build(self) -> Router {
        Router::new()
            .route("/status", get(handlers::status))
            .route("/session", post(handlers::create_session))
            .route("/session/:id", delete(handlers::delete_session))
            .route("/session/:id/url", post(handlers::navigate_to_url))
            .route("/session/:id/url", get(handlers::current_url))
            .route("/session/:id/title", get(handlers::get_title))
            .route("/session/:id/element", post(handlers::find_element))
            .route("/session/:id/elements", post(handlers::find_elements))
            .route(
                "/session/:id/element/:element_id/click",
                post(handlers::click_element),
            )
            .route(
                "/session/:id/element/:element_id/text",
                get(handlers::element_text),
            )
            .route(
                "/session/:id/element/:element_id/attribute/:attr",
                get(handlers::element_attribute),
            )
            .with_state(handlers::BridgeCtx {
                policy: self.policy,
                state: self.state,
                tracer: self.tracer,
                dispatcher: self.dispatcher,
            })
    }
}
