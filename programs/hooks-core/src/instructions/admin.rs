use anchor_lang::prelude::*;
use hooks_interface::{perm, INTERFACE_VERSION};

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

// ---------------------------------------------------------------- hook registry

#[derive(Accounts)]
pub struct RegisterHook<'info> {
    #[account(mut)]
    pub registrant: Signer<'info>,
    #[account(mut, seeds = [PROTOCOL_SEED], bump = protocol.bump)]
    pub protocol: Account<'info, Protocol>,
    /// CHECK: any executable program; loader state read by `read_program`.
    pub hook_program: UncheckedAccount<'info>,
    /// CHECK: validated by `read_program` (required for upgradeable programs).
    pub hook_program_data: Option<UncheckedAccount<'info>>,
    #[account(
        init,
        payer = registrant,
        space = 8 + HookRegistration::INIT_SPACE,
        seeds = [HOOK_SEED, hook_program.key().as_ref()],
        bump
    )]
    pub registration: Account<'info, HookRegistration>,
    pub system_program: Program<'info, System>,
}

/// Registration is permissionless, but only the hook's upgrade authority can register an
/// upgradeable hook; anyone can register an immutable one.
pub fn register_hook(ctx: Context<RegisterHook>, permissions: u32, interface_version: u8, metadata_uri: String) -> Result<()> {
    require!(perm::is_valid(permissions), CoreError::InvalidPermissions);
    require!(interface_version == INTERFACE_VERSION, CoreError::UnsupportedInterface);
    require!(metadata_uri.len() <= MAX_URI_LEN, CoreError::UriTooLong);
    let info = read_program(
        &ctx.accounts.hook_program,
        ctx.accounts.hook_program_data.as_ref().map(|a| a.as_ref()),
    )?;
    if let Some(auth) = info.upgrade_authority {
        require_keys_eq!(auth, ctx.accounts.registrant.key(), CoreError::NotUpgradeAuthority);
    }
    let now = Clock::get()?.unix_timestamp;
    let r = &mut ctx.accounts.registration;
    r.program = ctx.accounts.hook_program.key();
    r.registrant = ctx.accounts.registrant.key();
    r.upgrade_authority = info.upgrade_authority;
    r.deploy_slot = info.deploy_slot;
    r.permissions = permissions;
    r.interface_version = interface_version;
    r.bump = ctx.bumps.registration;
    r.pool_count = 0;
    r.registered_at = now;
    r.updated_at = now;
    r.metadata_uri = metadata_uri;
    ctx.accounts.protocol.hook_count += 1;
    emit_registration(r);
    Ok(())
}

#[derive(Accounts)]
pub struct UpdateHook<'info> {
    pub authority: Signer<'info>,
    /// CHECK: must match the registration.
    #[account(address = registration.program)]
    pub hook_program: UncheckedAccount<'info>,
    /// CHECK: validated by `read_program`.
    pub hook_program_data: Option<UncheckedAccount<'info>>,
    #[account(mut, seeds = [HOOK_SEED, hook_program.key().as_ref()], bump = registration.bump)]
    pub registration: Account<'info, HookRegistration>,
}

/// Refreshes the observed loader state and updates declared permissions / metadata. Existing
/// pools keep the permissions they were created with.
pub fn update_hook(ctx: Context<UpdateHook>, permissions: u32, metadata_uri: String) -> Result<()> {
    require!(perm::is_valid(permissions), CoreError::InvalidPermissions);
    require!(metadata_uri.len() <= MAX_URI_LEN, CoreError::UriTooLong);
    let info = read_program(
        &ctx.accounts.hook_program,
        ctx.accounts.hook_program_data.as_ref().map(|a| a.as_ref()),
    )?;
    let r = &mut ctx.accounts.registration;
    let allowed = info.upgrade_authority.unwrap_or(r.registrant);
    require_keys_eq!(allowed, ctx.accounts.authority.key(), CoreError::NotUpgradeAuthority);
    r.upgrade_authority = info.upgrade_authority;
    r.deploy_slot = info.deploy_slot;
    r.permissions = permissions;
    r.metadata_uri = metadata_uri;
    r.updated_at = Clock::get()?.unix_timestamp;
    emit_registration(r);
    Ok(())
}

fn emit_registration(r: &HookRegistration) {
    emit!(HookRegistered {
        program: r.program,
        registrant: r.registrant,
        permissions: r.permissions,
        upgrade_authority: r.upgrade_authority,
        deploy_slot: r.deploy_slot,
        metadata_uri: r.metadata_uri.clone(),
    });
}

// ---------------------------------------------------------------- attestations

#[derive(Accounts)]
pub struct Attest<'info> {
    #[account(mut)]
    pub attester: Signer<'info>,
    #[account(seeds = [HOOK_SEED, registration.program.as_ref()], bump = registration.bump)]
    pub registration: Account<'info, HookRegistration>,
    #[account(
        init,
        payer = attester,
        space = 8 + Attestation::INIT_SPACE,
        seeds = [ATTEST_SEED, registration.program.as_ref(), attester.key().as_ref()],
        bump
    )]
    pub attestation: Account<'info, Attestation>,
    pub system_program: Program<'info, System>,
}

/// Anyone may attest; the attestation records who did and for which deploy slot.
pub fn attest(ctx: Context<Attest>, kind: u8, content_hash: [u8; 32], uri: String) -> Result<()> {
    require!(kind <= ATTEST_WARNING, CoreError::InvalidPermissions);
    require!(uri.len() <= MAX_URI_LEN, CoreError::UriTooLong);
    let reg = &ctx.accounts.registration;
    let a = &mut ctx.accounts.attestation;
    a.hook_program = reg.program;
    a.attester = ctx.accounts.attester.key();
    a.kind = kind;
    a.bump = ctx.bumps.attestation;
    a.deploy_slot = reg.deploy_slot;
    a.content_hash = content_hash;
    a.created_at = Clock::get()?.unix_timestamp;
    a.uri = uri;
    emit!(Attested {
        hook_program: a.hook_program,
        attester: a.attester,
        kind,
        deploy_slot: a.deploy_slot,
        content_hash,
        uri: a.uri.clone(),
        revoked: false,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct RevokeAttestation<'info> {
    #[account(mut)]
    pub attester: Signer<'info>,
    #[account(mut, close = attester, has_one = attester @ CoreError::Unauthorized)]
    pub attestation: Account<'info, Attestation>,
}

pub fn revoke_attestation(ctx: Context<RevokeAttestation>) -> Result<()> {
    let a = &ctx.accounts.attestation;
    emit!(Attested {
        hook_program: a.hook_program,
        attester: a.attester,
        kind: a.kind,
        deploy_slot: a.deploy_slot,
        content_hash: a.content_hash,
        uri: a.uri.clone(),
        revoked: true,
    });
    Ok(())
}
