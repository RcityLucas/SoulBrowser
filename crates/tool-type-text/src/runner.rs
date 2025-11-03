use std::time::Instant;

use perceiver_structural::AnchorDescriptor;
use soulbrowser_core_types::{ExecRoute, SoulError};
use tracing::instrument;

use crate::errors::TypeTextError;
use crate::model::{
    ActionReport, ClearConfig, ClearMethod, DomDigest, ExecCtx, FieldSnapshot, InputMode,
    NetDigest, PostSignals, SelfHeal, TextOpt, TextParams, ValueDigest, WaitTier,
};
use crate::policy::TypePolicyView;
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
    pub policy: &'a TypePolicyView,
}

#[instrument(skip_all, fields(action = %ctx.action_id.0))]
pub async fn execute(
    ctx: &ExecCtx,
    mut params: TextParams,
    opt: TextOpt,
    deps: RuntimeDeps<'_>,
) -> Result<ActionReport, SoulError> {
    if !deps.policy.enabled {
        return Err(TypeTextError::Disabled.into());
    }
    if params.text.len() > deps.policy.max_text_len {
        return Err(TypeTextError::TextTooLong(deps.policy.max_text_len).into());
    }
    if matches!(params.mode, InputMode::Paste) && !deps.policy.allow_paste {
        return Err(TypeTextError::PasteDenied.into());
    }

    deps.events
        .emit_started(&ctx.action_id, &params.anchor)
        .await;
    let mut report = ActionReport::new(Instant::now());

    let field = precheck::run_precheck(
        deps.struct_port,
        deps.cdp,
        &ctx.route,
        &params.anchor,
        &deps.policy.timeouts,
    )
    .await?;
    deps.events
        .emit_precheck(&ctx.action_id, &precheck_event(&field))
        .await;

    if !field.visible || !field.clickable {
        if let Some(locator) = deps.locator {
            if let Some(new_anchor) = try_heal(locator, ctx, &params.anchor, "precheck").await? {
                deps.metrics.record_self_heal(true);
                report.self_heal = Some(SelfHeal {
                    attempted: true,
                    reason: Some("precheck".into()),
                    used_anchor: Some(new_anchor.clone()),
                });
                params.anchor = new_anchor;
            } else {
                deps.metrics.record_self_heal(false);
                return Err(TypeTextError::Precheck("anchor not clickable".into()).into());
            }
        } else {
            deps.metrics.record_precheck_failure("clickable");
            return Err(TypeTextError::Precheck("anchor not clickable".into()).into());
        }
    }

    if field.readonly {
        return Err(TypeTextError::ReadOnly.into());
    }

    if let Some(maxlen) = field.maxlength {
        if params.text.len() > maxlen as usize {
            return Err(TypeTextError::TextTooLong(maxlen as usize).into());
        }
    }

    if let Some(enabled) = field.enabled {
        if !enabled {
            return Err(TypeTextError::DisabledField.into());
        }
    }

    if params.clear.enabled {
        perform_clear(deps.cdp, &ctx.route, &params.clear).await?;
    }

    input_text(
        deps.cdp,
        deps.tempo,
        &ctx.route,
        &params,
        &deps.policy,
        deps.metrics,
    )
    .await?;

    if params.submit {
        deps.cdp.key_submit(&ctx.route).await?;
    }

    let wait_tier = if matches!(opt.wait, WaitTier::Auto) {
        deps.policy.wait_default
    } else {
        opt.wait
    };
    wait::apply_wait(deps.cdp, &ctx.route, wait_tier, &deps.policy.timeouts).await?;

    let post = collect_post(&deps, &ctx.route, &params.anchor, field.password_like).await?;
    report.post_signals = post.clone();
    report.ok = true;
    deps.metrics.record_ok(report.latency_ms);
    deps.events
        .emit_finished(&ctx.action_id, &post.value, true)
        .await;
    Ok(report.finish(Instant::now()))
}

async fn perform_clear(
    cdp: &dyn CdpPort,
    route: &ExecRoute,
    clear: &ClearConfig,
) -> Result<(), SoulError> {
    if !clear.enabled {
        return Ok(());
    }
    if clear.method.contains(ClearMethod::SELECT_ALL_DELETE) {
        cdp.clear_select_all(route).await?;
    } else if clear.method.contains(ClearMethod::BACKSPACE) {
        cdp.clear_backspace(route, clear.max_backspace).await?;
    }
    Ok(())
}

async fn input_text(
    cdp: &dyn CdpPort,
    tempo: Option<&dyn TempoPort>,
    route: &ExecRoute,
    params: &TextParams,
    policy: &TypePolicyView,
    metrics: &dyn MetricsPort,
) -> Result<(), SoulError> {
    ensure_mode_allowed(policy, params.mode)?;
    metrics.record_mode(match params.mode {
        InputMode::Character => "character",
        InputMode::Instant => "instant",
        InputMode::Natural => "natural",
        InputMode::Paste => "paste",
    });
    match params.mode {
        InputMode::Character => cdp.keyboard_type(route, &params.text).await,
        InputMode::Instant => cdp.insert_text(route, &params.text).await,
        InputMode::Natural => {
            if let Some(tempo_port) = tempo {
                let plan = tempo_port
                    .build_plan(InputMode::Natural, &params.text)
                    .await?;
                tempo_port.run_plan(route, &plan).await
            } else {
                cdp.keyboard_type(route, &params.text).await
            }
        }
        InputMode::Paste => cdp.paste_text(route, &params.text).await,
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
    password_like: bool,
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
    let value = deps
        .network
        .value_digest(route, anchor)
        .await
        .unwrap_or(ValueDigest::default());
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

    let mut value_digest = value;
    if password_like {
        value_digest.hash_after = None;
    }

    Ok(PostSignals {
        dom,
        net,
        value: value_digest,
        url,
        title,
    })
}

fn precheck_event(snapshot: &FieldSnapshot) -> crate::ports::PrecheckEvent {
    crate::ports::PrecheckEvent {
        visible: snapshot.visible,
        clickable: snapshot.clickable,
        enabled: snapshot.enabled,
        readonly: snapshot.readonly,
    }
}

fn ensure_mode_allowed(policy: &TypePolicyView, mode: InputMode) -> Result<(), SoulError> {
    match mode {
        InputMode::Paste if !policy.allow_paste => Err(TypeTextError::PasteDenied.into()),
        _ => Ok(()),
    }
}
