use std::path::PathBuf;

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;

use crate::model::{PixClip, SnapRef, StructSnap};

#[derive(Default)]
pub struct HotIndex {
    pub structs: DashMap<String, StructSnap>,
    pub clips: DashMap<String, PixClip>,
    pub actions: DashMap<String, SnapRef>,
    pub struct_refs: DashMap<String, usize>,
    pub clip_refs: DashMap<String, usize>,
    pub struct_paths: DashMap<String, PathBuf>,
    pub clip_paths: DashMap<String, PathBuf>,
}

impl HotIndex {
    pub fn upsert_struct(&self, snap: StructSnap, path: PathBuf) {
        let id = snap.id.clone();
        self.structs.insert(id.clone(), snap);
        self.struct_paths.insert(id.clone(), path);
        self.struct_refs.entry(id).or_insert(0);
    }

    pub fn upsert_clip(&self, clip: PixClip, path: PathBuf) {
        let id = clip.id.clone();
        self.clips.insert(id.clone(), clip);
        self.clip_paths.insert(id.clone(), path);
        self.clip_refs.entry(id).or_insert(0);
    }

    pub fn upsert_action(&self, action: SnapRef) {
        self.actions.insert(action.action.0.clone(), action);
    }

    pub fn struct_path(&self, id: &str) -> Option<PathBuf> {
        self.struct_paths.get(id).map(|entry| entry.clone())
    }

    pub fn clip_path(&self, id: &str) -> Option<PathBuf> {
        self.clip_paths.get(id).map(|entry| entry.clone())
    }

    pub fn inc_struct_ref(&self, id: &str) {
        match self.struct_refs.entry(id.to_string()) {
            Entry::Occupied(mut occ) => {
                *occ.get_mut() += 1;
            }
            Entry::Vacant(vac) => {
                vac.insert(1);
            }
        }
    }

    pub fn dec_struct_ref(&self, id: &str) -> usize {
        match self.struct_refs.entry(id.to_string()) {
            Entry::Occupied(mut occ) => {
                let count = occ.get_mut();
                if *count > 0 {
                    *count -= 1;
                }
                *count
            }
            Entry::Vacant(_) => 0,
        }
    }

    pub fn struct_ref_count(&self, id: &str) -> usize {
        self.struct_refs.get(id).map(|entry| *entry).unwrap_or(0)
    }

    pub fn inc_clip_ref(&self, id: &str) {
        match self.clip_refs.entry(id.to_string()) {
            Entry::Occupied(mut occ) => {
                *occ.get_mut() += 1;
            }
            Entry::Vacant(vac) => {
                vac.insert(1);
            }
        }
    }

    pub fn dec_clip_ref(&self, id: &str) -> usize {
        match self.clip_refs.entry(id.to_string()) {
            Entry::Occupied(mut occ) => {
                let count = occ.get_mut();
                if *count > 0 {
                    *count -= 1;
                }
                *count
            }
            Entry::Vacant(_) => 0,
        }
    }

    pub fn clip_ref_count(&self, id: &str) -> usize {
        self.clip_refs.get(id).map(|entry| *entry).unwrap_or(0)
    }

    pub fn remove_struct(&self, id: &str) -> Option<StructSnap> {
        self.struct_paths.remove(id);
        self.struct_refs.remove(id);
        self.structs.remove(id).map(|(_, snap)| snap)
    }

    pub fn remove_clip(&self, id: &str) -> Option<PixClip> {
        self.clip_paths.remove(id);
        self.clip_refs.remove(id);
        self.clips.remove(id).map(|(_, clip)| clip)
    }
}
