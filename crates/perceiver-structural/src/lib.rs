#![allow(dead_code)]

pub mod api;
pub mod cache;
pub mod differ;
pub mod errors;
pub mod events;
pub mod judges;
pub mod model;
pub mod policy;
pub mod ports;
pub mod resolver;
pub mod sampler;
pub mod structural;

pub use api::StructuralPerceiver;
pub use model::{
    AnchorDescriptor, AnchorGeometry, AnchorResolution, DomAxDiff, DomAxSnapshot, JudgeReport,
    ResolveHint,
};
pub use policy::ResolveOptions;
pub use ports::{AdapterPort, CdpPerceptionPort};
pub use structural::StructuralPerceiverImpl;
