use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolveOptions {
    pub max_candidates: usize,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self { max_candidates: 1 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PerceiverPolicyView {
    pub resolve: ResolveOptions,
}
