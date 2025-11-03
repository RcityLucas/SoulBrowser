use std::collections::HashMap;

use serde_json::Value;

use soulbrowser_snapshot_store::StructSnap;

use crate::model::{RecContext, RecQuery, Recipe, Status};

pub type FeatureMap = HashMap<String, f32>;

pub fn from_struct_snap(snap: &StructSnap) -> FeatureMap {
    let mut map = FeatureMap::new();
    map.insert(format!("level::{:?}", snap.level), 1.0);
    map.insert("kind::domax".into(), 1.0);
    map
}

pub fn from_observation(payload: &Value) -> FeatureMap {
    let mut map = FeatureMap::new();
    if let Some(tool) = payload.get("tool").and_then(|v| v.as_str()) {
        map.insert(format!("tool::{tool}"), 1.0);
    }
    map
}

pub fn from_context(ctx: &RecContext) -> FeatureMap {
    let mut map = FeatureMap::new();
    map.insert(format!("origin::{}", ctx.origin), 1.0);
    if let Some(path) = &ctx.path {
        map.insert(format!("path::{path}"), 1.0);
    }
    map.insert(format!("primitive::{}", ctx.primitive), 1.0);
    if let Some(intent) = &ctx.intent {
        map.insert(format!("intent::{intent}"), 1.0);
    }
    if let Some(anchor) = &ctx.anchor_fingerprint {
        map.insert(format!("anchor::{anchor}"), 1.0);
    }
    if let Some(struct_id) = &ctx.struct_id {
        map.insert(format!("struct::{struct_id}"), 0.5);
    }
    for pix in &ctx.pix_ids {
        map.insert(format!("pix::{pix}"), 0.3);
    }
    map
}

pub fn from_query(query: &RecQuery) -> FeatureMap {
    let mut map = FeatureMap::new();
    map.insert(format!("origin::{}", query.origin), 1.0);
    if let Some(path) = &query.path {
        map.insert(format!("path::{path}"), 1.0);
    }
    if let Some(ax_role) = &query.ax_role {
        map.insert(format!("ax::{ax_role}"), 1.0);
    }
    if let Some(text) = &query.text_hint {
        map.insert(format!("text::{text}"), 1.0);
    }
    if let Some(css) = &query.css_hint {
        map.insert(format!("css::{css}"), 1.0);
    }
    if let Some(intent) = &query.intent {
        map.insert(format!("intent::{intent}"), 1.0);
    }
    map
}

pub fn from_recipe(recipe: &Recipe) -> FeatureMap {
    let mut map = FeatureMap::new();
    map.insert(format!("origin::{}", recipe.scope.origin), 1.0);
    if let Some(path) = &recipe.scope.path_pat {
        map.insert(format!("path::{path}"), 1.0);
    }
    if let Some(intent) = recipe.labels.get("intent") {
        map.insert(format!("intent::{intent}"), 1.0);
    }
    if let Some(role) = &recipe.pre.ax_role {
        map.insert(format!("ax::{role}"), 1.0);
    }
    for locator in &recipe.strategy.locator_chain {
        map.entry(format!("locator::{locator}"))
            .and_modify(|w| *w += 1.0)
            .or_insert(1.0);
    }
    if Status::Active == recipe.status {
        map.insert("status::active".into(), 1.0);
    }
    map
}
