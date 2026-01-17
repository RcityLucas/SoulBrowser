use crate::context::RedactCtx;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[cfg(test)]
use std::cell::RefCell;

#[cfg(not(test))]
use once_cell::sync::OnceCell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HashAlg {
    Sha256,
    HmacSha256,
}

impl Default for HashAlg {
    fn default() -> Self {
        HashAlg::Sha256
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ShotMode {
    Mask,
    Blur,
}

impl Default for ShotMode {
    fn default() -> Self {
        ShotMode::Mask
    }
}

impl ShotMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShotMode::Mask => "mask",
            ShotMode::Blur => "blur",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ShotRule {
    pub name: String,
    pub selector: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PiiRule {
    pub name: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyPolicyView {
    pub enable: bool,
    pub message_max_len: usize,
    pub text_hash_alg: HashAlg,
    pub allow_regex: bool,
    pub pii_patterns: Vec<PiiRule>,
    pub origin_host_only: bool,
    pub query_allow_keys: Vec<String>,
    pub attrs_whitelist: Vec<String>,
    pub screenshot_enable: bool,
    pub screenshot_mode: ShotMode,
    pub blur_radius: u32,
    pub mask_color: String,
    pub expand_px: u32,
    pub screenshot_rules: Vec<ShotRule>,
    pub ban_labels: Vec<String>,
    pub label_max_len: usize,
    pub jsonl_max_line_bytes: usize,
    pub add_debug_watermark: bool,
    pub hmac_rotate_epoch_days: Option<u32>,
}

impl Default for PrivacyPolicyView {
    fn default() -> Self {
        Self {
            enable: false,
            message_max_len: 512,
            text_hash_alg: HashAlg::Sha256,
            allow_regex: false,
            pii_patterns: vec![
                PiiRule {
                    name: "email".into(),
                    pattern: r"(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}".into(),
                },
                PiiRule {
                    name: "phone".into(),
                    pattern: r"\b(?:1[3-9]\d{9}|\d{3}[- ]?\d{4}[- ]?\d{4})\b".into(),
                },
                PiiRule {
                    name: "id_card".into(),
                    pattern: r"\b\d{6}(19|20)\d{2}(0[1-9]|1[0-2])(0[1-9]|[12]\d|3[01])\d{3}[0-9Xx]\b".into(),
                },
                PiiRule {
                    name: "credit_card".into(),
                    pattern: r"\b(?:4[0-9]{12}(?:[0-9]{3})?|5[1-5][0-9]{14}|3[47][0-9]{13}|6(?:011|5[0-9]{2})[0-9]{12})\b".into(),
                },
                PiiRule {
                    name: "secret_token".into(),
                    pattern: r"(sk|pk|tok)_[A-Za-z0-9]{16,}".into(),
                },
            ],
            origin_host_only: true,
            query_allow_keys: vec![],
            attrs_whitelist: vec!["href".into(), "src".into(), "alt".into(), "title".into()],
            screenshot_enable: false,
            screenshot_mode: ShotMode::Mask,
            blur_radius: 12,
            mask_color: "#111111CC".into(),
            expand_px: 6,
            screenshot_rules: vec![],
            ban_labels: vec![
                "user".into(),
                "email".into(),
                "phone".into(),
                "full_url".into(),
            ],
            label_max_len: 64,
            jsonl_max_line_bytes: 16 * 1024,
            add_debug_watermark: false,
            hmac_rotate_epoch_days: None,
        }
    }
}

#[cfg(not(test))]
static PRIVACY_POLICY: OnceCell<Arc<RwLock<PrivacyPolicyView>>> = OnceCell::new();

#[cfg(test)]
thread_local! {
    static TEST_POLICY: RefCell<Arc<RwLock<PrivacyPolicyView>>> =
        RefCell::new(Arc::new(RwLock::new(PrivacyPolicyView::default())));
}

fn policy_cell() -> Arc<RwLock<PrivacyPolicyView>> {
    #[cfg(test)]
    {
        return TEST_POLICY.with(|cell| Arc::clone(&cell.borrow()));
    }

    #[cfg(not(test))]
    {
        PRIVACY_POLICY
            .get_or_init(|| Arc::new(RwLock::new(PrivacyPolicyView::default())))
            .clone()
    }
}

#[derive(Clone)]
pub struct PrivacyPolicyHandle {
    inner: Arc<RwLock<PrivacyPolicyView>>,
}

impl PrivacyPolicyHandle {
    pub fn global() -> Self {
        Self {
            inner: policy_cell(),
        }
    }

    pub fn snapshot(&self) -> PrivacyPolicyView {
        self.inner.read().clone()
    }

    pub fn update(&self, view: PrivacyPolicyView) {
        *self.inner.write() = view;
    }

    pub fn enabled_for(&self, ctx: &RedactCtx) -> bool {
        let policy = self.snapshot();
        policy.enable
            && (ctx.export
                || ctx.tag_matches("pii_risk=high")
                || ctx.tag_matches("scenario=checkout")
                || ctx.tag_matches("tool=type"))
    }
}

pub fn set_policy(view: PrivacyPolicyView) {
    PrivacyPolicyHandle::global().update(view);
}

pub fn current_policy() -> PrivacyPolicyView {
    PrivacyPolicyHandle::global().snapshot()
}
