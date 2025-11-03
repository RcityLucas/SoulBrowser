use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::task;
use wasmtime::{
    component::Component, component::Linker, component::ResourceTable, Config, Engine, Store,
};
use wasmtime_wasi::preview2::command;
use wasmtime_wasi::preview2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::preview2::{WasiCtx, WasiCtxBuilder, WasiView};

use crate::errors::{PluginError, PluginResult};
use crate::hooks::{HookCtx, HookExecutor};
use crate::manifest::PluginManifest;

#[derive(Clone)]
pub struct SandboxHost {
    engine: Engine,
    components: Arc<DashMap<String, Component>>,
}

impl Default for SandboxHost {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxHost {
    pub fn new() -> Self {
        let mut config = Config::default();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).expect("failed to create wasmtime engine");
        Self {
            engine,
            components: Arc::new(DashMap::new()),
        }
    }

    pub async fn invoke_hook(
        &self,
        manifest: Arc<PluginManifest>,
        hook: &str,
        payload: Value,
    ) -> PluginResult<Value> {
        let engine = self.engine.clone();
        let components = self.components.clone();
        let hook = hook.to_string();
        let manifest_clone = manifest.clone();

        let value = task::spawn_blocking(move || {
            invoke_blocking(&engine, &components, manifest_clone, hook, payload)
        })
        .await
        .map_err(|err| PluginError::Sandbox(format!("sandbox task failed: {}", err)))??;

        Ok(value)
    }
}

fn invoke_blocking(
    engine: &Engine,
    components: &DashMap<String, Component>,
    manifest: Arc<PluginManifest>,
    hook: String,
    payload: Value,
) -> PluginResult<Value> {
    let component = load_component(engine, components, &manifest)?;

    let input = json!({
        "hook": hook,
        "payload": payload,
    })
    .to_string();

    let stdin = MemoryInputPipe::new(Bytes::from(input));
    let stdout = MemoryOutputPipe::new(1024 * 1024);
    let stderr = MemoryOutputPipe::new(1024 * 1024);

    let mut builder = WasiCtxBuilder::new();
    builder.stdin(stdin);
    builder.stdout(stdout.clone());
    builder.stderr(stderr.clone());
    builder.env("PLUGIN_HOOK", hook.clone());

    let wasi = builder.build();

    let mut store = Store::new(
        engine,
        SandboxContext {
            table: ResourceTable::new(),
            wasi,
        },
    );

    let mut linker = Linker::new(engine);
    command::sync::add_to_linker(&mut linker)
        .map_err(|err| PluginError::Sandbox(err.to_string()))?;

    let (command, _instance) = command::sync::Command::instantiate(&mut store, &component, &linker)
        .map_err(|err| PluginError::Sandbox(err.to_string()))?;

    command
        .wasi_cli_run()
        .call_run(&mut store)
        .map_err(|err| PluginError::Sandbox(err.to_string()))?
        .map_err(|()| PluginError::Sandbox("component reported failure".into()))?;

    let output_bytes = stdout.contents();
    if output_bytes.is_empty() {
        Ok(Value::Null)
    } else {
        let output = String::from_utf8(output_bytes.to_vec())
            .map_err(|err| PluginError::Sandbox(err.to_string()))?;
        if output.trim().is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(&output).map_err(|err| PluginError::Sandbox(err.to_string()))
        }
    }
}

fn component_path(manifest: &PluginManifest) -> PathBuf {
    let entry = Path::new(&manifest.entry);
    if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        Path::new("plugins").join(&manifest.name).join(entry)
    }
}

fn load_component(
    engine: &Engine,
    components: &DashMap<String, Component>,
    manifest: &PluginManifest,
) -> PluginResult<Component> {
    if let Some(existing) = components.get(&manifest.name) {
        return Ok(existing.clone());
    }

    let path = component_path(manifest);
    let component = Component::from_file(engine, &path)
        .map_err(|err| PluginError::Sandbox(format!("failed to load component: {}", err)))?;
    components.insert(manifest.name.clone(), component.clone());
    Ok(component)
}

struct SandboxContext {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for SandboxContext {
    fn table(&self) -> &ResourceTable {
        &self.table
    }

    fn table_mut(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    fn ctx(&self) -> &WasiCtx {
        &self.wasi
    }

    fn ctx_mut(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

#[async_trait]
impl HookExecutor for SandboxHost {
    async fn invoke(
        &self,
        manifest: Arc<PluginManifest>,
        hook: &str,
        payload: Value,
        _ctx: HookCtx,
    ) -> PluginResult<Value> {
        self.invoke_hook(manifest, hook, payload).await
    }
}
