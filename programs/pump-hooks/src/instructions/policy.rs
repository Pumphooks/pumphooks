use anchor_lang::prelude::*;
use hook_types::{ModuleId, MAX_POLICIES};
use policy_vm::{Action, Condition, PolicyData, PolicySpec, ValidationEnv, MAX_ACTIONS, MAX_CONDITIONS};

use super::market::check_market_authority;
use crate::{errors::PumpHooksError as E, events::PolicyUpdated, state::*};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct ConditionArg {
    pub field: u8,
    pub op: u8,
    pub arg: u8,
    pub value: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug)]
pub struct ActionArg {
    pub kind: u8,
    pub arg: u8,
    pub arg16: u16,
    pub value: u64,
}

/// Compiled Composer output (WHEN trigger IF conditions THEN actions).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PolicyArgs {
    pub trigger: u8,
    pub combinator: u8,
    pub cooldown_secs: u32,
    pub deny_reason: u16,
    pub conditions: Vec<ConditionArg>,
    pub actions: Vec<ActionArg>,
    pub label_hash: [u8; 32],
}

impl PolicyArgs {
    fn to_spec(&self) -> Result<PolicySpec> {
        require!(self.conditions.len() <= MAX_CONDITIONS && self.actions.len() <= MAX_ACTIONS, E::InvalidPolicy);
        let mut s = PolicySpec {
            trigger: self.trigger,
            combinator: self.combinator,
            cooldown_secs: self.cooldown_secs,
            deny_reason: self.deny_reason,
            n_conditions: self.conditions.len() as u8,
            n_actions: self.actions.len() as u8,
            label_hash: self.label_hash,
            ..Default::default()
        };
        for (i, c) in self.conditions.iter().enumerate() {
            s.conditions[i] = Condition { field: c.field, op: c.op, arg: c.arg, _pad: [0; 5], value: c.value };
        }
        for (i, a) in self.actions.iter().enumerate() {
            s.actions[i] = Action { kind: a.kind, arg: a.arg, arg16: a.arg16, _pad: 0, value: a.value };
        }
        Ok(s)
    }
}

fn validated(market: &MarketConfig, args: &PolicyArgs) -> Result<PolicySpec> {
    let spec = args.to_spec()?;
    let slots = market.read_slots();
    let env = ValidationEnv { has_volatility_guard: slots.iter().any(|s| s.module_id == ModuleId::VolatilityGuard as u16) };
    spec.validate(&env).map_err(|e| {
        msg!("policy rejected: {:?}", e);
        error!(E::InvalidPolicy)
    })?;
    Ok(spec)
}

/// Set or clear policy bit `index` in the Policy slot's active mask.
fn set_policy_bit(m: &mut MarketConfig, index: u8, on: bool) -> Result<()> {
    let mut slots = m.read_slots();
    let s = slots
        .iter_mut()
        .find(|s| s.module_id == ModuleId::Policy as u16)
        .ok_or(E::PolicyModuleNotInstalled)?;
    if on {
        s.config[0] |= 1 << index;
    } else {
        s.config[0] &= !(1 << index);
    }
    m.write_slots(&slots);
    Ok(())
}

#[derive(Accounts)]
#[instruction(index: u8)]
pub struct CreatePolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    #[account(
        init,
        payer = authority,
        space = 8 + Policy::SIZE,
        seeds = [POLICY_SEED, market.key().as_ref(), &[index]],
        bump
    )]
    pub policy: AccountLoader<'info, Policy>,
    pub system_program: Program<'info, System>,
}

pub fn create_policy(ctx: Context<CreatePolicy>, index: u8, args: PolicyArgs) -> Result<()> {
    require!((index as usize) < MAX_POLICIES, E::BadPolicyIndex);
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    let spec = validated(&m, &args)?;
    set_policy_bit(&mut m, index, true)?;
    m.policy_mask |= 1 << index;

    let mut data: PolicyData = bytemuck::Zeroable::zeroed();
    data.market = ctx.accounts.market.key().to_bytes();
    data.index = index;
    data.enabled = 1;
    data.bump = ctx.bumps.policy;
    spec.write_to(&mut data);
    ctx.accounts.policy.load_init()?.write(&data);

    emit!(PolicyUpdated {
        market: ctx.accounts.market.key(),
        index,
        trigger: spec.trigger,
        n_conditions: spec.n_conditions,
        n_actions: spec.n_actions,
        enabled: true,
        closed: false,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(index: u8)]
pub struct UpdatePolicy<'info> {
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    #[account(mut, seeds = [POLICY_SEED, market.key().as_ref(), &[index]], bump)]
    pub policy: AccountLoader<'info, Policy>,
}

/// Replace the policy body (counters and registers reset) and/or toggle it.
pub fn update_policy(ctx: Context<UpdatePolicy>, index: u8, enabled: bool, args: Option<PolicyArgs>) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    let mut p = ctx.accounts.policy.load_mut()?;
    let mut data = p.read();
    if let Some(a) = args {
        let spec = validated(&m, &a)?;
        let (market, bump) = (data.market, data.bump);
        data = bytemuck::Zeroable::zeroed();
        data.market = market;
        data.index = index;
        data.bump = bump;
        spec.write_to(&mut data);
    }
    data.enabled = enabled as u8;
    p.write(&data);
    set_policy_bit(&mut m, index, enabled)?;
    emit!(PolicyUpdated {
        market: ctx.accounts.market.key(),
        index,
        trigger: data.trigger,
        n_conditions: data.n_conditions,
        n_actions: data.n_actions,
        enabled,
        closed: false,
    });
    Ok(())
}

#[derive(Accounts)]
#[instruction(index: u8)]
pub struct ClosePolicy<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    #[account(mut, close = authority, seeds = [POLICY_SEED, market.key().as_ref(), &[index]], bump)]
    pub policy: AccountLoader<'info, Policy>,
}

pub fn close_policy(ctx: Context<ClosePolicy>, index: u8) -> Result<()> {
    let mut m = ctx.accounts.market.load_mut()?;
    check_market_authority(&m, &ctx.accounts.authority.key())?;
    // The Policy slot may already be removed; clearing the mask is then a no-op.
    let _ = set_policy_bit(&mut m, index, false);
    m.policy_mask &= !(1 << index);
    emit!(PolicyUpdated {
        market: ctx.accounts.market.key(),
        index,
        trigger: 0,
        n_conditions: 0,
        n_actions: 0,
        enabled: false,
        closed: true,
    });
    Ok(())
}
