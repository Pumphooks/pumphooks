//! Programmable-market core. Pools, vaults, LP shares, settlement and hook invocation. Pool
//! hooks are installed from `hook-registry` versions (Pool environment); optional builtin policy
//! stacks are evaluated by `hook-kernel`. Market behavior lives in hook programs
//! (`docs/engines/pool-manager.md`, `docs/engines/pool-hook-interface.md`).

#![allow(unexpected_cfgs, deprecated)]

use anchor_lang::prelude::*;

pub mod engine;
pub mod errors;
pub mod events;
pub mod hook;
pub mod instructions;
pub mod policy;
pub mod state;
pub mod token;

use instructions::*;

declare_id!("8L8VFMpzP7S3ZfSyr9vu5vDVy93jgfWVfEzY4RTadFzw");

const _: () = assert!(ID.to_bytes()[0] == hooks_interface::CORE_PROGRAM_ID.to_bytes()[0]);

#[program]
pub mod hooks_core {
    use super::*;

    // protocol
    pub fn initialize_protocol(ctx: Context<InitializeProtocol>, fee_authority: Pubkey) -> Result<()> {
        instructions::initialize_protocol(ctx, fee_authority)
    }
    pub fn update_protocol(ctx: Context<UpdateProtocol>, update: ProtocolUpdate) -> Result<()> {
        instructions::update_protocol(ctx, update)
    }
    pub fn accept_admin(ctx: Context<AcceptAdmin>) -> Result<()> {
        instructions::accept_admin(ctx)
    }
    pub fn withdraw_protocol_fees(ctx: Context<WithdrawProtocolFees>) -> Result<()> {
        instructions::withdraw_protocol_fees(ctx)
    }

    // markets
    pub fn initialize_pool<'info>(
        ctx: Context<'_, '_, 'info, 'info, InitializePool<'info>>,
        params: InitPoolParams,
        hook_data: Vec<u8>,
    ) -> Result<()> {
        instructions::initialize_pool(ctx, params, hook_data)
    }
    pub fn attach_policy<'info>(
        ctx: Context<'_, '_, 'info, 'info, AttachPolicy<'info>>,
        stack: Vec<u8>,
    ) -> Result<()> {
        instructions::attach_policy(ctx, stack)
    }
    pub fn swap<'info>(
        ctx: Context<'_, '_, 'info, 'info, Swap<'info>>,
        zero_for_one: bool,
        exact_in: bool,
        amount: u64,
        limit: u64,
        hook_data: Vec<u8>,
    ) -> Result<()> {
        instructions::swap(ctx, zero_for_one, exact_in, amount, limit, hook_data)
    }
    pub fn add_liquidity<'info>(
        ctx: Context<'_, '_, 'info, 'info, Liquidity<'info>>,
        max_0: u64,
        max_1: u64,
        min_shares: u64,
        hook_data: Vec<u8>,
    ) -> Result<()> {
        instructions::add_liquidity(ctx, max_0, max_1, min_shares, hook_data)
    }
    pub fn remove_liquidity<'info>(
        ctx: Context<'_, '_, 'info, 'info, Liquidity<'info>>,
        shares: u64,
        min_0: u64,
        min_1: u64,
        hook_data: Vec<u8>,
    ) -> Result<()> {
        instructions::remove_liquidity(ctx, shares, min_0, min_1, hook_data)
    }
    pub fn donate<'info>(
        ctx: Context<'_, '_, 'info, 'info, Donate<'info>>,
        amount_0: u64,
        amount_1: u64,
        hook_data: Vec<u8>,
    ) -> Result<()> {
        instructions::donate(ctx, amount_0, amount_1, hook_data)
    }

    // hook credit and hook-initiated operations
    pub fn credit_deposit(ctx: Context<CreditDeposit>, amount_0: u64, amount_1: u64) -> Result<()> {
        instructions::credit_deposit(ctx, amount_0, amount_1)
    }
    pub fn credit_withdraw(ctx: Context<HookOp>, amount_0: u64, amount_1: u64) -> Result<()> {
        instructions::credit_withdraw(ctx, amount_0, amount_1)
    }
    pub fn credit_swap(ctx: Context<HookOp>, zero_for_one: bool, exact_in: bool, amount: u64, limit: u64, quote: u64) -> Result<()> {
        instructions::credit_swap(ctx, zero_for_one, exact_in, amount, limit, quote)
    }
    pub fn credit_donate(ctx: Context<HookOp>, amount_0: u64, amount_1: u64) -> Result<()> {
        instructions::credit_donate(ctx, amount_0, amount_1)
    }
    pub fn set_fee(ctx: Context<SetFee>, fee_ppm: u32) -> Result<()> {
        instructions::set_fee(ctx, fee_ppm)
    }
}
