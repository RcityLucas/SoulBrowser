use crate::model::{AnchorDescriptor, JudgeReport};

pub fn visible(_anchor: &AnchorDescriptor) -> JudgeReport {
    let mut facts = serde_json::Map::new();
    if let Some(geom) = &_anchor.geometry {
        facts.insert(
            "geometry".into(),
            serde_json::json!({
                "x": geom.x,
                "y": geom.y,
                "width": geom.width,
                "height": geom.height,
            }),
        );
    }
    JudgeReport {
        ok: true,
        reason: "geometry-check".into(),
        facts: serde_json::Value::Object(facts),
    }
}

pub fn clickable(anchor: &AnchorDescriptor) -> JudgeReport {
    let mut report = visible(anchor);
    report.reason = "stub clickable".into();
    report
}

pub fn enabled(anchor: &AnchorDescriptor) -> JudgeReport {
    let mut report = visible(anchor);
    report.reason = "stub enabled".into();
    report
}
