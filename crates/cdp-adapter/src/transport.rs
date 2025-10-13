use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::async_process::Child;
use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::cdp::browser_protocol::target::SessionId as CdpSessionId;
use chromiumoxide::cdp::events::CdpEventMessage;
use chromiumoxide::conn::Connection;
use chromiumoxide_types::{CallId, CdpJsonEventMessage, Message, MethodId, Response};
use futures::StreamExt;
use serde_json::json;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex, OnceCell};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::config::CdpConfig;
use crate::error::{AdapterError, AdapterErrorKind};
use crate::util::extract_ws_url;

#[derive(Clone, Debug)]
pub struct TransportEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum CommandTarget {
    Browser,
    Session(String),
}

#[async_trait]
pub trait CdpTransport: Send + Sync {
    async fn start(&self) -> Result<(), AdapterError>;
    async fn next_event(&self) -> Option<TransportEvent>;
    async fn send_command(
        &self,
        target: CommandTarget,
        method: &str,
        params: Value,
    ) -> Result<Value, AdapterError>;
}

#[derive(Default)]
pub struct NoopTransport;

#[async_trait]
impl CdpTransport for NoopTransport {
    async fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        None
    }

    async fn send_command(
        &self,
        _target: CommandTarget,
        method: &str,
        _params: Value,
    ) -> Result<Value, AdapterError> {
        Err(AdapterError::new(AdapterErrorKind::Internal)
            .with_hint(format!("transport not available for method {method}")))
    }
}

pub struct ChromiumTransport {
    cfg: CdpConfig,
    state: OnceCell<Arc<RuntimeState>>,
}

impl ChromiumTransport {
    pub fn new(cfg: CdpConfig) -> Self {
        Self {
            cfg,
            state: OnceCell::new(),
        }
    }

    async fn runtime(&self) -> Result<Arc<RuntimeState>, AdapterError> {
        if let Some(rt) = self.state.get() {
            return Ok(rt.clone());
        }

        let state = Arc::new(RuntimeState::start(self.cfg.clone()).await?);
        let _ = self.state.set(state.clone());
        Ok(state)
    }
}

#[async_trait]
impl CdpTransport for ChromiumTransport {
    async fn start(&self) -> Result<(), AdapterError> {
        let runtime = self.runtime().await?;

        runtime
            .send_internal(
                CommandTarget::Browser,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                Duration::from_millis(self.cfg.default_deadline_ms),
            )
            .await?;

        Ok(())
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        match self.runtime().await {
            Ok(runtime) => runtime.next_event().await,
            Err(err) => {
                warn!(target: "cdp-transport", ?err, "transport not ready");
                None
            }
        }
    }

    async fn send_command(
        &self,
        target: CommandTarget,
        method: &str,
        params: Value,
    ) -> Result<Value, AdapterError> {
        let runtime = self.runtime().await?;
        runtime
            .send_internal(
                target,
                method,
                params,
                Duration::from_millis(self.cfg.default_deadline_ms),
            )
            .await
    }
}

struct ControlMessage {
    target: CommandTarget,
    method: String,
    params: Value,
    responder: oneshot::Sender<Result<Value, AdapterError>>,
}

struct RuntimeState {
    command_tx: mpsc::Sender<ControlMessage>,
    events_rx: Mutex<mpsc::Receiver<TransportEvent>>,
    loop_task: JoinHandle<()>,
    child: Mutex<Option<Child>>,
}

impl RuntimeState {
    async fn start(cfg: CdpConfig) -> Result<Self, AdapterError> {
        let browser_cfg = Self::browser_config(&cfg)?;
        let (child, ws_url) = Self::launch_browser(browser_cfg).await?;

        let conn = Connection::<CdpEventMessage>::connect(&ws_url)
            .await
            .map_err(|err| AdapterError::new(AdapterErrorKind::CdpIo).with_hint(err.to_string()))?;

        let (command_tx, command_rx) = mpsc::channel(128);
        let (events_tx, events_rx) = mpsc::channel(512);

        let loop_task = tokio::spawn(async move {
            if let Err(err) = Self::run_loop(conn, command_rx, events_tx).await {
                error!(target: "cdp-transport", ?err, "transport loop terminated with error");
            }
        });

        info!(target: "cdp-transport", url = %ws_url, "chromium connection established");

        Ok(Self {
            command_tx,
            events_rx: Mutex::new(events_rx),
            loop_task,
            child: Mutex::new(child),
        })
    }

    async fn send_internal(
        &self,
        target: CommandTarget,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, AdapterError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let message = ControlMessage {
            target,
            method: method.to_string(),
            params,
            responder: resp_tx,
        };

        self.command_tx
            .send(message)
            .await
            .map_err(|err| AdapterError::new(AdapterErrorKind::CdpIo).with_hint(err.to_string()))?;

        match tokio::time::timeout(deadline, resp_rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(err))) => Err(err),
            Ok(Err(_)) => Err(AdapterError::new(AdapterErrorKind::CdpIo)
                .with_hint("command response channel closed")),
            Err(_) => {
                Err(AdapterError::new(AdapterErrorKind::NavTimeout).with_hint("command timed out"))
            }
        }
    }

    async fn next_event(&self) -> Option<TransportEvent> {
        let mut guard = self.events_rx.lock().await;
        guard.recv().await
    }

    fn browser_config(cfg: &CdpConfig) -> Result<BrowserConfig, AdapterError> {
        let mut builder = BrowserConfig::builder()
            .request_timeout(Duration::from_millis(cfg.default_deadline_ms))
            .launch_timeout(Duration::from_secs(20));

        if !cfg.headless {
            builder = builder.with_head();
        }

        builder = builder.chrome_executable(cfg.executable.clone());
        builder = builder.user_data_dir(cfg.user_data_dir.clone());

        builder.build().map_err(|err| {
            AdapterError::new(AdapterErrorKind::Internal)
                .with_hint(format!("browser config error: {err}"))
        })
    }

    async fn launch_browser(
        config: BrowserConfig,
    ) -> Result<(Option<Child>, String), AdapterError> {
        let mut child = config.launch().map_err(|err| {
            AdapterError::new(AdapterErrorKind::Internal)
                .with_hint(format!("failed to launch chromium: {err}"))
        })?;

        let ws_url = extract_ws_url(&mut child)
            .await
            .map_err(|err| AdapterError::new(AdapterErrorKind::CdpIo).with_hint(err.to_string()))?;

        Ok((Some(child), ws_url))
    }

    async fn run_loop(
        mut conn: Connection<CdpEventMessage>,
        mut command_rx: mpsc::Receiver<ControlMessage>,
        mut event_tx: mpsc::Sender<TransportEvent>,
    ) -> Result<(), AdapterError> {
        let mut inflight: HashMap<CallId, oneshot::Sender<Result<Value, AdapterError>>> =
            HashMap::new();

        loop {
            tokio::select! {
                Some(cmd) = command_rx.recv() => {
                    Self::handle_command(&mut conn, cmd, &mut inflight).await?;
                }
                message = conn.next() => {
                    match message {
                        Some(Ok(Message::Response(resp))) => {
                            Self::handle_response(resp, &mut inflight);
                        }
                        Some(Ok(Message::Event(event))) => {
                            if let Err(err) = Self::handle_event(event, &mut event_tx).await {
                                warn!(target: "cdp-transport", ?err, "failed to forward event");
                            }
                        }
                        Some(Err(err)) => {
                            let adapter_err = AdapterError::new(AdapterErrorKind::CdpIo)
                                .with_hint(err.to_string());
                            for (_, sender) in inflight.drain() {
                                let _ = sender.send(Err(adapter_err.clone()));
                            }
                            return Err(adapter_err);
                        }
                        None => {
                            let err = AdapterError::new(AdapterErrorKind::CdpIo)
                                .with_hint("cdp connection closed");
                            for (_, sender) in inflight.drain() {
                                let _ = sender.send(Err(err.clone()));
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    async fn handle_command(
        conn: &mut Connection<CdpEventMessage>,
        cmd: ControlMessage,
        inflight: &mut HashMap<CallId, oneshot::Sender<Result<Value, AdapterError>>>,
    ) -> Result<(), AdapterError> {
        let session = match cmd.target {
            CommandTarget::Browser => None,
            CommandTarget::Session(session_id) => Some(CdpSessionId::from(session_id)),
        };

        let method_id: MethodId = cmd.method.clone().into();
        match conn.submit_command(method_id, session, cmd.params) {
            Ok(call_id) => {
                inflight.insert(call_id, cmd.responder);
                Ok(())
            }
            Err(err) => {
                let adapter_err =
                    AdapterError::new(AdapterErrorKind::CdpIo).with_hint(err.to_string());
                let _ = cmd.responder.send(Err(adapter_err.clone()));
                Err(adapter_err)
            }
        }
    }

    fn handle_response(
        resp: Response,
        inflight: &mut HashMap<CallId, oneshot::Sender<Result<Value, AdapterError>>>,
    ) {
        let entry = inflight.remove(&resp.id);
        let result = Self::extract_payload(resp);

        if let Some(sender) = entry {
            let _ = sender.send(result);
        }
    }

    async fn handle_event(
        event: CdpEventMessage,
        event_tx: &mut mpsc::Sender<TransportEvent>,
    ) -> Result<(), AdapterError> {
        let raw: CdpJsonEventMessage = event.try_into().map_err(|err| {
            AdapterError::new(AdapterErrorKind::Internal)
                .with_hint(format!("failed to decode cdp event: {err}"))
        })?;

        let payload = TransportEvent {
            method: raw.method.into_owned(),
            params: raw.params,
            session_id: raw.session_id,
        };

        event_tx
            .send(payload)
            .await
            .map_err(|err| AdapterError::new(AdapterErrorKind::Internal).with_hint(err.to_string()))
    }

    fn extract_payload(resp: Response) -> Result<Value, AdapterError> {
        if let Some(result) = resp.result {
            Ok(result)
        } else if let Some(error) = resp.error {
            Err(AdapterError::new(AdapterErrorKind::CdpIo)
                .with_hint(format!("cdp error {}: {}", error.code, error.message)))
        } else {
            Err(AdapterError::new(AdapterErrorKind::Internal).with_hint("empty cdp response"))
        }
    }
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        self.loop_task.abort();

        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(mut child) = guard.take() {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        if let Err(err) = child.kill().await {
                            warn!(target: "cdp-transport", ?err, "failed to kill chromium child");
                        }
                    });
                } else {
                    debug!(target: "cdp-transport", "no tokio runtime available to kill chromium child");
                }
            }
        }
    }
}
