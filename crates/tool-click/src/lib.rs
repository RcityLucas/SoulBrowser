#![allow(dead_code)]

pub mod api;
pub mod errors;
pub mod model;
pub mod policy;
pub mod ports;

mod events;
mod metrics;
mod precheck;
mod redact;
mod runner;
mod tempo;
mod wait;

pub use api::{ClickTool, ClickToolBuilder};
pub use model::{ActionReport, ClickOpt, ClickParams, ExecCtx, WaitTier};
