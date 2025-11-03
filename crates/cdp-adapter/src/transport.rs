use std::collections::HashMap;
use std::convert::TryInto;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::async_process::Child;
use chromiumoxide::browser::BrowserConfig;
use chromiumoxide::cdp::browser_protocol::target::SessionId as CdpSessionId;
use chromiumoxide::cdp::events::CdpEventMessage;
use chromiumoxide::conn::Connection;
use chromiumoxide::error::CdpError;
use chromiumoxide_types::{CallId, CdpJsonEventMessage, Message, MethodId, Response};
use futures::{future::BoxFuture, StreamExt};
use serde_json::json;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, Mutex, OnceCell};
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};
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

type RuntimeFactory = Arc<
    dyn Fn(CdpConfig) -> BoxFuture<'static, Result<Arc<RuntimeState>, AdapterError>> + Send + Sync,
>;

#[derive(Clone)]
pub struct ChromiumTransport {
    cfg: CdpConfig,
    state: Arc<OnceCell<Mutex<Option<Arc<RuntimeState>>>>>,
    factory: RuntimeFactory,
}

impl ChromiumTransport {
    pub fn new(cfg: CdpConfig) -> Self {
        let factory: RuntimeFactory = Arc::new(|cfg: CdpConfig| {
            Box::pin(async move {
                let state = RuntimeState::start(cfg).await?;
                Ok(Arc::new(state))
            })
        });

        Self {
            cfg,
            state: Arc::new(OnceCell::new()),
            factory,
        }
    }

    async fn runtime(&self) -> Result<Arc<RuntimeState>, AdapterError> {
        let cell = self.state.get_or_init(|| async { Mutex::new(None) }).await;
        let mut guard = cell.lock().await;

        if let Some(rt) = guard.as_ref() {
            if rt.is_alive() {
                return Ok(rt.clone());
            }
        }

        let runtime = (self.factory)(self.cfg.clone()).await?;
        *guard = Some(runtime.clone());
        Ok(runtime)
    }

    #[cfg(test)]
    fn with_factory(cfg: CdpConfig, factory: RuntimeFactory) -> Self {
        Self {
            cfg,
            state: Arc::new(OnceCell::new()),
            factory,
        }
    }
}

#[async_trait]
impl CdpTransport for ChromiumTransport {
    async fn start(&self) -> Result<(), AdapterError> {
        let runtime = self.runtime().await?;

        let deadline = Duration::from_millis(self.cfg.default_deadline_ms);

        runtime
            .send_internal(
                CommandTarget::Browser,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                deadline,
            )
            .await?;

        runtime
            .send_internal(
                CommandTarget::Browser,
                "Target.setAutoAttach",
                json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": false,
                    "flatten": true,
                }),
                deadline,
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
        match runtime
            .send_internal(
                target,
                method,
                params,
                Duration::from_millis(self.cfg.default_deadline_ms),
            )
            .await
        {
            Ok(value) => Ok(value),
            Err(err) => Err(err),
        }
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
    heartbeat_task: Option<JoinHandle<()>>,
    child: Mutex<Option<Child>>,
    alive: Arc<AtomicBool>,
}

impl RuntimeState {
    async fn start(cfg: CdpConfig) -> Result<Self, AdapterError> {
        let (child, ws_url) = if let Some(url) = cfg.websocket_url.clone() {
            (None, url)
        } else {
            let browser_cfg = Self::browser_config(&cfg)?;
            Self::launch_browser(browser_cfg).await?
        };

        let conn = Connection::<CdpEventMessage>::connect(&ws_url)
            .await
            .map_err(|err| AdapterError::new(AdapterErrorKind::CdpIo).with_hint(err.to_string()))?;

        let (command_tx, command_rx) = mpsc::channel(128);
        let (events_tx, events_rx) = mpsc::channel(512);

        let alive = Arc::new(AtomicBool::new(true));
        let loop_alive = alive.clone();
        let heartbeat_alive = alive.clone();
        let heartbeat_tx = command_tx.clone();

        let loop_task = tokio::spawn(async move {
            let result = Self::run_loop(conn, command_rx, events_tx).await;
            loop_alive.store(false, Ordering::Relaxed);
            if let Err(err) = result {
                error!(target: "cdp-transport", ?err, "transport loop terminated with error");
            }
        });

        let heartbeat_task = Self::spawn_heartbeat(
            heartbeat_tx,
            heartbeat_alive,
            Duration::from_millis(cfg.heartbeat_interval_ms),
            Duration::from_millis(cfg.default_deadline_ms),
        );

        info!(target: "cdp-transport", url = %ws_url, "chromium connection established");

        Ok(Self {
            command_tx,
            events_rx: Mutex::new(events_rx),
            loop_task,
            heartbeat_task,
            child: Mutex::new(child),
            alive,
        })
    }

    #[cfg(test)]
    fn test_stub() -> (Arc<Self>, Arc<AtomicBool>) {
        let (command_tx, _command_rx) = mpsc::channel(8);
        let (_events_tx, events_rx) = mpsc::channel(8);
        let alive = Arc::new(AtomicBool::new(true));
        let loop_alive = alive.clone();
        let loop_task = tokio::spawn(async move {
            futures::future::pending::<()>().await;
            loop_alive.store(false, Ordering::Relaxed);
        });

        (
            Arc::new(Self {
                command_tx,
                events_rx: Mutex::new(events_rx),
                loop_task,
                heartbeat_task: None,
                child: Mutex::new(None),
                alive: alive.clone(),
            }),
            alive,
        )
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
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

    fn spawn_heartbeat(
        sender: mpsc::Sender<ControlMessage>,
        alive: Arc<AtomicBool>,
        interval_duration: Duration,
        deadline: Duration,
    ) -> Option<JoinHandle<()>> {
        if interval_duration.as_millis() == 0 {
            return None;
        }

        let response_deadline = if deadline > Duration::from_secs(5) {
            Duration::from_secs(5)
        } else {
            deadline
        };

        Some(tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            while alive.load(Ordering::Relaxed) {
                ticker.tick().await;

                if !alive.load(Ordering::Relaxed) {
                    break;
                }

                let (resp_tx, resp_rx) = oneshot::channel();
                let message = ControlMessage {
                    target: CommandTarget::Browser,
                    method: "Browser.getVersion".to_string(),
                    params: Value::Object(Default::default()),
                    responder: resp_tx,
                };

                if sender.send(message).await.is_err() {
                    debug!(target: "cdp-transport", "heartbeat send failed (channel closed)");
                    break;
                }

                match tokio::time::timeout(response_deadline, resp_rx).await {
                    Ok(Ok(Ok(_))) => {
                        // keep-alive succeeded
                    }
                    Ok(Ok(Err(err))) => {
                        warn!(target: "cdp-transport", ?err, "heartbeat command error");
                        break;
                    }
                    Ok(Err(_)) => {
                        debug!(
                            target: "cdp-transport",
                            "heartbeat response channel closed"
                        );
                        break;
                    }
                    Err(_) => {
                        warn!(target: "cdp-transport", "heartbeat timed out");
                        break;
                    }
                }
            }
        }))
    }

    fn browser_config(cfg: &CdpConfig) -> Result<BrowserConfig, AdapterError> {
        if cfg.websocket_url.is_some() {
            return Err(AdapterError::new(AdapterErrorKind::Internal)
                .with_hint("browser_config requested while websocket_url present"));
        }

        if !cfg.executable.as_os_str().is_empty() && !cfg.executable.exists() {
            return Err(AdapterError::new(AdapterErrorKind::CdpIo)
                .with_hint(format!(
                    "chrome executable not found at {}",
                    cfg.executable.display()
                ))
                .with_data(json!({
                    "expected": cfg.executable,
                    "hint": "Set SOULBROWSER_CHROME to the full path of chrome/chromium."
                })));
        }

        if cfg.websocket_url.is_some() {
            return Ok(BrowserConfig::builder().build().map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("browser config error: {err}"))
            })?);
        }

        let profile_dir = if cfg.user_data_dir.is_absolute() {
            cfg.user_data_dir.clone()
        } else {
            let cwd = std::env::current_dir().map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("failed to resolve cwd for user-data-dir: {err}"))
            })?;
            cwd.join(&cfg.user_data_dir)
        };

        if let Some(parent) = profile_dir.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                AdapterError::new(AdapterErrorKind::Internal)
                    .with_hint(format!("failed to create user-data-dir parent: {err}"))
            })?;
        }
        fs::create_dir_all(&profile_dir).map_err(|err| {
            AdapterError::new(AdapterErrorKind::Internal)
                .with_hint(format!("failed to ensure user-data-dir: {err}"))
        })?;

        let mut builder = BrowserConfig::builder()
            .request_timeout(Duration::from_millis(cfg.default_deadline_ms))
            .launch_timeout(Duration::from_secs(20));

        if !cfg.headless {
            builder = builder.with_head();
        }

        if std::env::var("SOULBROWSER_DISABLE_SANDBOX")
            .map(|v| v != "0" && v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            builder = builder.no_sandbox();
        }

        let mut args = vec![
            "--disable-background-networking",
            "--disable-background-timer-throttling",
            "--disable-breakpad",
            "--disable-client-side-phishing-detection",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-dev-shm-usage",
            "--disable-extensions",
            "--disable-hang-monitor",
            "--disable-popup-blocking",
            "--disable-prompt-on-repost",
            "--disable-sync",
            "--metrics-recording-only",
            "--no-first-run",
            "--no-default-browser-check",
            "--password-store=basic",
            "--remote-allow-origins=*",
            "--use-mock-keychain",
        ];
        if cfg.headless {
            args.push("--headless=new");
            args.push("--hide-scrollbars");
            args.push("--mute-audio");
        }
        builder = builder.args(args);

        if !cfg.executable.as_os_str().is_empty() {
            builder = builder.chrome_executable(cfg.executable.clone());
        }
        builder = builder.user_data_dir(profile_dir);

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
                            let adapter_err = Self::map_cdp_error(err);
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
            let retriable = error.code >= 500;
            Err(AdapterError::new(AdapterErrorKind::CdpIo)
                .with_hint(format!("cdp error {}: {}", error.code, error.message))
                .retriable(retriable))
        } else {
            Err(AdapterError::new(AdapterErrorKind::Internal).with_hint("empty cdp response"))
        }
    }

    fn map_cdp_error(err: CdpError) -> AdapterError {
        let hint = err.to_string();
        match err {
            CdpError::Timeout => AdapterError::new(AdapterErrorKind::NavTimeout)
                .with_hint(hint)
                .retriable(true),
            CdpError::FrameNotFound(_) => {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(hint)
            }
            CdpError::JavascriptException(_) => {
                AdapterError::new(AdapterErrorKind::Internal).with_hint(hint)
            }
            CdpError::Serde(_) => AdapterError::new(AdapterErrorKind::Internal).with_hint(hint),
            CdpError::Ws(_)
            | CdpError::Io(_)
            | CdpError::Chrome(_)
            | CdpError::ChromeMessage(_)
            | CdpError::ChannelSendError(_)
            | CdpError::NoResponse
            | CdpError::LaunchExit(_, _)
            | CdpError::LaunchTimeout(_)
            | CdpError::LaunchIo(_, _)
            | CdpError::DecodeError(_)
            | CdpError::ScrollingFailed(_)
            | CdpError::NotFound
            | CdpError::Url(_) => AdapterError::new(AdapterErrorKind::CdpIo)
                .with_hint(hint)
                .retriable(true),
        }
    }
}

impl Drop for RuntimeState {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        self.loop_task.abort();
        if let Some(handle) = &self.heartbeat_task {
            handle.abort();
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use tokio::sync::Mutex as TokioMutex;

    #[tokio::test]
    async fn recreates_runtime_when_dead() {
        let spawn_count = Arc::new(AtomicUsize::new(0));
        let alive_flags = Arc::new(TokioMutex::new(Vec::<Arc<AtomicBool>>::new()));

        let factory: RuntimeFactory = {
            let spawn_count = spawn_count.clone();
            let alive_flags = alive_flags.clone();
            Arc::new(move |cfg: CdpConfig| {
                let spawn_count = spawn_count.clone();
                let alive_flags = alive_flags.clone();
                Box::pin(async move {
                    let _ = cfg;
                    spawn_count.fetch_add(1, AtomicOrdering::SeqCst);
                    let (runtime, alive) = RuntimeState::test_stub();
                    alive_flags.lock().await.push(alive);
                    Ok(runtime)
                })
            })
        };

        let transport = ChromiumTransport::with_factory(CdpConfig::default(), factory);

        let rt1 = transport.runtime().await.expect("runtime #1");
        assert_eq!(spawn_count.load(AtomicOrdering::SeqCst), 1);

        {
            let guard = alive_flags.lock().await;
            guard[0].store(false, AtomicOrdering::SeqCst);
        }

        let rt1_clone = rt1.clone();
        drop(rt1);

        let rt2 = transport.runtime().await.expect("runtime #2");
        assert_eq!(spawn_count.load(AtomicOrdering::SeqCst), 2);
        assert!(!Arc::ptr_eq(&rt1_clone, &rt2));

        {
            let guard = alive_flags.lock().await;
            assert!(guard.len() >= 2);
        }

        drop(rt1_clone);
        drop(rt2);
    }
}
