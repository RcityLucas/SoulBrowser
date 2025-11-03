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

    pub fn dim(&self) -> usize {
        self.dim.max(16)
    }
}

impl Embedder for RuleEmbedder {
    fn encode(&self, features: &FeatureMap) -> Vec<f32> {
        let dim = self.dim();
        let mut vec = vec![0.0; dim];
        for (idx, (_, weight)) in features.iter().enumerate() {
            let pos = idx % dim;
            vec[pos] += *weight;
        }
        vec
    }
}
