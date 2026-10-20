//! SENTIENT signal submission. The off-chain engine signs `SIGNAL_DOMAIN ‖ StrategySignal`; anyone
//! may relay it. The transaction must contain an Ed25519 program instruction verifying exactly
//! that message under the signer configured in the market's SENTIENT slot. The engine never holds
//! a key that can move user or treasury funds.

use anchor_lang::{
    prelude::*,
    solana_program::{ed25519_program, sysvar::instructions as ix_sysvar},
};
use hook_runtime::modules::sentient;
use hook_types::{ModuleId, StrategySignal};

use crate::{errors::PumpHooksError as E, events::StrategySignalApplied, state::*};

#[derive(Accounts)]
pub struct SubmitSignal<'info> {
    #[account(mut)]
    pub market: AccountLoader<'info, MarketConfig>,
    /// CHECK: instructions sysvar (address checked).
    #[account(address = ix_sysvar::ID)]
    pub instructions: UncheckedAccount<'info>,
}

/// Find an Ed25519 verify instruction (self-contained offsets) for `(pubkey, message)`.
fn ed25519_verified(ix_ai: &AccountInfo, pubkey: &[u8; 32], message: &[u8]) -> Result<bool> {
    let current = ix_sysvar::load_current_index_checked(ix_ai)? as usize;
    for i in 0..current {
        let ix = ix_sysvar::load_instruction_at_checked(i, ix_ai)?;
        if ix.program_id != ed25519_program::ID {
            continue;
        }
        let d = &ix.data;
        if d.len() < 2 + 14 || d[0] == 0 {
            continue;
        }
        let n = d[0] as usize;
        for s in 0..n {
            let o = 2 + 14 * s;
            if d.len() < o + 14 {
                break;
            }
            let rd = |k: usize| u16::from_le_bytes([d[o + k], d[o + k + 1]]) as usize;
            let (sig_ix, pk_off, pk_ix, msg_off, msg_len, msg_ix) = (rd(2), rd(4), rd(6), rd(8), rd(10), rd(12));
            if sig_ix != u16::MAX as usize || pk_ix != u16::MAX as usize || msg_ix != u16::MAX as usize {
                continue;
            }
            if d.len() < pk_off + 32 || d.len() < msg_off + msg_len {
                continue;
            }
            if &d[pk_off..pk_off + 32] == pubkey && &d[msg_off..msg_off + msg_len] == message {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn submit_signal(ctx: Context<SubmitSignal>, signal: [u8; 96]) -> Result<()> {
    let signal: StrategySignal = bytemuck::pod_read_unaligned(&signal);
    let market_key = ctx.accounts.market.key();
    let mut m = ctx.accounts.market.load_mut()?;
    let mut slots = m.read_slots();
    let slot = slots
        .iter_mut()
        .find(|s| s.module_id == ModuleId::Sentient as u16 && s.is_enabled())
        .ok_or(E::SentientNotInstalled)?;
    let cfg = sentient::Config::decode(&slot.config);
    require!(ed25519_verified(&ctx.accounts.instructions, &cfg.signer, &signal.message())?, E::SignalNotVerified);
    let now = Clock::get()?.unix_timestamp;
    let action = sentient::apply_signal(market_key.to_bytes(), &slot.config, &mut slot.state, &signal, now).map_err(|r| {
        msg!("signal rejected: {:?}", r);
        error!(E::SignalRejected)
    })?;
    m.write_slots(&slots);
    emit!(StrategySignalApplied {
        market: market_key,
        signer: Pubkey::new_from_array(cfg.signer),
        nonce: signal.nonce,
        action: action as u8,
        param: signal.param,
        valid_until: signal.valid_until,
        confidence_bps: signal.confidence_bps,
    });
    Ok(())
}
