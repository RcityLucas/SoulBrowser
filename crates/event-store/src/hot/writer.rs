use std::sync::Arc;

use crate::config::HotCfg;
use crate::hot::rings::HotRings;
use crate::model::EventEnvelope;

pub struct HotWriter {
    rings: Arc<HotRings>,
}

impl HotWriter {
    pub fn new(cfg: HotCfg) -> Self {
        Self {
            rings: Arc::new(HotRings::new(cfg)),
        }
    }

    pub fn rings(&self) -> Arc<HotRings> {
        Arc::clone(&self.rings)
    }

    pub fn write(&self, event: EventEnvelope) {
        self.rings.write(event);
    }
}
