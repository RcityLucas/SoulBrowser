use std::collections::HashMap;

use crate::model::VectorItem;

use super::ann::AnnIndex;

/// Placeholder for a real HNSW implementation.
pub struct HnswStub {
    dim: usize,
    items: HashMap<String, VectorItem>,
}

impl HnswStub {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            items: HashMap::new(),
        }
    }
}

impl AnnIndex for HnswStub {
    fn insert(&mut self, item: VectorItem) {
        self.items.insert(item.id.clone(), item);
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<(VectorItem, f32)> {
        if query.is_empty() {
            return self
                .items
                .values()
                .take(top_k)
                .map(|item| (item.clone(), 0.0))
                .collect();
        }
        let mut scored = self
            .items
            .values()
            .map(|item| {
                let score = cosine_similarity(query, &item.embedding, self.dim);
                (item.clone(), score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    fn remove(&mut self, id: &str) {
        self.items.remove(id);
    }

    fn items(&self) -> Vec<VectorItem> {
        self.items.values().cloned().collect()
    }
}

fn cosine_similarity(query: &[f32], item: &[f32], dim: usize) -> f32 {
    let mut dot = 0.0;
    let mut norm_q = 0.0;
    let mut norm_i = 0.0;
    for i in 0..dim.min(query.len()).min(item.len()) {
        let q = query[i];
        let v = item[i];
        dot += q * v;
        norm_q += q * q;
        norm_i += v * v;
    }
    if norm_q == 0.0 || norm_i == 0.0 {
        return 0.0;
    }
    dot / (norm_q.sqrt() * norm_i.sqrt())
}
