use crate::codec::features::FeatureMap;

pub trait Embedder: Send + Sync {
    fn encode(&self, features: &FeatureMap) -> Vec<f32>;
}

#[derive(Default)]
pub struct RuleEmbedder {
    dim: usize,
}

impl RuleEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Embedder for RuleEmbedder {
    fn encode(&self, features: &FeatureMap) -> Vec<f32> {
        let mut vec = vec![0.0; self.dim.max(16)];
        for (idx, (_, weight)) in features.iter().enumerate() {
            let pos = idx % vec.len();
            vec[pos] += *weight;
        }
        vec
    }
}
