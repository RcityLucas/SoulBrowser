use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use soulbrowser_core_types::ActionId;
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration as TokioDuration, MissedTickBehavior};

use crate::errors::{SnapErrKind, SnapError};
use crate::fs::reader as fs_reader;
use crate::guard;
use crate::hash::hash_bytes;
use crate::index::mem::HotIndex;
use crate::metrics::SnapMetrics;
use crate::model::{
    BindRequest, DomAxRaw, ImageBuf, PixClip, PixMeta, PixThumb, Rect, ReplayBundle, SnapCtx,
    SnapLevel, SnapRef, StructSnap, SweepStats,
};
use crate::policy::{MaintenanceCfg, PixelCfg, SnapPolicyView, StructCfg};

pub type SnapResult<T> = Result<T, SnapError>;

#[async_trait]
pub trait SnapshotStore: Send + Sync {
    async fn put_struct(
        &self,
        ctx: SnapCtx,
        level: SnapLevel,
        domax: DomAxRaw,
    ) -> SnapResult<String>;
    async fn put_clip(&self, ctx: SnapCtx, region: Rect, img: ImageBuf) -> SnapResult<String>;
    async fn bind_action(&self, req: BindRequest) -> SnapResult<()>;

    async fn get_struct(&self, id: &str) -> SnapResult<StructSnap>;
    async fn get_clip(&self, id: &str) -> SnapResult<PixClip>;
    async fn refs_by_action(&self, action: &ActionId) -> SnapResult<Option<SnapRef>>;
    async fn replay_minimal(&self, action: &ActionId) -> SnapResult<ReplayBundle>;

    async fn sweep(&self) -> SnapResult<SweepStats>;

    fn set_read_only(&self, read_only: bool);
    fn status(&self) -> SnapshotStatus;
}

#[derive(Clone)]
pub struct SnapshotStoreBuilder {
    policy: SnapPolicyView,
    metrics: SnapMetrics,
}

impl SnapshotStoreBuilder {
    pub fn new(policy: SnapPolicyView) -> Self {
        Self {
            policy,
            metrics: SnapMetrics::default(),
        }
    }

    pub fn with_metrics(mut self, metrics: SnapMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn build(self) -> Arc<dyn SnapshotStore> {
        let store = Arc::new(InMemorySnapshotStore::new(self.policy, self.metrics));
        if let Err(err) = store.init_from_disk() {
            eprintln!("[snapshot-store] bootstrap failed: {}", err.kind());
        }
        store.start_background_tasks();
        store
    }
}

pub struct InMemorySnapshotStore {
    policy: SnapPolicyView,
    metrics: SnapMetrics,
    hot: HotIndex,
    struct_bytes: AtomicU64,
    pixel_bytes: AtomicU64,
    struct_order: Mutex<VecDeque<String>>,
    clip_order: Mutex<VecDeque<String>>,
    integrity: Mutex<Option<IntegrityReport>>,
    background: Mutex<Option<JoinHandle<()>>>,
    read_only: AtomicBool,
}

impl InMemorySnapshotStore {
    pub fn new(policy: SnapPolicyView, metrics: SnapMetrics) -> Self {
        Self {
            policy,
            metrics,
            hot: HotIndex::default(),
            struct_bytes: AtomicU64::new(0),
            pixel_bytes: AtomicU64::new(0),
            struct_order: Mutex::new(VecDeque::new()),
            clip_order: Mutex::new(VecDeque::new()),
            integrity: Mutex::new(None),
            background: Mutex::new(None),
            read_only: AtomicBool::new(false),
        }
    }

    fn ensure_enabled(&self) -> SnapResult<()> {
        if !self.policy.enabled {
            return Err(SnapErrKind::Disabled.into());
        }
        if self.read_only.load(Ordering::Relaxed) {
            return Err(SnapErrKind::ReadOnly.into());
        }
        Ok(())
    }

    fn policy_struct(&self) -> &StructCfg {
        &self.policy.struct_cfg
    }

    fn policy_pixel(&self) -> &PixelCfg {
        &self.policy.pixel_cfg
    }

    fn policy_maintenance(&self) -> &MaintenanceCfg {
        &self.policy.maintenance
    }

    pub fn init_from_disk(self: &Arc<Self>) -> SnapResult<()> {
        let mut report = IntegrityReport::default();
        self.load_structs_from_disk(&mut report)?;
        self.load_clips_from_disk(&mut report)?;
        self.load_actions_from_disk(&mut report)?;
        if self.policy_maintenance().integrity_on_boot {
            self.publish_integrity_report(report);
        }
        Ok(())
    }

    fn load_structs_from_disk(&self, report: &mut IntegrityReport) -> SnapResult<()> {
        let root = self.policy.io.root.join("struct");
        if !root.exists() {
            return Ok(());
        }
        let mut files: Vec<PathBuf> = Vec::new();
        self.walk_files(&root, &mut files)?;
        let mut records: Vec<(PathBuf, StructSnap)> = Vec::new();
        for path in files {
            match fs_reader::read_struct(&path) {
                Ok(snap) => {
                    report.checked_structs += 1;
                    records.push((path, snap));
                }
                Err(err) => {
                    report.struct_read_errors += 1;
                    self.metrics
                        .record_warn(&format!("struct_read_failed:{}", path.to_string_lossy()));
                    eprintln!("[snapshot-store] failed to read struct {:?}: {}", path, err);
                }
            }
        }
        records.sort_by(|a, b| a.1.ts_wall.cmp(&b.1.ts_wall));
        for (path, snap) in records {
            if self.hot.structs.contains_key(&snap.id) {
                continue;
            }
            if !self.validate_struct(&snap, &path, report) {
                continue;
            }
            self.hot.upsert_struct(snap.clone(), path);
            self.add_struct_usage(&snap.id, snap.meta.bytes);
        }
        Ok(())
    }

    fn load_clips_from_disk(&self, report: &mut IntegrityReport) -> SnapResult<()> {
        let root = self.policy.io.root.join("pixel");
        if !root.exists() {
            return Ok(());
        }
        let mut files: Vec<PathBuf> = Vec::new();
        self.walk_files(&root, &mut files)?;
        let mut records: Vec<(PathBuf, PixClip)> = Vec::new();
        for path in files {
            match fs_reader::read_clip(&path) {
                Ok(clip) => {
                    report.checked_pix += 1;
                    records.push((path, clip));
                }
                Err(err) => {
                    report.clip_read_errors += 1;
                    self.metrics
                        .record_warn(&format!("clip_read_failed:{}", path.to_string_lossy()));
                    eprintln!("[snapshot-store] failed to read clip {:?}: {}", path, err);
                }
            }
        }
        records.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, clip) in records {
            if self.hot.clips.contains_key(&clip.id) {
                continue;
            }
            if !self.validate_clip(&clip, &path, report) {
                continue;
            }
            self.hot.upsert_clip(clip.clone(), path);
            self.add_clip_usage(&clip.id, clip.meta.bytes);
        }
        Ok(())
    }

    fn load_actions_from_disk(&self, report: &mut IntegrityReport) -> SnapResult<()> {
        let root = self.policy.io.root.join("index").join("action");
        if !root.exists() {
            return Ok(());
        }
        let mut files: Vec<PathBuf> = Vec::new();
        self.walk_files(&root, &mut files)?;
        for path in files {
            let snapshot = match fs_reader::read_action(&path) {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    report.action_read_errors += 1;
                    self.metrics
                        .record_warn(&format!("action_read_failed:{}", path.to_string_lossy()));
                    eprintln!("[snapshot-store] failed to read action {:?}: {}", path, err);
                    continue;
                }
            };
            if snapshot.ttl_at < Utc::now() {
                continue;
            }
            self.apply_binding_change(None, &snapshot);
            if let Some(struct_id) = &snapshot.struct_id {
                if !self.hot.structs.contains_key(struct_id) {
                    report.missing_struct_refs += 1;
                    if self.policy_maintenance().warn_on_orphan {
                        self.metrics
                            .record_warn(&format!("missing_struct_ref:{}", struct_id));
                    }
                }
            }
            for pix in &snapshot.pix_ids {
                if !self.hot.clips.contains_key(pix) {
                    report.missing_clip_refs += 1;
                    if self.policy_maintenance().warn_on_orphan {
                        self.metrics
                            .record_warn(&format!("missing_clip_ref:{}", pix));
                    }
                }
            }
            self.hot.upsert_action(snapshot);
        }
        Ok(())
    }

    fn walk_files(&self, root: &Path, out: &mut Vec<PathBuf>) -> SnapResult<()> {
        if !root.exists() {
            return Ok(());
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            let metadata = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(err) => {
                    eprintln!("[snapshot-store] metadata failed: {}", err);
                    continue;
                }
            };
            if metadata.is_dir() {
                let entries = std::fs::read_dir(&path)
                    .map_err(|err| SnapErrKind::IoFailed(err.to_string()))?;
                for entry in entries {
                    let entry = entry.map_err(|err| SnapErrKind::IoFailed(err.to_string()))?;
                    stack.push(entry.path());
                }
            } else {
                out.push(path);
            }
        }
        Ok(())
    }

    fn struct_total(&self) -> u64 {
        self.struct_bytes.load(Ordering::Relaxed)
    }

    fn pixel_total(&self) -> u64 {
        self.pixel_bytes.load(Ordering::Relaxed)
    }

    fn add_struct_usage(&self, id: &str, bytes: u64) {
        self.struct_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.struct_order.lock().push_back(id.to_string());
    }

    fn add_clip_usage(&self, id: &str, bytes: u64) {
        self.pixel_bytes.fetch_add(bytes, Ordering::Relaxed);
        self.clip_order.lock().push_back(id.to_string());
    }

    fn reduce_struct_usage(&self, id: &str, bytes: u64) {
        self.struct_bytes.fetch_sub(bytes, Ordering::Relaxed);
        self.struct_order.lock().retain(|entry| entry != id);
    }

    fn reduce_clip_usage(&self, id: &str, bytes: u64) {
        self.pixel_bytes.fetch_sub(bytes, Ordering::Relaxed);
        self.clip_order.lock().retain(|entry| entry != id);
    }

    fn apply_binding_change(&self, old: Option<SnapRef>, new: &SnapRef) {
        if let Some(old_ref) = old {
            if let Some(struct_id) = old_ref.struct_id {
                if self.hot.dec_struct_ref(&struct_id) == 0 {
                    self.drop_struct(&struct_id, None);
                }
            }
            for pix in old_ref.pix_ids {
                if self.hot.dec_clip_ref(&pix) == 0 {
                    self.drop_clip(&pix, None);
                }
            }
        }

        if let Some(struct_id) = &new.struct_id {
            self.hot.inc_struct_ref(struct_id);
        }
        for pix in &new.pix_ids {
            self.hot.inc_clip_ref(pix);
        }
    }

    fn release_binding(&self, snap: SnapRef, stats: &mut SweepStats) {
        if let Some(struct_id) = snap.struct_id {
            if self.hot.dec_struct_ref(&struct_id) == 0 {
                self.drop_struct(&struct_id, Some(stats));
            }
        }
        for pix in snap.pix_ids {
            if self.hot.dec_clip_ref(&pix) == 0 {
                self.drop_clip(&pix, Some(stats));
            }
        }
    }

    fn ensure_struct_capacity(&self, additional: u64) -> SnapResult<()> {
        let max = self.policy_struct().max_bytes_total;
        if max == 0 {
            return Err(SnapErrKind::QuotaExceeded.into());
        }
        loop {
            let current = self.struct_total();
            if current + additional <= max {
                return Ok(());
            }
            if !self.evict_struct_once()? {
                return Err(SnapErrKind::QuotaExceeded.into());
            }
        }
    }

    fn ensure_pixel_capacity(&self, additional: u64) -> SnapResult<()> {
        let max = self.policy_pixel().max_bytes_total;
        if max == 0 {
            return Err(SnapErrKind::QuotaExceeded.into());
        }
        loop {
            let current = self.pixel_total();
            if current + additional <= max {
                return Ok(());
            }
            if !self.evict_clip_once()? {
                return Err(SnapErrKind::QuotaExceeded.into());
            }
        }
    }

    fn validate_struct(
        &self,
        snap: &StructSnap,
        path: &Path,
        report: &mut IntegrityReport,
    ) -> bool {
        let dom = match &snap.dom_zstd {
            Some(bytes) => bytes.as_slice(),
            None => {
                report.struct_payload_missing += 1;
                self.metrics.record_warn("struct_missing_payload");
                eprintln!(
                    "[snapshot-store] missing dom payload for struct {} ({:?})",
                    snap.id, path
                );
                return false;
            }
        };
        let ax = match &snap.ax_zstd {
            Some(bytes) => bytes.as_slice(),
            None => {
                report.struct_payload_missing += 1;
                self.metrics.record_warn("struct_missing_payload");
                eprintln!(
                    "[snapshot-store] missing ax payload for struct {} ({:?})",
                    snap.id, path
                );
                return false;
            }
        };
        let expected = crate::codec::domax::content_hash(dom, ax);
        if expected != snap.id {
            report.struct_hash_mismatch += 1;
            self.metrics.record_warn("struct_hash_mismatch");
            eprintln!(
                "[snapshot-store] hash mismatch for struct {} (expected {}, path {:?})",
                snap.id, expected, path
            );
            return false;
        }
        let bytes = dom.len() + ax.len();
        if snap.meta.bytes != bytes as u64 {
            report.struct_size_mismatch += 1;
            self.metrics.record_warn("struct_size_mismatch");
        }
        true
    }

    fn publish_integrity_report(&self, report: IntegrityReport) {
        if report.has_issues() {
            let summary = report.summary();
            self.metrics.record_warn("integrity_report");
            eprintln!("[snapshot-store] integrity issues detected: {}", summary);
        }
        *self.integrity.lock() = Some(report);
    }

    fn handle_io_failure(&self, ctx: &str, err: &io::Error) {
        self.metrics.record_warn(&format!("io_failed:{}", ctx));
        if self.policy_maintenance().fallback_read_only {
            self.enter_read_only(&format!("{}: {}", ctx, err));
        }
    }

    fn enter_read_only(&self, reason: &str) {
        if !self.policy_maintenance().fallback_read_only {
            return;
        }
        let prev = self.read_only.swap(true, Ordering::Relaxed);
        if !prev {
            self.metrics
                .record_warn(&format!("read_only_enter: {}", reason));
            eprintln!("[snapshot-store] entering read-only mode due to {}", reason);
        }
    }

    pub fn start_background_tasks(self: &Arc<Self>) {
        let interval_sec = self.policy_maintenance().sweep_interval_sec;
        if interval_sec == 0 {
            return;
        }
        let mut guard = self.background.lock();
        if guard.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let period = TokioDuration::from_secs(interval_sec.max(60));
        let handle = tokio::spawn(async move {
            let mut ticker = interval(period);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let Some(store) = weak.upgrade() else {
                    break;
                };
                if !store.policy.enabled || store.read_only.load(Ordering::Relaxed) {
                    continue;
                }
                match store.sweep().await {
                    Ok(stats) => {
                        if stats.expired > 0 || stats.removed_struct > 0 || stats.removed_pix > 0 {
                            store
                                .metrics
                                .record_sweep(stats.removed_struct, stats.removed_pix);
                        }
                    }
                    Err(err) => store
                        .metrics
                        .record_warn(&format!("sweep_failed:{}", err.kind())),
                }
            }
        });
        *guard = Some(handle);
    }

    fn validate_clip(&self, clip: &PixClip, path: &Path, report: &mut IntegrityReport) -> bool {
        let expected = hash_bytes("px", &clip.thumb.bytes);
        if expected != clip.id {
            report.clip_hash_mismatch += 1;
            self.metrics.record_warn("clip_hash_mismatch");
            eprintln!(
                "[snapshot-store] hash mismatch for clip {} (expected {}, path {:?})",
                clip.id, expected, path
            );
            return false;
        }
        if clip.meta.bytes != clip.thumb.bytes.len() as u64 {
            report.clip_size_mismatch += 1;
            self.metrics.record_warn("clip_size_mismatch");
        }
        true
    }

    fn evict_struct_once(&self) -> SnapResult<bool> {
        let mut order = self.struct_order.lock();
        let len = order.len();
        for _ in 0..len {
            let Some(id) = order.pop_front() else {
                break;
            };
            if self.hot.struct_ref_count(&id) > 0 {
                order.push_back(id);
                continue;
            }
            let path = self.hot.struct_path(&id);
            if let Some(snap) = self.hot.remove_struct(&id) {
                if let Some(path) = path {
                    crate::fs::writer::remove_file(&path);
                }
                self.reduce_struct_usage(&id, snap.meta.bytes);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn evict_clip_once(&self) -> SnapResult<bool> {
        let mut order = self.clip_order.lock();
        let len = order.len();
        for _ in 0..len {
            let Some(id) = order.pop_front() else {
                break;
            };
            if self.hot.clip_ref_count(&id) > 0 {
                order.push_back(id);
                continue;
            }
            let path = self.hot.clip_path(&id);
            if let Some(clip) = self.hot.remove_clip(&id) {
                if let Some(path) = path {
                    crate::fs::writer::remove_file(&path);
                }
                self.reduce_clip_usage(&id, clip.meta.bytes);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn drop_struct(&self, id: &str, stats: Option<&mut SweepStats>) {
        let path = self.hot.struct_path(id);
        if let Some(snap) = self.hot.remove_struct(id) {
            if let Some(path) = path {
                crate::fs::writer::remove_file(&path);
            }
            self.reduce_struct_usage(id, snap.meta.bytes);
            if let Some(stats) = stats {
                stats.removed_struct += 1;
            }
        }
    }

    fn drop_clip(&self, id: &str, stats: Option<&mut SweepStats>) {
        let path = self.hot.clip_path(id);
        if let Some(clip) = self.hot.remove_clip(id) {
            if let Some(path) = path {
                crate::fs::writer::remove_file(&path);
            }
            self.reduce_clip_usage(id, clip.meta.bytes);
            if let Some(stats) = stats {
                stats.removed_pix += 1;
            }
        }
    }
}

impl Drop for InMemorySnapshotStore {
    fn drop(&mut self) {
        if let Some(handle) = self.background.get_mut().take() {
            handle.abort();
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SnapshotStatus {
    pub read_only: bool,
    pub latest_integrity: Option<String>,
}

#[derive(Clone, Default)]
struct IntegrityReport {
    checked_structs: usize,
    struct_read_errors: usize,
    struct_hash_mismatch: usize,
    struct_size_mismatch: usize,
    struct_payload_missing: usize,
    checked_pix: usize,
    clip_read_errors: usize,
    clip_hash_mismatch: usize,
    clip_size_mismatch: usize,
    action_read_errors: usize,
    missing_struct_refs: usize,
    missing_clip_refs: usize,
}

impl IntegrityReport {
    fn has_issues(&self) -> bool {
        self.struct_read_errors > 0
            || self.struct_hash_mismatch > 0
            || self.struct_size_mismatch > 0
            || self.struct_payload_missing > 0
            || self.clip_read_errors > 0
            || self.clip_hash_mismatch > 0
            || self.clip_size_mismatch > 0
            || self.action_read_errors > 0
            || self.missing_struct_refs > 0
            || self.missing_clip_refs > 0
    }

    fn summary(&self) -> String {
        format!(
            "structs_checked={} struct_errors={} struct_hash={} struct_size={} struct_payload={} pix_checked={} pix_errors={} pix_hash={} pix_size={} action_errors={} missing_struct_refs={} missing_clip_refs={}",
            self.checked_structs,
            self.struct_read_errors,
            self.struct_hash_mismatch,
            self.struct_size_mismatch,
            self.struct_payload_missing,
            self.checked_pix,
            self.clip_read_errors,
            self.clip_hash_mismatch,
            self.clip_size_mismatch,
            self.action_read_errors,
            self.missing_struct_refs,
            self.missing_clip_refs
        )
    }
}

#[async_trait]
impl SnapshotStore for InMemorySnapshotStore {
    async fn put_struct(
        &self,
        ctx: SnapCtx,
        level: SnapLevel,
        domax: DomAxRaw,
    ) -> SnapResult<String> {
        self.ensure_enabled()?;
        let (dom_bytes, ax_bytes, meta) = crate::codec::domax::encode(&domax, self.policy_struct());
        let hash = crate::codec::domax::content_hash(&dom_bytes, &ax_bytes);

        if self.hot.structs.contains_key(&hash) {
            return Ok(hash);
        }

        guard::check_struct(self.policy_struct(), meta.bytes as usize)?;
        self.ensure_struct_capacity(meta.bytes)?;

        let snap = StructSnap {
            id: hash.clone(),
            kind: "domax".into(),
            level,
            page: ctx.page.clone(),
            frame: ctx.frame.clone(),
            action: ctx.action.clone(),
            ts_wall: ctx.ts_wall,
            ts_mono: ctx.ts_mono,
            dom_zstd: Some(dom_bytes.clone()),
            ax_zstd: Some(ax_bytes.clone()),
            meta,
        };

        let path = match crate::fs::writer::write_struct(&self.policy.io, &ctx, &snap) {
            Ok(path) => path,
            Err(err) => {
                self.handle_io_failure("write_struct", &err);
                return Err(SnapErrKind::IoFailed(err.to_string()).into());
            }
        };

        self.hot.upsert_struct(snap.clone(), path);
        self.add_struct_usage(&hash, snap.meta.bytes);
        self.metrics
            .record_put_struct(snap.meta.bytes as usize, snap.meta.masked);
        Ok(hash)
    }

    async fn put_clip(&self, ctx: SnapCtx, region: Rect, img: ImageBuf) -> SnapResult<String> {
        self.ensure_enabled()?;
        let pixel_cfg = self.policy_pixel();
        let encode_cfg = &pixel_cfg.encode;
        let region_clone = region.clone();
        let result = crate::codec::pix::crop_scale_encode(
            &img,
            &region,
            &encode_cfg.prefer,
            encode_cfg.quality,
            self.policy_pixel().max_bytes_per_clip,
        )
        .ok_or_else(|| SnapErrKind::Oversize)?;
        guard::check_clip(self.policy_pixel(), &region, &img, result.thumb.bytes.len())?;
        let thumb_bytes = if pixel_cfg.compress {
            crate::codec::pix::compress_thumb(&result.thumb.bytes, pixel_cfg.encode.quality as i32)
                .unwrap_or(result.thumb.bytes.clone())
        } else {
            result.thumb.bytes.clone()
        };
        let hash = hash_bytes("px", &thumb_bytes);

        if self.hot.clips.contains_key(&hash) {
            return Ok(hash);
        }

        self.ensure_pixel_capacity(result.meta.bytes)?;

        let clip = PixClip {
            id: hash.clone(),
            page: ctx.page.clone(),
            frame: ctx.frame.clone(),
            action: ctx.action.clone(),
            rect: region_clone,
            thumb: PixThumb {
                bytes: thumb_bytes,
                ..result.thumb
            },
            meta: PixMeta {
                compression: if pixel_cfg.compress {
                    Some("zstd".into())
                } else {
                    None
                },
                ..result.meta
            },
            compressed: pixel_cfg.compress,
        };

        let path = match crate::fs::writer::write_pix(&self.policy.io, &ctx, &clip) {
            Ok(path) => path,
            Err(err) => {
                self.handle_io_failure("write_pix", &err);
                return Err(SnapErrKind::IoFailed(err.to_string()).into());
            }
        };

        self.hot.upsert_clip(clip.clone(), path);
        self.add_clip_usage(&hash, clip.meta.bytes);
        self.metrics.record_put_clip(clip.meta.bytes as usize);
        Ok(hash)
    }

    async fn bind_action(&self, req: BindRequest) -> SnapResult<()> {
        self.ensure_enabled()?;
        let ttl = req
            .ttl
            .unwrap_or_else(|| Duration::from_secs(self.policy_struct().ttl_sec));
        if ttl.as_secs()
            > self
                .policy_struct()
                .ttl_sec
                .max(self.policy_pixel().ttl_sec)
        {
            return Err(SnapErrKind::TtlTooLong.into());
        }

        if let Some(struct_id) = &req.struct_id {
            if !self.hot.structs.contains_key(struct_id) {
                return Err(SnapErrKind::NotFound.into());
            }
        }
        for pix_id in &req.pix_ids {
            if !self.hot.clips.contains_key(pix_id) {
                return Err(SnapErrKind::NotFound.into());
            }
        }

        let ttl_at = chrono::Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
        let snap = SnapRef {
            action: req.action.clone(),
            page: req.page.clone(),
            frame: req.frame.clone(),
            struct_id: req.struct_id.clone(),
            pix_ids: req.pix_ids.clone(),
            ttl_at,
        };

        let key = req.action.0.clone();
        let previous = self.hot.actions.get(&key).map(|entry| entry.clone());
        self.apply_binding_change(previous, &snap);
        self.hot.upsert_action(snap.clone());
        let data = serde_json::to_vec_pretty(&snap)
            .map_err(|err| SnapErrKind::IoFailed(err.to_string()))?;
        if let Err(err) = crate::fs::writer::write_action_index(&self.policy.io, &req.action, &data)
        {
            self.handle_io_failure("write_action_index", &err);
            return Err(SnapErrKind::IoFailed(err.to_string()).into());
        }
        self.metrics.record_bind();
        Ok(())
    }

    async fn get_struct(&self, id: &str) -> SnapResult<StructSnap> {
        self.hot
            .structs
            .get(id)
            .map(|entry| entry.clone())
            .ok_or_else(|| SnapErrKind::NotFound.into())
    }

    async fn get_clip(&self, id: &str) -> SnapResult<PixClip> {
        self.hot
            .clips
            .get(id)
            .map(|entry| entry.clone())
            .ok_or_else(|| SnapErrKind::NotFound.into())
    }

    async fn refs_by_action(&self, action: &ActionId) -> SnapResult<Option<SnapRef>> {
        Ok(self.hot.actions.get(&action.0).map(|entry| entry.clone()))
    }

    async fn replay_minimal(&self, action: &ActionId) -> SnapResult<ReplayBundle> {
        let refs = self.refs_by_action(action).await?;
        Ok(ReplayBundle {
            struct_id: refs.as_ref().and_then(|r| r.struct_id.clone()),
            pix_ids: refs.map(|r| r.pix_ids).unwrap_or_default(),
            summary: Some("snapshot evidence placeholder".into()),
        })
    }

    async fn sweep(&self) -> SnapResult<SweepStats> {
        self.ensure_enabled()?;
        let now = chrono::Utc::now();
        let mut expired = Vec::new();
        for entry in self.hot.actions.iter() {
            if entry.ttl_at <= now {
                expired.push(entry.action.0.clone());
            }
        }

        let mut stats = SweepStats::default();
        for key in expired {
            if let Some((_, snap)) = self.hot.actions.remove(&key) {
                self.release_binding(snap.clone(), &mut stats);
                let path = crate::fs::layout::action_index_path(&self.policy.io, &snap.action);
                crate::fs::writer::remove_file(&path);
                stats.expired += 1;
            }
        }
        Ok(stats)
    }

    fn set_read_only(&self, read_only: bool) {
        let prev = self.read_only.swap(read_only, Ordering::Relaxed);
        if read_only && !prev {
            self.metrics.record_warn("read_only_enabled");
        } else if !read_only && prev {
            self.metrics.record_warn("read_only_disabled");
        }
    }

    fn status(&self) -> SnapshotStatus {
        let summary = self
            .integrity
            .lock()
            .as_ref()
            .map(|report| report.summary());
        SnapshotStatus {
            read_only: self.read_only.load(Ordering::Relaxed),
            latest_integrity: summary,
        }
    }
}
