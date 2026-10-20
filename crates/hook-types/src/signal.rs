use bytemuck::{Pod, Zeroable};

/// Domain separator prefixed to every signed StrategySignal so a signature cannot be replayed as
/// any other message.
pub const SIGNAL_DOMAIN: &[u8; 20] = b"PUMP_HOOKS_SIGNAL_V1";

/// Maximum clock skew allowed for `issued_at` in the future.
pub const MAX_SIGNAL_SKEW_SECS: i64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SignalAction {
    NoOp = 0,
    ChangeStrategyState = 1,
    QueueBuyback = 2,
    QueueTwap = 3,
    IncreaseReserveTarget = 4,
    DecreaseReserveTarget = 5,
    PauseStrategy = 6,
    ResumeStrategy = 7,
}

impl SignalAction {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::NoOp),
            1 => Some(Self::ChangeStrategyState),
            2 => Some(Self::QueueBuyback),
            3 => Some(Self::QueueTwap),
            4 => Some(Self::IncreaseReserveTarget),
            5 => Some(Self::DecreaseReserveTarget),
            6 => Some(Self::PauseStrategy),
            7 => Some(Self::ResumeStrategy),
            _ => None,
        }
    }
    pub const fn bit(self) -> u32 {
        1 << (self as u8)
    }
}

/// SENTIENT market regimes. Defensive regimes only restrict routed flow; none of them trade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Regime {
    Normal = 0,
    Cautious = 1,
    Defensive = 2,
    CircuitBreaker = 3,
}

impl Regime {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Cautious),
            2 => Some(Self::Defensive),
            3 => Some(Self::CircuitBreaker),
            _ => None,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Cautious => "CAUTIOUS",
            Self::Defensive => "DEFENSIVE",
            Self::CircuitBreaker => "CIRCUIT_BREAKER",
        }
    }
}

/// Off-chain proposal signed by the SENTIENT engine. The signed message is
/// `SIGNAL_DOMAIN ‖ bytes_of(StrategySignal)`. 96 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct StrategySignal {
    pub version: u8,
    pub action: u8,
    /// Action parameter (e.g. target `Regime` for `ChangeStrategyState`).
    pub param: u8,
    pub _pad: u8,
    pub confidence_bps: u16,
    pub amount_bps: u16,
    pub market: [u8; 32],
    pub nonce: u64,
    pub issued_at: i64,
    pub valid_until: i64,
    pub _reserved: [u8; 32],
}

const _: () = assert!(core::mem::size_of::<StrategySignal>() == 96);

/// Why a signal was rejected. Checked identically on-chain and in the SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalRejection {
    BadVersion,
    WrongMarket,
    UnknownAction,
    ActionNotPermitted,
    Replayed,
    Expired,
    FromFuture,
    ValidityTooLong,
    AmountTooLarge,
    BadParam,
}

pub struct SignalBounds {
    pub market: [u8; 32],
    pub last_nonce: u64,
    pub allowed_actions: u32,
    pub max_amount_bps: u16,
    pub max_validity_secs: i64,
}

impl StrategySignal {
    pub fn message(&self) -> [u8; 116] {
        let mut out = [0u8; 116];
        out[..20].copy_from_slice(SIGNAL_DOMAIN);
        out[20..].copy_from_slice(bytemuck::bytes_of(self));
        out
    }

    /// Bounds checks independent of signature verification.
    pub fn check(&self, b: &SignalBounds, now: i64) -> Result<SignalAction, SignalRejection> {
        if self.version != 1 {
            return Err(SignalRejection::BadVersion);
        }
        if self.market != b.market {
            return Err(SignalRejection::WrongMarket);
        }
        let action = SignalAction::from_u8(self.action).ok_or(SignalRejection::UnknownAction)?;
        if b.allowed_actions & action.bit() == 0 {
            return Err(SignalRejection::ActionNotPermitted);
        }
        if self.nonce <= b.last_nonce {
            return Err(SignalRejection::Replayed);
        }
        if self.issued_at > now.saturating_add(MAX_SIGNAL_SKEW_SECS) {
            return Err(SignalRejection::FromFuture);
        }
        if self.valid_until < now {
            return Err(SignalRejection::Expired);
        }
        if self.valid_until.saturating_sub(self.issued_at) > b.max_validity_secs {
            return Err(SignalRejection::ValidityTooLong);
        }
        if self.amount_bps > b.max_amount_bps || self.confidence_bps > 10_000 {
            return Err(SignalRejection::AmountTooLarge);
        }
        if action == SignalAction::ChangeStrategyState && Regime::from_u8(self.param).is_none() {
            return Err(SignalRejection::BadParam);
        }
        Ok(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> (StrategySignal, SignalBounds) {
        let s = StrategySignal {
            version: 1,
            action: SignalAction::ChangeStrategyState as u8,
            param: Regime::Defensive as u8,
            _pad: 0,
            confidence_bps: 8000,
            amount_bps: 0,
            market: [7; 32],
            nonce: 5,
            issued_at: 1000,
            valid_until: 1300,
            _reserved: [0; 32],
        };
        let b = SignalBounds {
            market: [7; 32],
            last_nonce: 4,
            allowed_actions: SignalAction::ChangeStrategyState.bit(),
            max_amount_bps: 500,
            max_validity_secs: 600,
        };
        (s, b)
    }

    #[test]
    fn accepts_valid() {
        let (s, b) = base();
        assert_eq!(s.check(&b, 1100), Ok(SignalAction::ChangeStrategyState));
    }

    #[test]
    fn rejects_replay_and_duplicate_nonce() {
        let (mut s, b) = base();
        s.nonce = 4;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::Replayed));
        s.nonce = 0;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::Replayed));
    }

    #[test]
    fn rejects_expired_and_future() {
        let (s, b) = base();
        assert_eq!(s.check(&b, 1301), Err(SignalRejection::Expired));
        assert_eq!(s.check(&b, 1000 - MAX_SIGNAL_SKEW_SECS - 1), Err(SignalRejection::FromFuture));
    }

    #[test]
    fn rejects_wrong_market_action_amount() {
        let (mut s, b) = base();
        s.market = [8; 32];
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::WrongMarket));
        let (mut s, b) = base();
        s.action = SignalAction::QueueBuyback as u8;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::ActionNotPermitted));
        let (mut s, b) = base();
        s.action = 99;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::UnknownAction));
        let (mut s, b) = base();
        s.amount_bps = 501;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::AmountTooLarge));
        let (mut s, b) = base();
        s.valid_until = s.issued_at + 601;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::ValidityTooLong));
        let (mut s, b) = base();
        s.param = 9;
        assert_eq!(s.check(&b, 1100), Err(SignalRejection::BadParam));
    }

    #[test]
    fn message_is_domain_separated() {
        let (s, _) = base();
        let m = s.message();
        assert_eq!(&m[..20], SIGNAL_DOMAIN);
        assert_eq!(&m[20..], bytemuck::bytes_of(&s));
    }
}
