use agent_core::plan::{
    AgentLocator, AgentScrollTarget, AgentTool, AgentToolKind, AgentValidation, AgentWaitCondition,
};
use agent_core::WaitMode;

use super::{stage_overlay, StageStrategy, StrategyApplication, StrategyInput, StrategyStep};

#[derive(Debug, Default)]
pub struct AutoActStrategy;

impl AutoActStrategy {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StageStrategy for AutoActStrategy {
    fn id(&self) -> &'static str {
        "auto"
    }

    fn stage(&self) -> agent_core::planner::PlanStageKind {
        agent_core::planner::PlanStageKind::Act
    }

    fn apply(&self, input: &StrategyInput<'_>) -> Option<StrategyApplication> {
        if should_seed_baidu_search(input) {
            return Some(build_baidu_search_application(input));
        }

        Some(build_scroll_application(input))
    }
}

fn build_scroll_application(input: &StrategyInput<'_>) -> StrategyApplication {
    let detail = format!(
        "滚动页面以探索更多关于{}的交互元素",
        input.context.search_seed()
    );
    let tool = AgentTool {
        kind: AgentToolKind::Scroll {
            target: AgentScrollTarget::Pixels(720),
        },
        wait: WaitMode::DomReady,
        timeout_ms: Some(5_000),
    };
    let step = StrategyStep::new("探索可交互区域", tool).with_detail(detail);
    StrategyApplication {
        steps: vec![step],
        note: Some("自动追加滚动动作，确保存在 Act 阶段".to_string()),
        overlay: Some(stage_overlay(
            agent_core::planner::PlanStageKind::Act,
            "auto",
            "applied",
            "🕹️ 自动探索交互区域",
        )),
    }
}

fn build_baidu_search_application(input: &StrategyInput<'_>) -> StrategyApplication {
    let query = input.context.search_seed();
    let mut type_step = StrategyStep::new(
        "输入搜索关键词",
        AgentTool {
            kind: AgentToolKind::TypeText {
                locator: AgentLocator::Css("input#kw".to_string()),
                text: query.clone(),
                submit: false,
            },
            wait: WaitMode::DomReady,
            timeout_ms: Some(8_000),
        },
    )
    .with_detail(format!("在搜索框输入 {query}"));
    type_step.validations.push(AgentValidation {
        description: "确保搜索框可见".to_string(),
        condition: AgentWaitCondition::ElementVisible(AgentLocator::Css("input#kw".to_string())),
    });

    let mut click_step = StrategyStep::new(
        "提交搜索",
        AgentTool {
            kind: AgentToolKind::Click {
                locator: AgentLocator::Css("input#su".to_string()),
            },
            wait: WaitMode::Idle,
            timeout_ms: Some(8_000),
        },
    )
    .with_detail("点击百度一下提交");
    click_step.validations.push(AgentValidation {
        description: "等待结果区域出现".to_string(),
        condition: AgentWaitCondition::ElementVisible(AgentLocator::Css(
            "div#content_left".to_string(),
        )),
    });

    StrategyApplication {
        steps: vec![type_step, click_step],
        note: Some("自动填写并提交百度搜索".to_string()),
        overlay: Some(stage_overlay(
            agent_core::planner::PlanStageKind::Act,
            "auto",
            "applied",
            "🕹️ 自动提交百度搜索",
        )),
    }
}

fn should_seed_baidu_search(input: &StrategyInput<'_>) -> bool {
    input
        .context
        .best_known_url()
        .map(|url| url.contains("baidu.com"))
        .unwrap_or(false)
}
