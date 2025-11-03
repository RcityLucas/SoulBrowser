use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::instrument;

use crate::errors::ClickError;
use crate::model::{
    ActionReport, ClickOpt, ClickParams, DomDigest, ExecCtx, NetDigest, PostSignals,
    PrecheckSnapshot, SelfHeal, WaitTier,
};
use crate::policy::ClickPolicyView;
use crate::ports::{
    CdpPort, EventsPort, HealRequest, LocatorPort, MetricsPort, NetworkPort, StructPort, TempoPort,
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
    pub policy: &'a ClickPolicyView,
}

#[instrument(skip_all, fields(action = %ctx.action_id.0, button = ?params.button))]
pub async fn execute(
    ctx: &ExecCtx,
    mut params: ClickParams,
    opt: ClickOpt,
    deps: RuntimeDeps<'_>,
) -> Result<ActionReport, SoulError> {
    if !deps.policy.enabled {
        return Err(ClickError::Disabled.into());
    }
    if !deps.policy.allowed_buttons.contains(&params.button) {
        return Err(ClickError::ButtonNotAllowed.into());
    }
    if let Some((dx, dy)) = params.offset {
        if dx.abs() > deps.policy.max_offset_px || dy.abs() > deps.policy.max_offset_px {
            return Err(ClickError::OffsetOutOfRange.into());
        }
    }

    deps.events
        .emit_started(&ctx.action_id, &params.anchor)
        .await;
    let mut report = ActionReport::new(Instant::now());

    let precheck = precheck::run_precheck(
        deps.struct_port,
        deps.cdp,
        &ctx.route,
        &params.anchor,
        &deps.policy.timeouts,
    )
    .await?;
    deps.events
        .emit_precheck(&ctx.action_id, &precheck_event(&precheck))
        .await;

    let mut self_heal = None;
    if !precheck.visible || !precheck.clickable {
        if deps.policy.allow_self_heal {
            if let Some(locator) = deps.locator {
                if let Some(new_anchor) =
                    try_heal(locator, ctx, &params.anchor, "auto-precheck").await?
                {
                    deps.metrics.record_self_heal(true);
                    self_heal = Some(SelfHeal {
                        attempted: true,
                        reason: Some("auto-precheck".into()),
                        used_anchor: Some(new_anchor.clone()),
                    });
                    params.anchor = new_anchor;
                } else {
                    deps.metrics.record_self_heal(false);
                    self_heal = Some(SelfHeal {
                        attempted: true,
                        reason: Some("auto-precheck".into()),
                        used_anchor: None,
                    });
                }
            }
        } else {
            deps.metrics.record_precheck_failure("clickable");
            return Err(ClickError::Precheck("anchor not clickable".into()).into());
        }
    }

    report.precheck = Some(precheck.clone());
    if self_heal.is_some() {
        report.self_heal = self_heal.clone();
    }

    let coords = deps
        .cdp
        .element_center(&ctx.route, &params.anchor)
        .await
        .map(|(x, y)| apply_offset((x, y), params.offset))?;

    if let Some(tempo) = deps.tempo {
        let plan = tempo.prepare(&ctx.route, &params.anchor).await?;
        tempo.apply(&plan).await?;
    }

    deps.cdp
        .dispatch_click(
            &ctx.route,
            coords,
            params.button,
            params.click_count,
            params.modifiers.bits(),
        )
        .await?;

    let wait_tier = if matches!(opt.wait, WaitTier::Auto) {
        deps.policy.wait_default
    } else {
        opt.wait
    };
    let wait_result =
        wait::apply_wait(deps.cdp, &ctx.route, wait_tier, &deps.policy.timeouts).await;
    if let Err(err) = wait_result {
        deps.metrics.record_fail("wait");
        report.error = Some(err.clone());
        deps.events
            .emit_finished(&ctx.action_id, &PostSignals::default(), false, Some(&err))
            .await;
        return Err(err);
    }

    let post = collect_post(&deps, &ctx.route, &params.anchor).await?;
    report.post_signals = post.clone();
    report.ok = true;
    deps.metrics.record_ok(report.latency_ms);
    deps.events
        .emit_finished(&ctx.action_id, &post, true, None)
        .await;
    Ok(report.finish(Instant::now()))
}

fn apply_offset(coords: (i32, i32), offset: Option<(i32, i32)>) -> (i32, i32) {
    if let Some((dx, dy)) = offset {
        (coords.0 + dx, coords.1 + dy)
    } else {
        coords
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
) -> Result<PostSignals, SoulError> {
    let dom = deps
        .struct_port
        .local_diff(route, anchor)
        .await
        .unwrap_or(DomDigest::default());
    let net = deps
        .network
        .window_digest(route)
        .await
        .unwrap_or(NetDigest::default());
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
    Ok(PostSignals::merge(dom, net, url, title))
}

fn precheck_event(snapshot: &PrecheckSnapshot) -> crate::ports::PrecheckEvent {
    crate::ports::PrecheckEvent {
        visible: snapshot.visible,
        clickable: snapshot.clickable,
        enabled: snapshot.enabled,
    }
}
