//! PUMP HOOKS registry (`docs/STANDARD.md` §4).
//!
//! - `Hook` `["hook", slug]`: a named hook and its developer authority.
//! - `HookVersion` `["version", hook, index]`: append-only; only `status` changes after publish.
//! - `Reviewer` `["reviewer", key]`: a named reviewer, vouched for by the registry admin.
//! - `Review` `["review", version, reviewer]`: a signed statement with evidence.
//!
//! Registration is not a review. Publishing proves nothing about safety; it records what a
//! version claims (callbacks, capabilities, artifact hash) and what the loader says about its
//! program (upgrade authority, deploy slot) at publication.

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::prelude::*;
use hook_kernel::{builtin, POLICY_PROGRAM_CAPS};
use hook_standard::{cap, loader, Environment, StackEntry, IMPL_BUILTIN, IMPL_PROGRAM};

declare_id!("9gKUqwnUa6ZJkCvNT4bnLY7A5czcJFuSV1dpYeosHHs4");

pub const CONFIG_SEED: &[u8] = b"config";
pub const HOOK_SEED: &[u8] = b"hook";
pub const VERSION_SEED: &[u8] = b"version";
pub const REVIEWER_SEED: &[u8] = b"reviewer";
pub const REVIEW_SEED: &[u8] = b"review";
pub const MAX_SLUG: usize = 32;
pub const MAX_URI: usize = 200;
pub const MAX_NAME: usize = 64;

pub const STATUS_ACTIVE: u8 = 0;
pub const STATUS_DEPRECATED: u8 = 1;
pub const STATUS_REVOKED: u8 = 2;

pub const REVIEW_COMPATIBILITY_TESTED: u8 = 0;
pub const REVIEW_INDEPENDENT_AUDIT: u8 = 1;
pub const REVIEW_SECURITY_WARNING: u8 = 2;

pub const UPGRADE_IMMUTABLE: u8 = 0;
pub const UPGRADE_UPGRADEABLE: u8 = 1;
pub const UPGRADE_BUILTIN: u8 = 2;

#[program]
pub mod hook_registry {
    use super::*;

    /// Only this program's upgrade authority may initialize, so deployment can't be front-run.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let info = loader::read_program(
            &ctx.accounts.registry_program,
            Some(&ctx.accounts.registry_program_data),
        )?;
        require!(
            info.upgrade_authority == Some(ctx.accounts.admin.key()),
            RegistryError::Unauthorized
        );
        let c = &mut ctx.accounts.config;
        c.admin = ctx.accounts.admin.key();
        c.pending_admin = None;
        c.bump = ctx.bumps.config;
        Ok(())
    }

    pub fn propose_admin(ctx: Context<AdminOnly>, new_admin: Pubkey) -> Result<()> {
        ctx.accounts.config.pending_admin = Some(new_admin);
        Ok(())
    }

    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        let c = &mut ctx.accounts.config;
        require!(
            c.pending_admin == Some(ctx.accounts.new_admin.key()),
            RegistryError::Unauthorized
        );
        c.admin = ctx.accounts.new_admin.key();
        c.pending_admin = None;
        Ok(())
    }

    /// Names a reviewer. The admin vouches only for identity, not for any review's content.
    pub fn add_reviewer(
        ctx: Context<AddReviewer>,
        reviewer: Pubkey,
        name: String,
        uri: String,
    ) -> Result<()> {
        require!(
            !name.is_empty() && name.len() <= MAX_NAME,
            RegistryError::InvalidString
        );
        require!(uri.len() <= MAX_URI, RegistryError::InvalidString);
        let r = &mut ctx.accounts.reviewer_account;
        r.reviewer = reviewer;
        r.name = name;
        r.uri = uri;
        r.active = true;
        r.bump = ctx.bumps.reviewer_account;
        emit!(ReviewerUpdated {
            reviewer,
            active: true
        });
        Ok(())
    }

    pub fn set_reviewer_active(ctx: Context<SetReviewerActive>, active: bool) -> Result<()> {
        let r = &mut ctx.accounts.reviewer_account;
        r.active = active;
        emit!(ReviewerUpdated {
            reviewer: r.reviewer,
            active
        });
        Ok(())
    }

    pub fn create_hook(ctx: Context<CreateHook>, slug: String) -> Result<()> {
        require!(valid_slug(&slug), RegistryError::InvalidSlug);
        let h = &mut ctx.accounts.hook;
        h.authority = ctx.accounts.authority.key();
        h.pending_authority = None;
        h.slug = slug.clone();
        h.version_count = 0;
        h.latest = None;
        h.status = STATUS_ACTIVE;
        h.created_at = Clock::get()?.unix_timestamp;
        h.bump = ctx.bumps.hook;
        emit!(HookCreated {
            hook: h.key(),
            slug,
            authority: h.authority
        });
        Ok(())
    }

    pub fn propose_hook_authority(
        ctx: Context<HookAuthorityOnly>,
        new_authority: Pubkey,
    ) -> Result<()> {
        ctx.accounts.hook.pending_authority = Some(new_authority);
        Ok(())
    }

    pub fn accept_hook_authority(ctx: Context<AcceptHookAuthority>) -> Result<()> {
        let h = &mut ctx.accounts.hook;
        require!(
            h.pending_authority == Some(ctx.accounts.new_authority.key()),
            RegistryError::Unauthorized
        );
        h.authority = ctx.accounts.new_authority.key();
        h.pending_authority = None;
        Ok(())
    }

    /// Hook-level status: deprecated or revoked hooks accept no new versions.
    pub fn set_hook_status(ctx: Context<HookAuthorityOnly>, status: u8) -> Result<()> {
        let h = &mut ctx.accounts.hook;
        require!(
            status > h.status && status <= STATUS_REVOKED,
            RegistryError::InvalidStatus
        );
        h.status = status;
        Ok(())
    }

    /// Appends a version. Builtin versions may only be published by the registry admin (builtins
    /// are code inside our environment programs). Upgradeable program versions must be published
    /// by the program's upgrade authority.
    pub fn publish_version(ctx: Context<PublishVersion>, p: PublishParams) -> Result<()> {
        let hook = &mut ctx.accounts.hook;
        require!(hook.status == STATUS_ACTIVE, RegistryError::InvalidStatus);
        require!(
            p.manifest_uri.len() <= MAX_URI,
            RegistryError::InvalidString
        );
        if let Some(prev) = hook.latest {
            require!(p.semver > prev, RegistryError::VersionNotIncreasing);
        }
        validate_claims(&p)?;

        let (program, upgrade_policy, upgrade_authority, deploy_slot) = match p.impl_kind {
            IMPL_BUILTIN => {
                require_keys_eq!(
                    ctx.accounts.publisher.key(),
                    ctx.accounts.config.admin,
                    RegistryError::Unauthorized
                );
                (Pubkey::default(), UPGRADE_BUILTIN, None, 0)
            }
            _ => {
                let program = ctx
                    .accounts
                    .hook_program
                    .as_ref()
                    .ok_or(error!(RegistryError::MissingProgram))?;
                let info = loader::read_program(
                    program,
                    ctx.accounts.hook_program_data.as_ref().map(|a| a.as_ref()),
                )?;
                if let Some(auth) = info.upgrade_authority {
                    require_keys_eq!(
                        auth,
                        ctx.accounts.publisher.key(),
                        RegistryError::Unauthorized
                    );
                }
                let policy = if info.upgrade_authority.is_some() {
                    UPGRADE_UPGRADEABLE
                } else {
                    UPGRADE_IMMUTABLE
                };
                (
                    program.key(),
                    policy,
                    info.upgrade_authority,
                    info.deploy_slot,
                )
            }
        };

        let clock = Clock::get()?;
        let v = &mut ctx.accounts.version;
        v.hook = hook.key();
        v.index = hook.version_count;
        v.semver = p.semver;
        v.impl_kind = p.impl_kind;
        v.builtin_id = p.builtin_id;
        v.program = program;
        v.environments = p.environments;
        v.callbacks = p.callbacks;
        v.capabilities = p.capabilities;
        v.config_schema_hash = p.config_schema_hash;
        v.manifest_hash = p.manifest_hash;
        v.manifest_uri = p.manifest_uri;
        v.artifact_hash = p.artifact_hash;
        v.upgrade_policy = upgrade_policy;
        v.observed_upgrade_authority = upgrade_authority;
        v.observed_deploy_slot = deploy_slot;
        v.status = STATUS_ACTIVE;
        v.publisher = ctx.accounts.publisher.key();
        v.published_at = clock.unix_timestamp;
        v.published_slot = clock.slot;
        v.bump = ctx.bumps.version;

        hook.version_count += 1;
        hook.latest = Some(p.semver);
        emit!(VersionPublished {
            hook: hook.key(),
            version: v.key(),
            index: v.index,
            semver: v.semver,
            impl_kind: v.impl_kind,
            program,
            environments: v.environments,
            capabilities: v.capabilities,
            manifest_hash: v.manifest_hash,
            upgrade_policy,
        });
        Ok(())
    }

    /// Active → Deprecated → Revoked, never backwards.
    pub fn set_version_status(ctx: Context<SetVersionStatus>, status: u8) -> Result<()> {
        let v = &mut ctx.accounts.version;
        require!(
            status > v.status && status <= STATUS_REVOKED,
            RegistryError::InvalidStatus
        );
        v.status = status;
        emit!(VersionStatusChanged {
            version: v.key(),
            status
        });
        Ok(())
    }

    /// Compatibility and audit reviews require an active named reviewer. Security warnings may be
    /// posted by anyone and are marked unverified when the poster is not a named reviewer.
    pub fn post_review(
        ctx: Context<PostReview>,
        kind: u8,
        evidence_hash: [u8; 32],
        evidence_uri: String,
    ) -> Result<()> {
        require!(
            kind <= REVIEW_SECURITY_WARNING,
            RegistryError::InvalidReview
        );
        require!(
            !evidence_uri.is_empty() && evidence_uri.len() <= MAX_URI,
            RegistryError::InvalidString
        );
        let named = match &ctx.accounts.reviewer_account {
            Some(r) => {
                let (expected, _) = Pubkey::find_program_address(
                    &[REVIEWER_SEED, ctx.accounts.reviewer.key.as_ref()],
                    &crate::ID,
                );
                require_keys_eq!(r.key(), expected, RegistryError::Unauthorized);
                r.active
            }
            None => false,
        };
        require!(
            named || kind == REVIEW_SECURITY_WARNING,
            RegistryError::NotNamedReviewer
        );
        let r = &mut ctx.accounts.review;
        r.version = ctx.accounts.version.key();
        r.reviewer = ctx.accounts.reviewer.key();
        r.named_reviewer = named;
        r.kind = kind;
        r.evidence_hash = evidence_hash;
        r.evidence_uri = evidence_uri;
        r.created_at = Clock::get()?.unix_timestamp;
        r.retracted = false;
        r.bump = ctx.bumps.review;
        emit!(ReviewPosted {
            version: r.version,
            reviewer: r.reviewer,
            kind,
            named_reviewer: named,
            evidence_hash
        });
        Ok(())
    }

    pub fn retract_review(ctx: Context<RetractReview>) -> Result<()> {
        let r = &mut ctx.accounts.review;
        r.retracted = true;
        emit!(ReviewRetracted {
            version: r.version,
            reviewer: r.reviewer
        });
        Ok(())
    }
}

// ---------------------------------------------------------------- validation

pub fn valid_slug(s: &str) -> bool {
    (3..=MAX_SLUG).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// What a version may claim, per the standard and the kernel.
pub fn validate_claims(p: &PublishParams) -> Result<()> {
    require!(
        p.environments != 0 && p.environments & !0b1111 == 0,
        RegistryError::InvalidClaims
    );
    require!(
        p.capabilities != 0 && p.capabilities & !cap::ALL == 0,
        RegistryError::InvalidClaims
    );
    for env in Environment::ALL {
        let listed = p.environments & env.bit() != 0;
        let cbs = p.callbacks[env as usize];
        require!(listed == (cbs != 0), RegistryError::InvalidClaims);
        require!(
            cbs & !env.callback_mask() == 0,
            RegistryError::InvalidClaims
        );
        if !listed {
            continue;
        }
        match p.impl_kind {
            IMPL_BUILTIN => {
                require!(
                    builtin::name(p.builtin_id).is_some(),
                    RegistryError::InvalidClaims
                );
                require!(
                    cbs & !builtin::supported_callbacks(p.builtin_id, env) == 0,
                    RegistryError::InvalidClaims
                );
                require!(
                    p.capabilities == builtin::required_caps(p.builtin_id),
                    RegistryError::InvalidClaims
                );
            }
            IMPL_PROGRAM => {
                require!(p.builtin_id == 0, RegistryError::InvalidClaims);
                if env != Environment::Pool {
                    require!(
                        p.capabilities & !POLICY_PROGRAM_CAPS == 0,
                        RegistryError::InvalidClaims
                    );
                }
            }
            _ => return err!(RegistryError::InvalidClaims),
        }
    }
    Ok(())
}

/// What an environment checks when installing `entry` that references `version` (whose account
/// key is `version_key`, owned by this program).
pub fn verify_install(
    version_key: &Pubkey,
    version: &HookVersion,
    entry: &StackEntry,
    env: Environment,
) -> Result<()> {
    require!(
        entry.hook_version == version_key.to_bytes(),
        RegistryError::VersionMismatch
    );
    require!(
        version.status == STATUS_ACTIVE,
        RegistryError::InvalidStatus
    );
    require!(
        version.environments & env.bit() != 0,
        RegistryError::VersionMismatch
    );
    require!(
        entry.impl_kind == version.impl_kind,
        RegistryError::VersionMismatch
    );
    match version.impl_kind {
        IMPL_BUILTIN => require!(
            entry.builtin_id == version.builtin_id,
            RegistryError::VersionMismatch
        ),
        _ => {
            require!(
                entry.program == version.program.to_bytes(),
                RegistryError::VersionMismatch
            );
            require!(
                entry.pinned_slot == 0 || entry.pinned_slot == version.observed_deploy_slot,
                RegistryError::VersionMismatch
            );
        }
    }
    require!(
        entry.callbacks & !version.callbacks[env as usize] == 0,
        RegistryError::VersionMismatch
    );
    require!(
        entry.granted & !version.capabilities == 0,
        RegistryError::VersionMismatch
    );
    Ok(())
}

// ---------------------------------------------------------------- state

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey,
    pub pending_admin: Option<Pubkey>,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Hook {
    pub authority: Pubkey,
    pub pending_authority: Option<Pubkey>,
    #[max_len(MAX_SLUG)]
    pub slug: String,
    pub version_count: u32,
    pub latest: Option<[u16; 3]>,
    pub status: u8,
    pub created_at: i64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct HookVersion {
    pub hook: Pubkey,
    pub index: u32,
    pub semver: [u16; 3],
    pub impl_kind: u8,
    pub builtin_id: u8,
    /// Zero for builtins.
    pub program: Pubkey,
    pub environments: u8,
    /// Callback mask per environment (index = `Environment as u8`).
    pub callbacks: [u16; 4],
    pub capabilities: u32,
    pub config_schema_hash: [u8; 32],
    /// sha256 of the canonical manifest JSON.
    pub manifest_hash: [u8; 32],
    #[max_len(MAX_URI)]
    pub manifest_uri: String,
    /// sha256 of the program's `.so` as built from `source` (claimed by the publisher).
    pub artifact_hash: [u8; 32],
    pub upgrade_policy: u8,
    pub observed_upgrade_authority: Option<Pubkey>,
    pub observed_deploy_slot: u64,
    pub status: u8,
    pub publisher: Pubkey,
    pub published_at: i64,
    pub published_slot: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Reviewer {
    pub reviewer: Pubkey,
    #[max_len(MAX_NAME)]
    pub name: String,
    #[max_len(MAX_URI)]
    pub uri: String,
    pub active: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Review {
    pub version: Pubkey,
    pub reviewer: Pubkey,
    pub named_reviewer: bool,
    pub kind: u8,
    pub evidence_hash: [u8; 32],
    #[max_len(MAX_URI)]
    pub evidence_uri: String,
    pub created_at: i64,
    pub retracted: bool,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PublishParams {
    pub semver: [u16; 3],
    pub impl_kind: u8,
    pub builtin_id: u8,
    pub environments: u8,
    pub callbacks: [u16; 4],
    pub capabilities: u32,
    pub config_schema_hash: [u8; 32],
    pub manifest_hash: [u8; 32],
    pub manifest_uri: String,
    pub artifact_hash: [u8; 32],
}

// ---------------------------------------------------------------- accounts

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + Config::INIT_SPACE, seeds = [CONFIG_SEED], bump)]
    pub config: Account<'info, Config>,
    /// CHECK: this program; its ProgramData proves who may initialize.
    #[account(address = crate::ID)]
    pub registry_program: UncheckedAccount<'info>,
    /// CHECK: validated by `loader::read_program`.
    pub registry_program_data: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ RegistryError::Unauthorized)]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
pub struct AcceptAdmin<'info> {
    pub new_admin: Signer<'info>,
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
}

#[derive(Accounts)]
#[instruction(reviewer: Pubkey)]
pub struct AddReviewer<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ RegistryError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(init, payer = admin, space = 8 + Reviewer::INIT_SPACE, seeds = [REVIEWER_SEED, reviewer.as_ref()], bump)]
    pub reviewer_account: Account<'info, Reviewer>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetReviewerActive<'info> {
    pub admin: Signer<'info>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump, has_one = admin @ RegistryError::Unauthorized)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [REVIEWER_SEED, reviewer_account.reviewer.as_ref()], bump = reviewer_account.bump)]
    pub reviewer_account: Account<'info, Reviewer>,
}

#[derive(Accounts)]
#[instruction(slug: String)]
pub struct CreateHook<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = 8 + Hook::INIT_SPACE, seeds = [HOOK_SEED, slug.as_bytes()], bump)]
    pub hook: Account<'info, Hook>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct HookAuthorityOnly<'info> {
    pub authority: Signer<'info>,
    #[account(mut, has_one = authority @ RegistryError::Unauthorized)]
    pub hook: Account<'info, Hook>,
}

#[derive(Accounts)]
pub struct AcceptHookAuthority<'info> {
    pub new_authority: Signer<'info>,
    #[account(mut)]
    pub hook: Account<'info, Hook>,
}

#[derive(Accounts)]
pub struct PublishVersion<'info> {
    #[account(mut)]
    pub publisher: Signer<'info>,
    /// Must be the hook's authority (and, for builtins, also the registry admin).
    #[account(mut, constraint = hook.authority == publisher.key() @ RegistryError::Unauthorized)]
    pub hook: Account<'info, Hook>,
    #[account(seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        init,
        payer = publisher,
        space = 8 + HookVersion::INIT_SPACE,
        seeds = [VERSION_SEED, hook.key().as_ref(), &hook.version_count.to_le_bytes()],
        bump
    )]
    pub version: Account<'info, HookVersion>,
    /// CHECK: validated by `loader::read_program` (program versions only).
    pub hook_program: Option<UncheckedAccount<'info>>,
    /// CHECK: validated by `loader::read_program`.
    pub hook_program_data: Option<UncheckedAccount<'info>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetVersionStatus<'info> {
    pub authority: Signer<'info>,
    #[account(has_one = authority @ RegistryError::Unauthorized)]
    pub hook: Account<'info, Hook>,
    #[account(mut, has_one = hook @ RegistryError::VersionMismatch)]
    pub version: Account<'info, HookVersion>,
}

#[derive(Accounts)]
pub struct PostReview<'info> {
    #[account(mut)]
    pub reviewer: Signer<'info>,
    pub reviewer_account: Option<Account<'info, Reviewer>>,
    pub version: Account<'info, HookVersion>,
    #[account(
        init,
        payer = reviewer,
        space = 8 + Review::INIT_SPACE,
        seeds = [REVIEW_SEED, version.key().as_ref(), reviewer.key().as_ref()],
        bump
    )]
    pub review: Account<'info, Review>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RetractReview<'info> {
    pub reviewer: Signer<'info>,
    #[account(mut, has_one = reviewer @ RegistryError::Unauthorized)]
    pub review: Account<'info, Review>,
}

// ---------------------------------------------------------------- events & errors

#[event]
pub struct HookCreated {
    pub hook: Pubkey,
    pub slug: String,
    pub authority: Pubkey,
}

#[event]
pub struct VersionPublished {
    pub hook: Pubkey,
    pub version: Pubkey,
    pub index: u32,
    pub semver: [u16; 3],
    pub impl_kind: u8,
    pub program: Pubkey,
    pub environments: u8,
    pub capabilities: u32,
    pub manifest_hash: [u8; 32],
    pub upgrade_policy: u8,
}

#[event]
pub struct VersionStatusChanged {
    pub version: Pubkey,
    pub status: u8,
}

#[event]
pub struct ReviewerUpdated {
    pub reviewer: Pubkey,
    pub active: bool,
}

#[event]
pub struct ReviewPosted {
    pub version: Pubkey,
    pub reviewer: Pubkey,
    pub kind: u8,
    pub named_reviewer: bool,
    pub evidence_hash: [u8; 32],
}

#[event]
pub struct ReviewRetracted {
    pub version: Pubkey,
    pub reviewer: Pubkey,
}

#[error_code]
pub enum RegistryError {
    #[msg("Signer is not authorized for this action")]
    Unauthorized,
    #[msg("Slug must be 3-32 chars of a-z, 0-9 and '-'")]
    InvalidSlug,
    #[msg("String too long or empty")]
    InvalidString,
    #[msg("Version must be greater than the latest version")]
    VersionNotIncreasing,
    #[msg("Version claims are invalid for the standard")]
    InvalidClaims,
    #[msg("Program account required for program versions")]
    MissingProgram,
    #[msg("Status transitions only move forward")]
    InvalidStatus,
    #[msg("Invalid review kind")]
    InvalidReview,
    #[msg("Only active named reviewers may post this review kind")]
    NotNamedReviewer,
    #[msg("Installation does not match the registry version")]
    VersionMismatch,
}
