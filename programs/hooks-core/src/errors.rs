use anchor_lang::prelude::*;

#[error_code]
pub enum CoreError {
    #[msg("Signer is not the program's upgrade authority")]
    NotUpgradeAuthority,
    #[msg("Account is not the expected program data account")]
    InvalidProgramData,
    #[msg("Program is not deployed by a supported loader")]
    UnsupportedLoader,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Protocol fee share above cap")]
    ProtocolFeeTooHigh,
    #[msg("Operation paused by protocol")]
    Paused,
    #[msg("Invalid permission mask")]
    InvalidPermissions,
    #[msg("Pool permissions exceed what the hook version declares in hook-registry")]
    PermissionsNotDeclared,
    #[msg("Mints must be distinct and sorted (mint_0 < mint_1)")]
    MintOrder,
    #[msg("Mint uses a token extension the core does not support")]
    UnsupportedMintExtension,
    #[msg("Fee above maximum")]
    FeeTooHigh,
    #[msg("Hook program account missing or mismatched")]
    HookAccountMismatch,
    #[msg("Hook version missing, inactive, or not a Pool program hook for this program")]
    HookVersionMismatch,
    #[msg("Hook program was redeployed since the pool pinned it")]
    HookRedeployed,
    #[msg("Hook rejected the operation")]
    HookRejected,
    #[msg("Hook returned an invalid or unauthenticated result")]
    HookResultInvalid,
    #[msg("Hook returned a delta, fee or quote it is not permitted to return")]
    HookResultNotPermitted,
    #[msg("Hook data too long")]
    HookDataTooLong,
    #[msg("Hook quote out of range")]
    QuoteOutOfRange,
    #[msg("Hook delta exceeds the specified amount")]
    DeltaExceedsAmount,
    #[msg("Hook credit would go negative")]
    InsufficientCredit,
    #[msg("Slippage limit exceeded")]
    Slippage,
    #[msg("Amount must be positive")]
    ZeroAmount,
    #[msg("Insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Token transfer fee makes exact-out input ambiguous")]
    ExactOutTransferFee,
    #[msg("Vault balance does not match pool accounting")]
    VaultInvariant,
    #[msg("Wrong vault, mint or token program for this pool")]
    PoolAccountMismatch,
    #[msg("Invalid pool policy stack")]
    InvalidPolicy,
    #[msg("Policy can only be attached once, by the creator, before the first deposit")]
    PolicyLocked,
    #[msg("Pool policy account missing or mismatched")]
    PolicyMismatch,
    #[msg("Registry version for a policy entry not supplied")]
    MissingVersion,
    #[msg("Pool policy rejected the operation")]
    PolicyRejected,
}
