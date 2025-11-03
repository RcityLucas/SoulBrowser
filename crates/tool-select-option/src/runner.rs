use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::{instrument, warn};

use crate::errors::SelectError;
use crate::model::{
    ActionReport, ExecCtx, FieldSnapshot, MatchKind, PostSignals, SelectMode, SelectOpt,
    SelectParams, SelectionDigest, SelfHeal, WaitTier,
};
use crate::policy::SelectPolicyView;
use crate::ports::{
    match_kind_label, CdpPort, EventsPort, HealRequest, LocatorPort, MetricsPort, NetworkPort,
    PostEventPayload, SelectionState, StructPort, TempoPort,
};
use crate::{precheck, redact, wait};

pub struct RuntimeDeps<'a> {
    pub cdp: &'a dyn CdpPort,
    pub struct_port: &'a dyn StructPort,
    pub network: &'a dyn NetworkPort,
    pub locator: Option<&'a dyn LocatorPort>,
    pub events: &'a dyn EventsPort,
    pub metrics: &'a dyn MetricsPort,
    pub tempo: Option<&'a dyn TempoPort>,
    pub policy: &'a SelectPolicyView,
}

#[instrument(skip_all, fields(action = %ctx.action_id.0, mode = ?params.mode, match_kind = ?params.match_kind))]
pub async fn execute(
    ctx: &ExecCtx,
    mut params: SelectParams,
    opt: SelectOpt,
    deps: RuntimeDeps<'_>,
) -> Result<ActionReport, SoulError> {
    if !deps.policy.enabled {
        return Err(SelectError::Disabled.into());
    }
    if !deps.policy.allowed_modes.contains(&params.mode) {
        return Err(SelectError::ModeNotAllowed.into());
    }

    validate_target(&params)?;

    deps.metrics.record_mode(mode_label(params.mode));
    deps.metrics
        .record_match_kind(match_kind_label(params.match_kind));

    deps.events
        .emit_started(&ctx.action_id, &params.control_anchor)
        .await;
    let mut report = ActionReport::new(Instant::now());

    let (field, heal) = ensure_precheck(ctx, &mut params, &deps).await?;
    report.precheck = Some(field.clone());
    if let Some(heal) = heal.clone() {
        report.self_heal = Some(heal);
    }

    if field.readonly {
        deps.metrics.record_fail("readonly");
        return Err(SelectError::ReadOnly.into());
    }
    if let Some(false) = field.enabled {
        deps.metrics.record_precheck_failure("enabled");
        return Err(SelectError::DisabledField.into());
    }

    let before_state = deps
        .struct_port
        .selection_state(&ctx.route, &params.control_anchor)
        .await
        .ok();

    if let Some(tempo) = deps.tempo {
        let plan = tempo
            .plan(&ctx.route, &params.control_anchor, params.mode)
            .await?;
        tempo.apply(&plan).await?;
    }

    if deps.policy.timeouts.selection().is_zero() {
        warn!("selection timeout is zero; continuing without delay bound");
    }

    if let Err(err) = deps.cdp.select_option(&ctx.route, &params).await {
        deps.metrics.record_fail("select");
        report.error = Some(err.to_string());
        deps.events
            .emit_finished(
                &ctx.action_id,
                &PostEventPayload::new(SelectionDigest::default()),
                false,
            )
            .await;
        return Err(err);
    }

    let wait_tier = if matches!(opt.wait, WaitTier::Auto) {
        deps.policy.wait_default
    } else {
        opt.wait
    };
    if let Err(err) = wait::apply_wait(deps.cdp, &ctx.route, wait_tier, &deps.policy.timeouts).await
    {
        deps.metrics.record_fail("wait");
        report.error = Some(err.to_string());
        deps.events
            .emit_finished(
                &ctx.action_id,
                &PostEventPayload::new(SelectionDigest::default()),
                false,
            )
            .await;
        return Err(err);
    }

    let (post, after_state) =
        collect_post(&deps, &ctx.route, &params.control_anchor, before_state).await?;
    validate_post_selection(&params, &after_state)?;
    report.post_signals = post.clone();
    report.ok = true;
    deps.metrics.record_ok(report.latency_ms);
    deps.events
        .emit_finished(
            &ctx.action_id,
            &PostEventPayload::new(post.selection.clone()),
            true,
        )
        .await;
    Ok(report.finish(Instant::now()))
}

async fn ensure_precheck(
    ctx: &ExecCtx,
    params: &mut SelectParams,
    deps: &RuntimeDeps<'_>,
) -> Result<(FieldSnapshot, Option<SelfHeal>), SoulError> {
    let field = precheck::run_precheck(
        deps.struct_port,
        deps.cdp,
        &ctx.route,
        &params.control_anchor,
        &deps.policy.timeouts,
    )
    .await?;
    deps.events
        .emit_precheck(&ctx.action_id, &precheck_event(&field))
        .await;

    if field.visible && field.clickable {
        return Ok((field, None));
    }

    if !deps.policy.allow_self_heal {
        deps.metrics.record_precheck_failure("clickable");
        return Err(SelectError::Precheck("anchor not clickable".into()).into());
    }

    let Some(locator) = deps.locator else {
        deps.metrics.record_fail("heal-missing");
        return Err(SelectError::SelfHealUnavailable.into());
    };

    match try_heal(locator, ctx, &params.control_anchor, "precheck").await? {
        Some(new_anchor) => {
            deps.metrics.record_self_heal(true);
            params.control_anchor = new_anchor.clone();
            let heal_snapshot = precheck::run_precheck(
                deps.struct_port,
                deps.cdp,
                &ctx.route,
                &params.control_anchor,
                &deps.policy.timeouts,
            )
            .await?;
            let heal = SelfHeal {
                attempted: true,
                reason: Some("precheck".into()),
                used_anchor: Some(new_anchor),
            };
            Ok((heal_snapshot, Some(heal)))
        }
        None => {
            deps.metrics.record_self_heal(false);
            Err(SelectError::Precheck("anchor not clickable".into()).into())
        }
    }
}

async fn try_heal(
    locator: &dyn LocatorPort,
    ctx: &ExecCtx,
    anchor: &AnchorDescriptor,
    reason: &str,
) -> Result<Option<AnchorDescriptor>, SoulError> {
    let outcome = locator
        .try_once(HealRequest {
            action_id: ctx.action_id.clone(),
            route: ctx.route.clone(),
            primary: anchor.clone(),
            reason: reason.to_string(),
        })
        .await?;
    Ok(outcome.used_anchor)
}

async fn collect_post(
    deps: &RuntimeDeps<'_>,
    route: &ExecRoute,
    anchor: &AnchorDescriptor,
    before: Option<SelectionState>,
) -> Result<(PostSignals, SelectionState), SoulError> {
    let dom = deps
        .struct_port
        .local_diff(route, anchor)
        .await
        .unwrap_or_default();
    let net = deps.network.window_digest(route).await.unwrap_or_default();
    let after_state = deps
        .struct_port
        .selection_state(route, anchor)
        .await
        .unwrap_or_default();
    let selection = build_selection_digest(before.as_ref(), &after_state);
    let url = deps
        .cdp
        .current_url(route)
        .await
        .ok()
        .map(|u| redact::url(&u));
    let title = deps
        .cdp
        .current_title(route)
        .await
        .ok()
        .map(|t| redact::title(&t, 128));

    Ok((
        PostSignals {
            dom,
            net,
            selection,
            url,
            title,
        },
        after_state,
    ))
}

fn build_selection_digest(
    before: Option<&SelectionState>,
    after: &SelectionState,
) -> SelectionDigest {
    let changed = match before {
        Some(prev) => {
            prev.selected_indices != after.selected_indices
                || prev.selected_values != after.selected_values
        }
        None => !after.selected_indices.is_empty(),
    };
    SelectionDigest {
        changed,
        selected_count: after.selected_indices.len(),
        selected_indices: after.selected_indices.clone(),
        selected_hash: redact::selection_hash(&after.selected_values),
    }
}

fn precheck_event(snapshot: &FieldSnapshot) -> crate::ports::PrecheckEvent {
    crate::ports::PrecheckEvent {
        visible: snapshot.visible,
        clickable: snapshot.clickable,
        enabled: snapshot.enabled,
        readonly: snapshot.readonly,
    }
}

fn mode_label(mode: SelectMode) -> &'static str {
    match mode {
        SelectMode::Single => "single",
        SelectMode::Multiple => "multiple",
        SelectMode::Toggle => "toggle",
    }
}

fn validate_target(params: &SelectParams) -> Result<(), SoulError> {
    match params.match_kind {
        MatchKind::Value | MatchKind::Label => {
            if params.item.trim().is_empty() {
                return Err(SelectError::InvalidTarget("empty item".into()).into());
            }
        }
        MatchKind::Index => {
            params.item.trim().parse::<u32>().map_err(|_| {
                SelectError::InvalidTarget("index must be a non-negative integer".into())
            })?;
        }
        MatchKind::Anchor => {
            if params.option_anchor.is_none() {
                return Err(SelectError::InvalidTarget("option anchor required".into()).into());
            }
        }
    }
    Ok(())
}

fn validate_post_selection(params: &SelectParams, state: &SelectionState) -> Result<(), SoulError> {
    if matches!(params.mode, SelectMode::Toggle) {
        return Ok(());
    }
    match params.match_kind {
        MatchKind::Value => {
            if !state.selected_values.iter().any(|val| val == &params.item) {
                return Err(SelectError::OptionMissing.into());
            }
        }
        MatchKind::Index => {
            let idx = params.item.trim().parse::<u32>().map_err(|_| {
                SelectError::InvalidTarget("index must be a non-negative integer".into())
            })?;
            if !state.selected_indices.contains(&idx) {
                return Err(SelectError::OptionMissing.into());
            }
        }
        MatchKind::Label | MatchKind::Anchor => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use soulbrowser_core_types::FrameId;

    #[test]
    fn digest_changes_detected() {
        let before = SelectionState {
            selected_indices: vec![1],
            selected_values: vec!["foo".into()],
        };
        let after = SelectionState {
            selected_indices: vec![2],
            selected_values: vec!["bar".into()],
        };
        let digest = build_selection_digest(Some(&before), &after);
        assert!(digest.changed);
        assert_eq!(digest.selected_count, 1);
        assert_eq!(digest.selected_indices, vec![2]);
        assert!(digest.selected_hash.is_some());
    }

    fn dummy_anchor() -> AnchorDescriptor {
        AnchorDescriptor {
            strategy: "css".into(),
            value: Value::Null,
            frame_id: FrameId::new(),
            confidence: 1.0,
            backend_node_id: None,
            geometry: None,
        }
    }

    #[test]
    fn validate_target_anchor_requires_option() {
        let mut params = SelectParams {
            control_anchor: dummy_anchor(),
            match_kind: MatchKind::Anchor,
            item: "".into(),
            option_anchor: None,
            mode: SelectMode::Single,
        };
        assert!(validate_target(&params).is_err());
        params.option_anchor = Some(dummy_anchor());
        assert!(validate_target(&params).is_ok());
    }

    #[test]
    fn validate_post_selection_value_checks_membership() {
        let params = SelectParams {
            control_anchor: dummy_anchor(),
            match_kind: MatchKind::Value,
            item: "foo".into(),
            option_anchor: None,
            mode: SelectMode::Single,
        };
        let state_ok = SelectionState {
            selected_indices: vec![0],
            selected_values: vec!["foo".into()],
        };
        assert!(validate_post_selection(&params, &state_ok).is_ok());
        let state_missing = SelectionState {
            selected_indices: vec![0],
            selected_values: vec!["bar".into()],
        };
        assert!(validate_post_selection(&params, &state_missing).is_err());
    }
}
