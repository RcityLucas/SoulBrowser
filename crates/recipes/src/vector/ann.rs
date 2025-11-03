use std::collections::HashMap;

use crate::model::VectorItem;

pub trait AnnIndex: Send + Sync {
    fn insert(&mut self, item: VectorItem);
    fn search(&self, query: &[f32], top_k: usize) -> Vec<(VectorItem, f32)>;
    fn remove(&mut self, id: &str);
    fn items(&self) -> Vec<VectorItem>;
}

#[derive(Default)]
pub struct InMemoryAnn {
    dim: usize,
    store: HashMap<String, VectorItem>,
}

impl InMemoryAnn {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            store: HashMap::new(),
        }
    }
}

impl AnnIndex for InMemoryAnn {
    fn insert(&mut self, item: VectorItem) {
        let mut adjusted = item.clone();
        adjusted.embedding = pad_or_trim(&adjusted.embedding, self.dim);
        self.store.insert(adjusted.id.clone(), adjusted);
    }

    fn search(&self, query: &[f32], top_k: usize) -> Vec<(VectorItem, f32)> {
        if query.is_empty() {
            return self
                .store
                .values()
                .take(top_k)
                .map(|item| (item.clone(), 0.0))
                .collect();
        }
        let query = pad_or_trim(query, self.dim);
        let norm_q = l2(&query);
        if norm_q == 0.0 {
            return Vec::new();
        }
        let mut scored = self
            .store
            .values()
            .map(|item| {
                let embedding = pad_or_trim(&item.embedding, self.dim);
                let sim = cosine(&query, &embedding, norm_q);
                (item.clone(), sim)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    fn remove(&mut self, id: &str) {
        self.store.remove(id);
    }

    fn items(&self) -> Vec<VectorItem> {
        self.store.values().cloned().collect()
    }
}

fn l2(vec: &[f32]) -> f32 {
    vec.iter().map(|v| v * v).sum::<f32>().sqrt()
}

fn cosine(query: &[f32], item: &[f32], norm_q: f32) -> f32 {
    let norm_i = l2(item);
    if norm_i == 0.0 {
        return 0.0;
    }
    let len = query.len().min(item.len());
    let mut dot = 0.0;
    for i in 0..len {
        dot += query[i] * item[i];
    }
    dot / (norm_q * norm_i)
}

fn pad_or_trim(vec: &[f32], dim: usize) -> Vec<f32> {
    if dim == 0 {
        return Vec::new();
    }
    let mut out = vec.to_vec();
    if out.len() > dim {
        out.truncate(dim);
    } else if out.len() < dim {
        out.extend(std::iter::repeat(0.0).take(dim - out.len()));
    }
    out
}
