use crate::model::AnchorDescriptor;

pub fn select_top(candidates: Vec<AnchorDescriptor>) -> Option<AnchorDescriptor> {
    candidates.into_iter().next()
}
