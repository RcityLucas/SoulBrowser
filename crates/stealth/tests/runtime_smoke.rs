use stealth::{ProfileCatalog, StealthControl, StealthRuntime};

#[tokio::test]
async fn apply_and_retrieve_profile() {
    let runtime = StealthRuntime::new();
    runtime
        .load_catalog(ProfileCatalog {
            profiles: vec!["sp_generic".into()],
        })
        .await;

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
