pub mod fetch;
pub mod plan;

pub use fetch::{describe_source, merge_outcome, run_fetch, FetchOutcome};
pub use plan::{build_plan, FetchPlan, PlanSource};
