use std::sync::Arc;

use axum::Router;

use crate::events::{EventsPort, ObserverEvents};
use crate::guard::RequestGuard;
use crate::http::{self, AdapterState};
use crate::idempotency::IdempotencyStore;
use crate::policy::AdapterPolicyHandle;
use crate::ports::{DispatcherPort, ReadonlyPort};
use crate::trace::AdapterTracer;

/// Builder for wiring the L7 adapter entrypoints.
#[derive(Clone)]
pub struct AdapterBootstrap {
    policy: AdapterPolicyHandle,
    dispatcher: Arc<dyn DispatcherPort>,
    readonly: Arc<dyn ReadonlyPort>,
    events: Arc<dyn EventsPort>,
    tracer: AdapterTracer,
    idempotency: Arc<IdempotencyStore>,
}

impl AdapterBootstrap {
    pub fn new(
        policy: AdapterPolicyHandle,
        dispatcher: Arc<dyn DispatcherPort>,
        readonly: Arc<dyn ReadonlyPort>,
    ) -> Self {
        Self {
            policy,
            dispatcher,
            readonly,
            events: Arc::new(ObserverEvents::default()),
            tracer: AdapterTracer::default(),
            idempotency: Arc::new(IdempotencyStore::new()),
        }
    }

    pub fn with_events(mut self, events: Arc<dyn EventsPort>) -> Self {
        self.events = events;
        self
    }

    pub fn with_tracer(mut self, tracer: AdapterTracer) -> Self {
        self.tracer = tracer;
        self
    }

    pub fn with_idempotency(mut self, store: Arc<IdempotencyStore>) -> Self {
        self.idempotency = store;
        self
    }

    fn state_internal(&self) -> AdapterState {
        AdapterState::new(
            self.policy.clone(),
            Arc::clone(&self.dispatcher),
            Arc::clone(&self.readonly),
            Arc::new(RequestGuard::new()),
            Arc::clone(&self.events),
            self.tracer.clone(),
            Arc::clone(&self.idempotency),
        )
    }

    pub fn state(&self) -> AdapterState {
        self.state_internal()
    }

    pub fn into_state(self) -> AdapterState {
        AdapterState::new(
            self.policy,
            self.dispatcher,
            self.readonly,
            Arc::new(RequestGuard::new()),
            self.events,
            self.tracer,
            self.idempotency,
        )
    }

    /// Build the HTTP router; additional transports (gRPC/MCP) will be
    /// added in subsequent phases.
    pub fn build_http(&self) -> Router {
        http::router_with_state(self.state_internal())
    }
}
