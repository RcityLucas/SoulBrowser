use std::sync::Arc;
use std::time::{Instant, SystemTime};

use serde_json::Value;

use crate::audit::{AuditPort, NoopAudit, PluginAuditEvent};
use crate::errors::{PluginError, PluginResult};
use crate::guard::PluginGuard;
use crate::hooks::{HookCtx, HookExecutor};
use crate::manifest::PluginManifest;
use crate::metrics::observe_call;
use crate::policy::{PluginPolicyHandle, PluginPolicyView, TenantPolicy};
use crate::privacy;
use crate::registry::{PluginRecord, PluginRegistry, PluginStatus};
use crate::sandbox::SandboxHost;

#[derive(Clone)]
pub struct PluginRuntime {
    registry: PluginRegistry,
    guard: PluginGuard,
    executor: Arc<dyn HookExecutor>,
    audit: Arc<dyn AuditPort>,
    policy: PluginPolicyHandle,
}

impl PluginRuntime {
    pub fn new(policy: PluginPolicyHandle) -> Self {
        Self::with_executor(policy, Arc::new(SandboxHost::new()))
    }

    pub fn with_executor(policy: PluginPolicyHandle, executor: Arc<dyn HookExecutor>) -> Self {
        Self {
            registry: PluginRegistry::new(policy.clone()),
            guard: PluginGuard::new(policy.clone()),
            executor,
            audit: Arc::new(NoopAudit),
            policy,
        }
    }

    pub fn with_audit<A>(mut self, audit: Arc<A>) -> Self
    where
        A: AuditPort + 'static,
    {
        self.audit = audit;
        self
    }

    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    pub fn guard(&self) -> &PluginGuard {
        &self.guard
    }

    pub fn executor(&self) -> Arc<dyn HookExecutor> {
        Arc::clone(&self.executor)
    }

    pub fn policy(&self) -> PluginPolicyHandle {
        self.policy.clone()
    }

    pub fn register(&self, manifest: PluginManifest) -> PluginResult<()> {
        self.guard.check_install(&manifest)?;
        self.registry
            .upsert(manifest)
            .map_err(|err| PluginError::Manifest(err.to_string()))
    }

    pub async fn invoke(
        &self,
        tenant: &str,
        plugin: &str,
        hook: &str,
        payload: Value,
        mut ctx: HookCtx,
    ) -> PluginResult<Value> {
        let policy = self.policy.snapshot();
        let tenant_policy = select_tenant(&policy, tenant)?;
        ensure_plugins_enabled(&policy, tenant_policy)?;

        let record = self
            .registry
            .get(plugin)
            .ok_or_else(|| PluginError::NotFound(plugin.to_string()))?;
        ensure_plugin_enabled(&record)?;
        ensure_hook_allowed(&policy, plugin, hook)?;

        if ctx.tenant.is_none() {
            ctx.tenant = Some(tenant.to_string());
        }

        let permit = self.guard.acquire(plugin).await?;
        let manifest = Arc::new(record.manifest.clone());

        let redacted_payload = privacy::redact_payload(payload, ctx.trace_id.clone());
        let started = Instant::now();
        let result = self
            .executor
            .invoke(manifest, hook, redacted_payload, ctx.clone())
            .await;
        drop(permit);

        let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        observe_call(plugin, hook, result.is_ok(), latency_ms);

        match result {
            Ok(value) => {
                self.audit.record(PluginAuditEvent {
                    plugin: plugin.to_string(),
                    hook: hook.to_string(),
                    tenant: Some(tenant.to_string()),
                    tags: Vec::new(),
                    timestamp: Some(SystemTime::now()),
                });
                Ok(privacy::redact_payload(value, ctx.trace_id))
            }
            Err(err) => Err(err),
        }
    }
}

fn select_tenant<'a>(policy: &'a PluginPolicyView, id: &str) -> PluginResult<&'a TenantPolicy> {
    policy
        .tenants
        .iter()
        .find(|tenant| tenant.id == id && tenant.enable)
        .ok_or(PluginError::Blocked)
}

fn ensure_plugins_enabled(policy: &PluginPolicyView, tenant: &TenantPolicy) -> PluginResult<()> {
    if !policy.enable {
        return Err(PluginError::Disabled);
    }
    if !tenant.allow_plugins {
        return Err(PluginError::Blocked);
    }
    Ok(())
}

fn ensure_plugin_enabled(record: &PluginRecord) -> PluginResult<()> {
    match record.status {
        PluginStatus::Enabled => Ok(()),
        PluginStatus::Disabled | PluginStatus::Blocked => Err(PluginError::Blocked),
    }
}

fn ensure_hook_allowed(policy: &PluginPolicyView, plugin: &str, hook: &str) -> PluginResult<()> {
    if policy.hook_allow.is_empty() {
        return Ok(());
    }
    let allowed = policy
        .hook_allow
        .iter()
        .any(|allow| allow.plugin == plugin && allow.hook == hook);
    if allowed {
        Ok(())
    } else {
        Err(PluginError::Blocked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct StubExecutor;

    #[async_trait]
    impl HookExecutor for StubExecutor {
        async fn invoke(
            &self,
            _manifest: Arc<PluginManifest>,
            hook: &str,
            payload: Value,
            _ctx: HookCtx,
        ) -> PluginResult<Value> {
            Ok(json!({
                "hook": hook,
                "payload": payload,
            }))
        }
    }

    fn manifest() -> PluginManifest {
        PluginManifest {
            name: "demo.plugin".into(),
            version: "1.0.0".into(),
            description: None,
            entry: "plugin.wasm".into(),
            permissions: Default::default(),
            hooks: vec!["pre_tool".into()],
            provider: None,
        }
    }

    fn allow_all_policy() -> PluginPolicyView {
        PluginPolicyView {
            enable: true,
            tenants: vec![TenantPolicy {
                id: "tenant-a".into(),
                enable: true,
                allow_plugins: true,
            }],
            default_trust: crate::policy::Trust::Internal,
            allowed_extpoints: Vec::new(),
            kill_switch: Vec::new(),
            require_signature: false,
            allowed_cas: Vec::new(),
            allow_runtime: vec!["wasm32-wasi".into()],
            cpu_ms: 20,
            wall_ms: 50,
            mem_mb: 64,
            ipc_bytes: 32 * 1024,
            kv_space_mb: 4,
            concurrency: 2,
            net_enable: false,
            net_whitelist: Vec::new(),
            fs_enable: false,
            hook_allow: Vec::new(),
        }
    }

    #[tokio::test]
    async fn register_and_invoke_plugin() {
        let handle = PluginPolicyHandle::global();
        crate::policy::set_policy(allow_all_policy());
        let runtime = PluginRuntime::with_executor(handle.clone(), Arc::new(StubExecutor));
        runtime.register(manifest()).unwrap();

        let payload = json!({"input": "value"});
        let result = runtime
            .invoke(
                "tenant-a",
                "demo.plugin",
                "pre_tool",
                payload.clone(),
                HookCtx::default(),
            )
            .await
            .unwrap();
        assert_eq!(result["hook"], "pre_tool");
        assert_eq!(result["payload"], payload);
    }

    #[tokio::test]
    async fn hook_not_allowed_is_blocked() {
        let handle = PluginPolicyHandle::global();
        let mut policy = allow_all_policy();
        policy.hook_allow.push(crate::policy::HookAllow {
            plugin: "demo.plugin".into(),
            hook: "pre_tool".into(),
            for_tools: None,
            views: None,
        });
        crate::policy::set_policy(policy);

        let runtime = PluginRuntime::with_executor(handle.clone(), Arc::new(StubExecutor));
        runtime.register(manifest()).unwrap();

        let err = runtime
            .invoke(
                "tenant-a",
                "demo.plugin",
                "post_tool",
                json!({}),
                HookCtx::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PluginError::Blocked));
    }
}
