use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::model::SnapCtx;
use crate::policy::IoCfg;
use soulbrowser_core_types::ActionId;

pub fn struct_path(cfg: &IoCfg, ctx: &SnapCtx, hash: &str) -> PathBuf {
    let mut path = base_path(cfg, ctx.ts_wall);
    path.push("struct");
    path.push(format!("ss_{hash}.bin"));
    path
}

pub fn pix_path(cfg: &IoCfg, ctx: &SnapCtx, hash: &str) -> PathBuf {
    let mut path = base_path(cfg, ctx.ts_wall);
    path.push("pixel");
    path.push(format!("px_{hash}.bin"));
    path
}

pub fn action_index_path(cfg: &IoCfg, action: &ActionId) -> PathBuf {
    let mut path = cfg.root.join("index/action");
    path.push(format!("{}.json", action.0));
    path
}

fn base_path(cfg: &IoCfg, ts: DateTime<Utc>) -> PathBuf {
    let mut path = cfg.root.clone();
    path.push(ts.format("%Y/%m/%d/%H").to_string());
    path
}
