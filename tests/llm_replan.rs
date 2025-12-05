use std::sync::Arc;

use agent_core::{MockLlmProvider, PlanToFlowOptions, PlannerConfig};
use soulbrowser_cli::agent::executor::{DispatchRecord, StepExecutionReport};
use soulbrowser_cli::agent::{ChatRunner, FlowExecutionReport, StepExecutionStatus};
use soulbrowser_cli::replan::augment_request_for_replan;

struct MockFlowExecutor;

impl MockFlowExecutor {
    fn run(&self, attempt: u32) -> FlowExecutionReport {
        if attempt == 0 {
            FlowExecutionReport {
                success: false,
                steps: vec![StepExecutionReport {
                    step_id: "mock-step".into(),
                    title: "Mock action".into(),
                    status: StepExecutionStatus::Failed,
                    attempts: 1,
                    error: Some("synthetic failure".into()),
                    dispatches: Vec::<DispatchRecord>::new(),
                }],
            }
        } else {
            FlowExecutionReport {
                success: true,
                steps: vec![StepExecutionReport {
                    step_id: "mock-step".into(),
                    title: "Mock action".into(),
                    status: StepExecutionStatus::Success,
                    attempts: 1,
                    error: None,
                    dispatches: Vec::<DispatchRecord>::new(),
                }],
            }
        }
    }
}

#[tokio::test]
async fn llm_replan_loop_eventually_succeeds() {
    let provider = Arc::new(MockLlmProvider::default());
    let runner = ChatRunner::with_config(PlannerConfig::default(), PlanToFlowOptions::default())
        .with_llm_provider(provider);

    let agent_request = runner.request_from_prompt(
        "Complete a mock browser task".to_string(),
        None,
        vec!["stay deterministic".to_string()],
    );

    let mut session = runner.plan(agent_request.clone()).await.expect("plan");
    let executor = MockFlowExecutor;
    let mut exec_request = agent_request.clone();
    let mut attempt = 0u32;

    loop {
        let report = executor.run(attempt);
        if report.success {
            assert_eq!(attempt, 1, "replan should produce success on retry");
            break;
        }

        let (updated_request, failure_summary) =
            augment_request_for_replan(&exec_request, &report, attempt, None, None, None)
                .expect("augment");
        exec_request = updated_request;

        let replanned = runner
            .replan(exec_request.clone(), &session.plan, &failure_summary)
            .await
            .expect("replan succeed");
        assert!(
            replanned
                .plan
                .steps
                .iter()
                .any(|step| step.metadata.contains_key("replan_reason")),
            "replanned steps should describe the reason"
        );

        session = replanned;
        attempt += 1;
        assert!(attempt < 3, "mock should not loop forever");
    }
}
