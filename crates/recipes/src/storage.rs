use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::model::{RecVersion, Recipe, RecipeId, RecipeVersion};

#[derive(Default)]
pub struct RecipeStore {
    inner: Arc<RwLock<HashMap<RecipeId, BTreeMap<u32, Recipe>>>>,
}

impl RecipeStore {
    pub fn insert(&self, recipe: Recipe) {
        let mut guard = self.inner.write();
        let entry = guard.entry(recipe.id.clone()).or_insert_with(BTreeMap::new);
        entry.insert(recipe.version.0, recipe);
    }

    pub fn latest(&self, id: &RecipeId) -> Option<Recipe> {
        let guard = self.inner.read();
        guard.get(id).and_then(|versions| {
            versions
                .iter()
                .next_back()
                .map(|(_, recipe)| recipe.clone())
        })
    }

    pub fn latest_all(&self) -> Vec<Recipe> {
        let guard = self.inner.read();
        guard
            .values()
            .filter_map(|versions| {
                versions
                    .iter()
                    .next_back()
                    .map(|(_, recipe)| recipe.clone())
            })
            .collect()
    }

    pub fn find_version(&self, ver: &RecVersion) -> Option<Recipe> {
        self.inner
            .read()
            .get(&ver.id)
            .and_then(|versions| versions.get(&ver.version.0).cloned())
    }

    pub fn next_version(&self, id: &RecipeId) -> RecipeVersion {
        let guard = self.inner.read();
        let next = guard
            .get(id)
            .and_then(|versions| versions.iter().next_back().map(|(version, _)| version + 1))
            .unwrap_or(1);
        RecipeVersion(next)
    }

    pub fn all_recipes(&self) -> Vec<Recipe> {
        let guard = self.inner.read();
        guard
            .values()
            .flat_map(|versions| versions.values().cloned())
            .collect()
    }

    pub fn clear(&self) {
        self.inner.write().clear();
    }

    pub fn replace_all(&self, recipes: Vec<Recipe>) {
        let mut guard = self.inner.write();
        guard.clear();
        for recipe in recipes {
            guard
                .entry(recipe.id.clone())
                .or_insert_with(BTreeMap::new)
                .insert(recipe.version.0, recipe);
        }
    }
    pub fn all_versions(&self, id: &RecipeId) -> Vec<Recipe> {
        let guard = self.inner.read();
        guard
            .get(id)
            .map(|versions| versions.values().cloned().collect())
            .unwrap_or_default()
    }
}
