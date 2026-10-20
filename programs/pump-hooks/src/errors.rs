use anchor_lang::prelude::*;
use hook_runtime::HookError;

#[error_code]
pub enum PumpHooksError {
    #[msg("Signer is not the authority")]
    Unauthorized,
    #[msg("No pending authority transfer for this signer")]
    NoPendingAuthority,
    #[msg("Protocol is paused")]
    ProtocolPaused,
    #[msg("Market is paused")]
    MarketPaused,
    #[msg("Mint is not owned by a token program")]
    InvalidMint,
    #[msg("Account is not this mint's Pump bonding curve")]
    InvalidBondingCurve,
    #[msg("Pool is not the canonical PumpSwap pool for this mint")]
    InvalidPool,
    #[msg("Quote mint does not match the Pump market")]
    InvalidQuoteMint,
    #[msg("Cashback coins are not routable through Pump v2/v3 instructions")]
    CashbackCoinUnsupported,
    #[msg("Coin has migrated; call sync_market and route through PumpSwap")]
    MarketMigrated,
    #[msg("Coin has not migrated; route through the bonding curve")]
    MarketNotMigrated,
    #[msg("Wrong number of Pump accounts")]
    PumpAccountsMissing,
    #[msg("Pump account does not match the market")]
    PumpAccountMismatch,
    #[msg("User token account has the wrong owner or mint")]
    InvalidUserTokenAccount,
    #[msg("Unknown trade mode")]
    InvalidMode,
    #[msg("Zero amount")]
    ZeroAmount,
    #[msg("Observed balances moved in an impossible direction")]
    UnexpectedBalanceChange,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("A hook denied the routed trade")]
    HookDenied,
    #[msg("An after-callback check failed; trade reverted")]
    HookPostCondition,
    #[msg("Installed module is disabled by the protocol kill switch")]
    ModuleDisabled,
    #[msg("Hook acted outside its granted capabilities")]
    CapabilityViolation,
    #[msg("Unknown module")]
    UnknownModule,
    #[msg("Module is not available in this release")]
    ModuleUnavailable,
    #[msg("Hook definition is disabled")]
    HookDefinitionDisabled,
    #[msg("Invalid hook config")]
    BadHookConfig,
    #[msg("Priority outside the module's namespace")]
    BadPriority,
    #[msg("Callbacks not supported by the module")]
    BadCallbacks,
    #[msg("Capability not allowed for the module")]
    CapabilityNotAllowed,
    #[msg("Module already installed")]
    DuplicateModule,
    #[msg("No free hook slots")]
    TooManySlots,
    #[msg("Runtime hook limit reached")]
    TooManyRuntimeHooks,
    #[msg("Module not installed on this market")]
    SlotNotFound,
    #[msg("Policy account missing or out of order")]
    PolicyAccountMissing,
    #[msg("Invalid policy")]
    InvalidPolicy,
    #[msg("Policy index out of range")]
    BadPolicyIndex,
    #[msg("Install the Policy module before creating policies")]
    PolicyModuleNotInstalled,
    #[msg("Policy config is managed by policy instructions")]
    PolicyConfigManaged,
    #[msg("SENTIENT is not installed on this market")]
    SentientNotInstalled,
    #[msg("No matching ed25519 verification of the signal by the configured signer")]
    SignalNotVerified,
    #[msg("Signal rejected")]
    SignalRejected,
}

impl From<HookError> for PumpHooksError {
    fn from(e: HookError) -> Self {
        match e {
            HookError::UnknownModule(_) => Self::UnknownModule,
            HookError::ModuleUnavailable(_) => Self::ModuleUnavailable,
            HookError::ModuleDisabled(_) => Self::ModuleDisabled,
            HookError::BadConfig(_) => Self::BadHookConfig,
            HookError::BadPriority(_) => Self::BadPriority,
            HookError::BadCallbacks(_) => Self::BadCallbacks,
            HookError::CapabilityNotAllowed(_) => Self::CapabilityNotAllowed,
            HookError::CapabilityViolation(_) => Self::CapabilityViolation,
            HookError::DuplicateModule(_) => Self::DuplicateModule,
            HookError::TooManySlots => Self::TooManySlots,
            HookError::TooManyRuntimeHooks => Self::TooManyRuntimeHooks,
            HookError::SlotNotFound => Self::SlotNotFound,
            HookError::PolicyAccountMissing(_) => Self::PolicyAccountMissing,
            HookError::PostCondition { .. } => Self::HookPostCondition,
        }
    }
}

pub fn hook_err(e: HookError) -> Error {
    msg!("hook error: {:?}", e);
    PumpHooksError::from(e).into()
}
