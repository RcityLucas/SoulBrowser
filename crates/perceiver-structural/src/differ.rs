use crate::model::{DomAxDiff, DomAxSnapshot};

pub fn diff(_base: &DomAxSnapshot, _current: &DomAxSnapshot) -> DomAxDiff {
    DomAxDiff {
        changes: Vec::new(),
    }
}
