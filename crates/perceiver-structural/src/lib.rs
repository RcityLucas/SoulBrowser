#![allow(dead_code)]

pub mod api;
pub mod cache;
pub mod differ;
pub mod errors;
pub mod events;
pub mod judges;
pub mod lifecycle;
pub mod metrics;
pub mod model;
pub mod policy;
pub mod ports;
pub mod reason;
pub mod redact;
pub mod resolver;
pub mod sampler;
pub mod structural;

pub use api::StructuralPerceiver;
pub use lifecycle::LifecycleWatcher;
pub use model::{
    AnchorDescriptor, AnchorGeometry, AnchorResolution, DiffFocus, DomAxDiff, DomAxSnapshot,
    InteractionAdvice, JudgeReport, ResolveHint, ResolveOpt, Scope, ScoreBreakdown, ScoreComponent,
    SelectorOrHint, SnapLevel, SnapshotId,
};
pub use policy::{
    CachePolicy, DiffPolicy, JudgePolicy, PerceiverPolicyView, ResolveOptions, ScoreWeights,
};
pub use ports::{AdapterPort, CdpPerceptionPort};
pub use structural::StructuralPerceiverImpl;
