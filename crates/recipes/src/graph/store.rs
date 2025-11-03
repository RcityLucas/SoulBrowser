use dashmap::DashMap;

#[derive(Default)]
pub struct GraphStore {
    pub edges: DashMap<(String, String), GraphEdge>,
}

impl GraphStore {
    pub fn add_edge(&self, from: String, to: String, edge: GraphEdge) {
        self.edges.insert((from, to), edge);
    }

    pub fn remove_edge(&self, from: &str, to: &str) {
        self.edges.remove(&(from.to_string(), to.to_string()));
    }

    pub fn neighbors(&self, node: &str) -> Vec<(String, GraphEdge)> {
        self.edges
            .iter()
            .filter(|entry| entry.key().0 == node)
            .map(|entry| (entry.key().1.clone(), entry.value().clone()))
            .collect()
    }

    pub fn snapshot(
        &self,
        filter: Option<&std::collections::HashSet<String>>,
    ) -> crate::model::GraphSnapshot {
        use crate::model::GraphRelation;
        let mut nodes = std::collections::HashSet::new();
        let mut edges = Vec::new();
        for entry in self.edges.iter() {
            let (from, to) = entry.key();
            if let Some(set) = filter {
                if !set.contains(from) || !set.contains(to) {
                    continue;
                }
            }
            nodes.insert(from.clone());
            nodes.insert(to.clone());
            let edge = entry.value();
            edges.push(GraphRelation {
                from: from.clone(),
                to: to.clone(),
                rel: edge.relation.clone(),
                weight: edge.weight,
            });
        }
        crate::model::GraphSnapshot {
            nodes: nodes.into_iter().collect(),
            edges,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub relation: String,
    pub weight: f32,
}
