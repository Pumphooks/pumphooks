use hook_types::{
    Capabilities as C, HookCallback, HookContext, HookDecision, HookSlot, MarketStats, ModuleId,
    Side, UserState, MAX_HOOK_SLOTS, MAX_POLICIES, MAX_RUNTIME_HOOKS, MAX_TRACE,
};
use policy_vm::{EvalInput, PolicyData};

use crate::modules::{
    holder_xp::HolderXp, launch_shield::LaunchShield, max_wallet::MaxWallet, sell_limit::SellLimit,
    sentient::Sentient, trading_hours::TradingHours, treasury_router::TreasuryRouter, twap::Twap,
    volatility_guard, volatility_guard::VolatilityGuard, volume_guard::VolumeGuard, HookEnv, PumpHook,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookError {
    UnknownModule(u16),
    ModuleUnavailable(u16),
    ModuleDisabled(u16),
    BadConfig(u16),
    BadPriority(u16),
    BadCallbacks(u16),
    CapabilityNotAllowed(u16),
    /// A module acted beyond its granted capabilities. Always fails the transaction.
    CapabilityViolation(u16),
    DuplicateModule(u16),
    TooManySlots,
    TooManyRuntimeHooks,
    SlotNotFound,
    PolicyAccountMissing(u8),
    /// An after-callback check failed (e.g. observed size above the limit). Reverts the trade.
    PostCondition { module: u16, reason: u16 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceEntry {
    pub callback: u8,
    pub slot_index: u8,
    pub module_id: u16,
    pub allowed: bool,
    pub reason_code: u16,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct Trace {
    pub entries: [TraceEntry; MAX_TRACE],
    pub len: usize,
}

impl Default for Trace {
    fn default() -> Self {
        Self { entries: [TraceEntry::default(); MAX_TRACE], len: 0 }
    }
}

impl Trace {
    fn push(&mut self, e: TraceEntry) {
        if self.len < MAX_TRACE {
            self.entries[self.len] = e;
            self.len += 1;
        }
    }
    pub fn as_slice(&self) -> &[TraceEntry] {
        &self.entries[..self.len]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyEvent {
    pub policy_index: u8,
    pub trigger: u8,
    pub matched: bool,
    /// 0 allow, 1 deny, 2 record, 3 signal/intent.
    pub result: u8,
    pub reason_code: u16,
    pub tag: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PhaseOutcome {
    /// `Some` when a before-callback denied the trade.
    pub denied: Option<TraceEntry>,
    pub policy_events: [PolicyEvent; 8],
    pub policy_event_count: usize,
}

impl PhaseOutcome {
    fn push_policy(&mut self, e: PolicyEvent) {
        if self.policy_event_count < self.policy_events.len() {
            self.policy_events[self.policy_event_count] = e;
            self.policy_event_count += 1;
        }
    }
    pub fn policy_events(&self) -> &[PolicyEvent] {
        &self.policy_events[..self.policy_event_count]
    }
}

pub fn validate_module_config(module: ModuleId, config: &[u8; 48]) -> Result<(), HookError> {
    match module {
        ModuleId::LaunchShield => LaunchShield::validate_config(config),
        ModuleId::VolumeGuard => VolumeGuard::validate_config(config),
        ModuleId::VolatilityGuard => VolatilityGuard::validate_config(config),
        ModuleId::HolderXp => HolderXp::validate_config(config),
        ModuleId::TreasuryRouter => TreasuryRouter::validate_config(config),
        ModuleId::Twap => Twap::validate_config(config),
        ModuleId::Sentient => Sentient::validate_config(config),
        ModuleId::MaxWallet => MaxWallet::validate_config(config),
        ModuleId::SellLimit => SellLimit::validate_config(config),
        ModuleId::TradingHours => TradingHours::validate_config(config),
        ModuleId::Policy => {
            // config[0] = active policy mask (bits 0..MAX_POLICIES).
            if config[0] >> MAX_POLICIES != 0 || config[1..].iter().any(|&x| x != 0) {
                Err(HookError::BadConfig(ModuleId::Policy as u16))
            } else {
                Ok(())
            }
        }
        ModuleId::ConditionalOrders => Err(HookError::ModuleUnavailable(module as u16)),
    }
}

fn dispatch(module: ModuleId, env: &mut HookEnv<'_>) -> Result<HookDecision, HookError> {
    match module {
        ModuleId::LaunchShield => LaunchShield::on_callback(env),
        ModuleId::VolumeGuard => VolumeGuard::on_callback(env),
        ModuleId::VolatilityGuard => VolatilityGuard::on_callback(env),
        ModuleId::HolderXp => HolderXp::on_callback(env),
        ModuleId::TreasuryRouter => TreasuryRouter::on_callback(env),
        ModuleId::Twap => Twap::on_callback(env),
        ModuleId::Sentient => Sentient::on_callback(env),
        ModuleId::MaxWallet => MaxWallet::on_callback(env),
        ModuleId::SellLimit => SellLimit::on_callback(env),
        ModuleId::TradingHours => TradingHours::on_callback(env),
        ModuleId::Policy | ModuleId::ConditionalOrders => Ok(HookDecision::allow()),
    }
}

/// Install (or replace) a module in a market's slot table. Validates the module, priority
/// namespace, callbacks, capabilities and config; keeps slots sorted by `(priority, insertion)`.
/// Returns the slot index the module ended up at.
pub fn install_slot(
    slots: &mut [HookSlot; MAX_HOOK_SLOTS],
    module_id: u16,
    priority: u16,
    callbacks: u16,
    capabilities: u32,
    config: [u8; 48],
) -> Result<usize, HookError> {
    let module = ModuleId::from_u16(module_id).ok_or(HookError::UnknownModule(module_id))?;
    let info = module.info();
    if !info.available {
        return Err(HookError::ModuleUnavailable(module_id));
    }
    if priority < info.priority_min || priority > info.priority_max {
        return Err(HookError::BadPriority(module_id));
    }
    let callbacks = match module {
        ModuleId::Policy => info.callbacks,
        _ if callbacks == 0 => info.default_callbacks,
        _ => callbacks,
    };
    if callbacks & !info.callbacks != 0 || callbacks & !HookCallback::V1_MASK != 0 {
        return Err(HookError::BadCallbacks(module_id));
    }
    if !C::is_subset(capabilities, info.capabilities) || capabilities & C::FORBIDDEN_V1 != 0 {
        return Err(HookError::CapabilityNotAllowed(module_id));
    }
    validate_module_config(module, &config)?;
    if slots.iter().any(|s| s.module_id == module_id) {
        return Err(HookError::DuplicateModule(module_id));
    }
    let used = slots.iter().filter(|s| !s.is_empty()).count();
    if used >= MAX_HOOK_SLOTS {
        return Err(HookError::TooManySlots);
    }
    let runtime = slots.iter().filter(|s| !s.is_empty() && s.is_runtime()).count();
    if callbacks != 0 && runtime >= MAX_RUNTIME_HOOKS {
        return Err(HookError::TooManyRuntimeHooks);
    }

    let mut slot: HookSlot = bytemuck::Zeroable::zeroed();
    slot.module_id = module_id;
    slot.priority = priority;
    slot.callbacks = callbacks;
    slot.flags = hook_types::slot_flags::ENABLED | hook_types::slot_flags::REQUIRED;
    slot.capabilities = capabilities;
    slot.version = info.version;
    slot.config = config;

    // Insert keeping order: after every slot with priority <= ours.
    let mut pos = 0;
    while pos < used && slots[pos].priority <= priority {
        pos += 1;
    }
    let mut i = used;
    while i > pos {
        slots[i] = slots[i - 1];
        i -= 1;
    }
    slots[pos] = slot;
    Ok(pos)
}

/// Remove a module, compacting the table. Order of the remaining slots is preserved.
pub fn remove_slot(slots: &mut [HookSlot; MAX_HOOK_SLOTS], module_id: u16) -> Result<HookSlot, HookError> {
    let pos = slots.iter().position(|s| s.module_id == module_id && !s.is_empty()).ok_or(HookError::SlotNotFound)?;
    let removed = slots[pos];
    for i in pos..MAX_HOOK_SLOTS - 1 {
        slots[i] = slots[i + 1];
    }
    slots[MAX_HOOK_SLOTS - 1] = bytemuck::Zeroable::zeroed();
    Ok(removed)
}

/// Volatility in bps if a Volatility Guard is installed and warm (for the policy `VOLATILITY` field).
pub fn volatility_bps(slots: &[HookSlot; MAX_HOOK_SLOTS], ctx: &HookContext) -> Option<u64> {
    slots
        .iter()
        .find(|s| s.module_id == ModuleId::VolatilityGuard as u16 && s.is_enabled())
        .and_then(|s| volatility_guard::current_deviation(&s.config, &s.state, ctx))
}

/// Run one phase (before or after) of a routed trade over the market's hook stack.
///
/// - Hooks run in slot order (sorted by priority) for each callback of the phase.
/// - Before-phase: the first denial stops execution and is returned in `PhaseOutcome::denied`;
///   the router must then abort the transaction.
/// - After-phase: a denial is a protocol error (`PostCondition`).
/// - A module in `disabled_modules` that is still installed fails closed.
/// - Every effect is checked against the slot's granted capabilities.
#[allow(clippy::too_many_arguments)]
pub fn run_phase(
    side: Side,
    before: bool,
    slots: &mut [HookSlot; MAX_HOOK_SLOTS],
    ctx: &mut HookContext,
    market: &MarketStats,
    user: &mut UserState,
    policies: &mut [Option<&mut PolicyData>; MAX_POLICIES],
    disabled_modules: u64,
    trace: &mut Trace,
) -> Result<PhaseOutcome, HookError> {
    let mut out = PhaseOutcome::default();
    let vol = volatility_bps(slots, ctx);

    for callback in HookCallback::phase(side, before) {
        ctx.callback = callback as u8;
        for idx in 0..MAX_HOOK_SLOTS {
            let slot = slots[idx];
            if slot.is_empty() || !slot.is_enabled() || !slot.subscribes(callback.mask()) {
                continue;
            }
            let module = ModuleId::from_u16(slot.module_id).ok_or(HookError::UnknownModule(slot.module_id))?;
            if disabled_modules & module.kill_bit() != 0 {
                return Err(HookError::ModuleDisabled(slot.module_id));
            }
            ctx.slot_index = idx as u8;

            let decision = if module == ModuleId::Policy {
                run_policies(slot.config[0], callback, &*ctx, market, user, policies, vol, &mut out)?
            } else {
                let mut state = slot.state;
                let mut scratch_user = *user;
                let decision = {
                    let mut env = HookEnv {
                        callback,
                        ctx: &*ctx,
                        config: &slot.config,
                        state: &mut state,
                        user: &mut scratch_user,
                        market,
                    };
                    dispatch(module, &mut env)?
                };
                enforce(&slot, &decision, &state, user, &scratch_user)?;
                slots[idx].state = state;
                *user = scratch_user;
                decision
            };

            let entry = TraceEntry {
                callback: callback as u8,
                slot_index: idx as u8,
                module_id: slot.module_id,
                allowed: decision.is_allowed(),
                reason_code: decision.reason_code,
                flags: decision.flags,
            };
            trace.push(entry);
            if !decision.is_allowed() {
                if !before {
                    return Err(HookError::PostCondition { module: slot.module_id, reason: decision.reason_code });
                }
                if slot.capabilities & C::VETO_ROUTED_SWAP == 0 {
                    return Err(HookError::CapabilityViolation(slot.module_id));
                }
                out.denied = Some(entry);
                return Ok(out);
            }
        }
    }
    Ok(out)
}

/// Router bookkeeping after a successful routed trade (after both phases ran). `ctx` carries the
/// observed amounts.
pub fn commit_trade(ctx: &HookContext, market: &mut MarketStats, user: &mut UserState) {
    if ctx.is_buy() {
        market.routed_buy_quote = market.routed_buy_quote.saturating_add(ctx.quote_amount);
        user.routed_buy_quote = user.routed_buy_quote.saturating_add(ctx.quote_amount);
        if user.first_buy_ts == 0 {
            user.first_buy_ts = ctx.timestamp;
        }
    } else {
        market.routed_sell_quote = market.routed_sell_quote.saturating_add(ctx.quote_amount);
        user.routed_sell_quote = user.routed_sell_quote.saturating_add(ctx.quote_amount);
        user.last_sell_ts = ctx.timestamp;
    }
    market.trade_count = market.trade_count.saturating_add(1);
    user.trade_count = user.trade_count.saturating_add(1);
    user.last_trade_ts = ctx.timestamp;
    user.last_trade_slot = ctx.slot;
    user.last_side = ctx.side;
}

fn enforce(
    slot: &HookSlot,
    decision: &HookDecision,
    new_state: &[u8; 48],
    old_user: &UserState,
    new_user: &UserState,
) -> Result<(), HookError> {
    let caps = slot.capabilities;
    let id = slot.module_id;
    if !decision.is_allowed() && caps & C::VETO_ROUTED_SWAP == 0 {
        return Err(HookError::CapabilityViolation(id));
    }
    if *new_state != slot.state && caps & C::WRITE_HOOK_STATE == 0 {
        return Err(HookError::CapabilityViolation(id));
    }
    if new_user != old_user {
        if caps & C::WRITE_USER_STATE == 0 {
            return Err(HookError::CapabilityViolation(id));
        }
        let points_up = new_user.xp > old_user.xp
            || new_user.pending_xp > old_user.pending_xp
            || new_user.policy_points > old_user.policy_points;
        if points_up && caps & C::AWARD_POINTS == 0 {
            return Err(HookError::CapabilityViolation(id));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_policies(
    mask: u8,
    callback: HookCallback,
    ctx: &HookContext,
    market: &MarketStats,
    user: &mut UserState,
    policies: &mut [Option<&mut PolicyData>; MAX_POLICIES],
    vol: Option<u64>,
    out: &mut PhaseOutcome,
) -> Result<HookDecision, HookError> {
    let mut flags = 0u32;
    for i in 0..MAX_POLICIES {
        if mask & (1 << i) == 0 {
            continue;
        }
        let p = policies[i].as_deref_mut().ok_or(HookError::PolicyAccountMissing(i as u8))?;
        let o = {
            let input = EvalInput { ctx, market, user, volatility_bps: vol };
            policy_vm::evaluate(p, callback, &input)
        };
        if !o.evaluated {
            continue;
        }
        user.policy_points = user.policy_points.saturating_add(o.points);
        flags |= o.flags;
        let result = if o.deny.is_some() {
            1
        } else if o.signal || o.intent {
            3
        } else if o.record_tag.is_some() {
            2
        } else {
            0
        };
        out.push_policy(PolicyEvent {
            policy_index: i as u8,
            trigger: callback as u8,
            matched: o.matched,
            result,
            reason_code: o.deny.unwrap_or(0),
            tag: o.record_tag.unwrap_or(0),
        });
        if let Some(r) = o.deny {
            return Ok(HookDecision::deny(r));
        }
    }
    Ok(HookDecision::allow().with_flags(flags))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
