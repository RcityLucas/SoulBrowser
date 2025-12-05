use std::sync::Arc;

use cdp_adapter::{
    adapter::CookieParam,
    commands::{
        Anchor, AxSnapshotConfig, AxSnapshotResult, DomSnapshotConfig, DomSnapshotResult,
        QuerySpec, SelectSpec,
    },
    ids::PageId as AdapterPageId,
    AdapterError,
};
use stealth::{
    config::{StealthProfile, StealthProfileBundle, Viewport},
    ProfileCatalog, StealthControl, StealthRuntime,
};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockCdp {
    ua_calls: Mutex<
        Vec<(
            AdapterPageId,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
    >,
    timezone_calls: Mutex<Vec<(AdapterPageId, String)>>,
    metrics_calls: Mutex<Vec<(AdapterPageId, u32, u32, f64, bool)>>,
    touch_calls: Mutex<Vec<(AdapterPageId, bool)>>,
}

#[async_trait::async_trait]
impl cdp_adapter::Cdp for MockCdp {
    async fn navigate(
        &self,
        _page: AdapterPageId,
        _url: &str,
        _deadline: std::time::Duration,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn query(
        &self,
        _page: AdapterPageId,
        _spec: QuerySpec,
    ) -> Result<Vec<Anchor>, AdapterError> {
        Ok(Vec::new())
    }

    async fn click(
        &self,
        _page: AdapterPageId,
        _selector: &str,
        _deadline: std::time::Duration,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn type_text(
        &self,
        _page: AdapterPageId,
        _selector: &str,
        _text: &str,
        _deadline: std::time::Duration,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn select_option(
        &self,
        _page: AdapterPageId,
        _spec: SelectSpec,
        _deadline: std::time::Duration,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn wait_basic(
        &self,
        _page: AdapterPageId,
        _gate: String,
        _timeout: std::time::Duration,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn screenshot(
        &self,
        _page: AdapterPageId,
        _deadline: std::time::Duration,
    ) -> Result<Vec<u8>, AdapterError> {
        Ok(Vec::new())
    }

    async fn set_network_tap(
        &self,
        _page: AdapterPageId,
        _enabled: bool,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn grant_permissions(
        &self,
        _origin: &str,
        _permissions: &[String],
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn reset_permissions(
        &self,
        _origin: &str,
        _permissions: &[String],
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn set_cookies(
        &self,
        _page: AdapterPageId,
        _cookies: &[CookieParam],
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    async fn dom_snapshot(
        &self,
        _page: AdapterPageId,
        _config: DomSnapshotConfig,
    ) -> Result<DomSnapshotResult, AdapterError> {
        Ok(DomSnapshotResult {
            documents: Vec::new(),
            strings: Vec::new(),
            raw: serde_json::Value::Null,
        })
    }

    async fn ax_snapshot(
        &self,
        _page: AdapterPageId,
        _config: AxSnapshotConfig,
    ) -> Result<AxSnapshotResult, AdapterError> {
        Ok(AxSnapshotResult {
            nodes: Vec::new(),
            tree_id: None,
            raw: serde_json::Value::Null,
        })
    }

    async fn set_user_agent(
        &self,
        page: AdapterPageId,
        user_agent: &str,
        accept_language: Option<&str>,
        platform: Option<&str>,
        locale: Option<&str>,
    ) -> Result<(), AdapterError> {
        let mut guard = self.ua_calls.lock().await;
        guard.push((
            page,
            user_agent.to_string(),
            accept_language.map(|s| s.to_string()),
            platform.map(|s| s.to_string()),
            locale.map(|s| s.to_string()),
        ));
        Ok(())
    }

    async fn set_timezone(&self, page: AdapterPageId, timezone: &str) -> Result<(), AdapterError> {
        let mut guard = self.timezone_calls.lock().await;
        guard.push((page, timezone.to_string()));
        Ok(())
    }

    async fn set_device_metrics(
        &self,
        page: AdapterPageId,
        width: u32,
        height: u32,
        device_scale_factor: f64,
        mobile: bool,
    ) -> Result<(), AdapterError> {
        let mut guard = self.metrics_calls.lock().await;
        guard.push((page, width, height, device_scale_factor, mobile));
        Ok(())
    }

    async fn set_touch_emulation(
        &self,
        page: AdapterPageId,
        enabled: bool,
    ) -> Result<(), AdapterError> {
        let mut guard = self.touch_calls.lock().await;
        guard.push((page, enabled));
        Ok(())
    }
}

#[tokio::test]
async fn apply_and_retrieve_profile() {
    let runtime = StealthRuntime::new();
    runtime
        .load_catalog(ProfileCatalog {
            profiles: vec!["sp_generic".into()],
        })
        .await;
    runtime.load_bundle(sample_profile_bundle()).await;

    let profile_id = runtime.apply_stealth("https://example.com").await.unwrap();
    assert!(runtime
        .applied_profile_for("https://example.com")
        .map(|p| p.profile_id == profile_id)
        .unwrap_or(false));
}

#[tokio::test]
async fn ensure_consistency_requires_profile() {
    let runtime = StealthRuntime::new();
    let err = runtime.ensure_consistency("https://missing.com").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn configure_page_applies_profile() {
    let adapter = Arc::new(MockCdp::default());
    let runtime = StealthRuntime::with_adapter(adapter.clone());
    runtime
        .load_catalog(ProfileCatalog {
            profiles: vec!["sp_generic".into()],
        })
        .await;
    runtime.load_bundle(sample_profile_bundle()).await;

    runtime.apply_stealth("https://example.com").await.unwrap();

    let page_id = AdapterPageId::new();
    runtime
        .configure_page(page_id, "https://example.com")
        .await
        .unwrap();

    let ua_calls = adapter.ua_calls.lock().await;
    assert_eq!(ua_calls.len(), 1);
    assert_eq!(ua_calls[0].1, "Mozilla/5.0");
    assert_eq!(ua_calls[0].2.as_deref(), Some("en-US"));
    assert_eq!(ua_calls[0].3.as_deref(), Some("Win32"));
    assert_eq!(ua_calls[0].4.as_deref(), Some("en-US"));
    drop(ua_calls);

    let tz_calls = adapter.timezone_calls.lock().await;
    assert_eq!(tz_calls.len(), 1);
    assert_eq!(tz_calls[0].1, "America/Los_Angeles");
    drop(tz_calls);

    let metrics_calls = adapter.metrics_calls.lock().await;
    assert_eq!(metrics_calls.len(), 1);
    assert_eq!(metrics_calls[0].1, 1280);
    assert_eq!(metrics_calls[0].2, 720);
    assert_eq!(metrics_calls[0].3, 1.0);
    assert!(!metrics_calls[0].4);
    drop(metrics_calls);

    let touch_calls = adapter.touch_calls.lock().await;
    assert_eq!(touch_calls.len(), 1);
    assert!(touch_calls[0].1);
}

#[tokio::test]
async fn configure_page_requires_adapter() {
    let runtime = StealthRuntime::new();
    runtime
        .load_catalog(ProfileCatalog {
            profiles: vec!["sp_generic".into()],
        })
        .await;
    runtime.load_bundle(sample_profile_bundle()).await;
    runtime.apply_stealth("https://example.com").await.unwrap();

    let result = runtime
        .configure_page(AdapterPageId::new(), "https://example.com")
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn captcha_decision_defaults_to_manual() {
    let runtime = StealthRuntime::new();
    let decision = runtime
        .decide_captcha(&stealth::CaptchaChallenge {
            id: "cc1".into(),
            origin: "https://example.com".into(),
            kind: stealth::CaptchaKind::Checkbox,
        })
        .await
        .unwrap();
    assert_eq!(
        decision.strategy as u8,
        stealth::DecisionStrategy::Manual as u8
    );
}

fn sample_profile_bundle() -> StealthProfileBundle {
    StealthProfileBundle {
        profiles: vec![StealthProfile {
            name: "sp_generic".into(),
            user_agent: "Mozilla/5.0".into(),
            accept_language: Some("en-US".into()),
            platform: Some("Win32".into()),
            locale: Some("en-US".into()),
            timezone: Some("America/Los_Angeles".into()),
            viewport: Some(Viewport {
                width: 1280,
                height: 720,
                device_scale_factor: 1.0,
                mobile: false,
            }),
            touch: true,
        }],
        tempos: Vec::new(),
        policy: None,
    }
}
