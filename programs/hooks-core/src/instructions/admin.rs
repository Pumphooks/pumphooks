use anchor_lang::prelude::*;

use crate::{errors::CoreError, events::*, hook::read_program, state::*};

// ---------------------------------------------------------------- protocol

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + Protocol::INIT_SPACE, seeds = [PROTOCOL_SEED], bump)]
    pub protocol: Account<'info, Protocol>,
    /// CHECK: this program; its ProgramData proves who may initialize.
    #[account(address = crate::ID)]
    pub core_program: UncheckedAccount<'info>,
    /// CHECK: validated by `read_program`.
    pub core_program_data: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

/// Only the core's upgrade authority may initialize, so deployment can't be front-run.
pub fn initialize_protocol(ctx: Context<InitializeProtocol>, fee_authority: Pubkey) -> Result<()> {
    let info = read_program(&ctx.accounts.core_program, Some(&ctx.accounts.core_program_data))?;
    require!(
        info.upgrade_authority == Some(ctx.accounts.admin.key()),
        CoreError::NotUpgradeAuthority
    );
    let p = &mut ctx.accounts.protocol;
    p.admin = ctx.accounts.admin.key();
    p.pending_admin = Pubkey::default();
    p.fee_authority = fee_authority;
    p.protocol_fee_bps = 0;
    p.paused = 0;
    p.bump = ctx.bumps.protocol;
    emit_protocol(p);
    Ok(())
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct ProtocolUpdate {
    pub pending_admin: Option<Pubkey>,
    pub fee_authority: Option<Pubkey>,
    pub protocol_fee_bps: Option<u16>,
    pub paused: Option<u8>,
}

#[derive(Accounts)]
pub struct UpdateProtocol<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump, has_one = admin @ CoreError::Unauthorized)]
    pub protocol: Account<'info, Protocol>,
}

pub fn update_protocol(ctx: Context<UpdateProtocol>, u: ProtocolUpdate) -> Result<()> {
    let p = &mut ctx.accounts.protocol;
    if let Some(a) = u.pending_admin {
        p.pending_admin = a;
    }
    if let Some(f) = u.fee_authority {
        p.fee_authority = f;
    }
    if let Some(bps) = u.protocol_fee_bps {
        require!(bps <= MAX_PROTOCOL_FEE_BPS, CoreError::ProtocolFeeTooHigh);
        p.protocol_fee_bps = bps;
    }
    if let Some(paused) = u.paused {
        require!(paused & !(PAUSE_POOLS | PAUSE_SWAPS | PAUSE_ADDS) == 0, CoreError::Unauthorized);
        p.paused = paused;
    }
    emit_protocol(p);
    Ok(())
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub pending_admin: Signer<'info>,
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump, has_one = pending_admin @ CoreError::Unauthorized)]
    pub protocol: Account<'info, Protocol>,
}

pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
    let p = &mut ctx.accounts.protocol;
    p.admin = p.pending_admin;
    p.pending_admin = Pubkey::default();
    emit_protocol(p);
    Ok(())
}

fn emit_protocol(p: &Protocol) {
    emit!(ProtocolUpdated {
        admin: p.admin,
        pending_admin: p.pending_admin,
        fee_authority: p.fee_authority,
        protocol_fee_bps: p.protocol_fee_bps,
        paused: p.paused,
    });
}
