use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::SnapLevel;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapPolicyView {
    pub enabled: bool,
    pub struct_cfg: StructCfg,
    pub pixel_cfg: PixelCfg,
    pub io: IoCfg,
    #[serde(default)]
    pub maintenance: MaintenanceCfg,
}

impl Default for SnapPolicyView {
    fn default() -> Self {
        Self {
            enabled: true,
            struct_cfg: StructCfg::default(),
            pixel_cfg: PixelCfg::default(),
            io: IoCfg::default(),
            maintenance: MaintenanceCfg::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructCfg {
    pub level_default: SnapLevel,
    pub ttl_sec: u64,
    pub max_bytes_total: u64,
    pub field_whitelist: Vec<String>,
    pub mask_text: bool,
    pub max_text_len: usize,
    pub compress: bool,
    #[serde(default)]
    pub compress_level: i32,
    #[serde(default)]
    pub mask_secret_fields: Vec<String>,
}

impl Default for StructCfg {
    fn default() -> Self {
        Self {
            level_default: SnapLevel::Light,
            ttl_sec: 600,
            max_bytes_total: 512 * 1024 * 1024,
            field_whitelist: vec![
                "id".into(),
                "class".into(),
                "role".into(),
                "name".into(),
                "value".into(),
                "disabled".into(),
                "bbox".into(),
            ],
            mask_text: true,
            max_text_len: 256,
            compress: true,
            compress_level: 3,
            mask_secret_fields: vec!["password".into(), "token".into(), "secret".into()],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixelCfg {
    pub ttl_sec: u64,
    pub max_bytes_total: u64,
    pub max_clip_area: u32,
    pub max_bytes_per_clip: usize,
    pub max_clips_per_action: u8,
    pub forbid_fullpage: bool,
    pub encode: PixEncode,
    #[serde(default)]
    pub compress: bool,
}

impl Default for PixelCfg {
    fn default() -> Self {
        Self {
            ttl_sec: 300,
            max_bytes_total: 256 * 1024 * 1024,
            max_clip_area: 180_000,
            max_bytes_per_clip: 16 * 1024,
            max_clips_per_action: 3,
            forbid_fullpage: true,
            encode: PixEncode::default(),
            compress: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PixEncode {
    pub prefer: String,
    pub quality: u8,
}

impl Default for PixEncode {
    fn default() -> Self {
        Self {
            prefer: "jpeg".into(),
            quality: 80,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IoCfg {
    pub root: PathBuf,
    pub chunk_size: usize,
}

impl Default for IoCfg {
    fn default() -> Self {
        Self {
            root: PathBuf::from("./snapshots"),
            chunk_size: 1 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaintenanceCfg {
    pub sweep_interval_sec: u64,
    pub integrity_on_boot: bool,
    pub warn_on_orphan: bool,
    #[serde(default)]
    pub fallback_read_only: bool,
}

impl Default for MaintenanceCfg {
    fn default() -> Self {
        Self {
            sweep_interval_sec: 300,
            integrity_on_boot: true,
            warn_on_orphan: true,
            fallback_read_only: true,
        }
    }
}
