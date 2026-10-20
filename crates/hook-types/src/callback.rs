/// Hook lifecycle callbacks. Wire values are frozen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HookCallback {
    BeforeBuy = 0,
    AfterBuy = 1,
    BeforeSell = 2,
    AfterSell = 3,
    BeforeSwap = 4,
    AfterSwap = 5,
    // Reserved for future releases. Never renumber.
    BeforeLiquidity = 6,
    AfterLiquidity = 7,
    BeforeFeeAction = 8,
    AfterFeeAction = 9,
    OnGraduation = 10,
    OnStrategySignal = 11,
}

impl HookCallback {
    pub const ALL: [HookCallback; 12] = [
        HookCallback::BeforeBuy,
        HookCallback::AfterBuy,
        HookCallback::BeforeSell,
        HookCallback::AfterSell,
        HookCallback::BeforeSwap,
        HookCallback::AfterSwap,
        HookCallback::BeforeLiquidity,
        HookCallback::AfterLiquidity,
        HookCallback::BeforeFeeAction,
        HookCallback::AfterFeeAction,
        HookCallback::OnGraduation,
        HookCallback::OnStrategySignal,
    ];

    /// Callbacks the v1 router dispatches.
    pub const V1_MASK: u16 = 0b11_1111;

    pub const fn from_u8(v: u8) -> Option<Self> {
        if (v as usize) < Self::ALL.len() {
            Some(Self::ALL[v as usize])
        } else {
            None
        }
    }

    pub const fn mask(self) -> u16 {
        1u16 << (self as u8)
    }

    pub const fn is_before(self) -> bool {
        matches!(
            self,
            HookCallback::BeforeBuy
                | HookCallback::BeforeSell
                | HookCallback::BeforeSwap
                | HookCallback::BeforeLiquidity
                | HookCallback::BeforeFeeAction
        )
    }

    pub const fn is_v1(self) -> bool {
        (self as u8) <= HookCallback::AfterSwap as u8
    }

    /// Callbacks dispatched for a routed trade, in order, for one phase.
    pub const fn phase(side: Side, before: bool) -> [HookCallback; 2] {
        match (side, before) {
            (Side::Buy, true) => [HookCallback::BeforeBuy, HookCallback::BeforeSwap],
            (Side::Buy, false) => [HookCallback::AfterBuy, HookCallback::AfterSwap],
            (Side::Sell, true) => [HookCallback::BeforeSell, HookCallback::BeforeSwap],
            (Side::Sell, false) => [HookCallback::AfterSell, HookCallback::AfterSwap],
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            HookCallback::BeforeBuy => "BeforeBuy",
            HookCallback::AfterBuy => "AfterBuy",
            HookCallback::BeforeSell => "BeforeSell",
            HookCallback::AfterSell => "AfterSell",
            HookCallback::BeforeSwap => "BeforeSwap",
            HookCallback::AfterSwap => "AfterSwap",
            HookCallback::BeforeLiquidity => "BeforeLiquidity",
            HookCallback::AfterLiquidity => "AfterLiquidity",
            HookCallback::BeforeFeeAction => "BeforeFeeAction",
            HookCallback::AfterFeeAction => "AfterFeeAction",
            HookCallback::OnGraduation => "OnGraduation",
            HookCallback::OnStrategySignal => "OnStrategySignal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Venue {
    BondingCurve = 0,
    PumpSwap = 1,
}

impl Venue {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Venue::BondingCurve),
            1 => Some(Venue::PumpSwap),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

impl Side {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Side::Buy),
            1 => Some(Side::Sell),
            _ => None,
        }
    }
}

/// How the user specified the trade. Maps 1:1 onto Pump instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TradeMode {
    /// `buy_v3(amount, max_sol_cost)` / amm `buy_v2(base_amount_out, max_quote_amount_in)`.
    ExactBaseOut = 0,
    /// `buy_exact_quote_in_v3` / amm `buy_exact_quote_in_v2`.
    ExactQuoteIn = 1,
    /// `sell_v3(amount, min_sol_output)` / amm `sell_v2(base_amount_in, min_quote_amount_out)`.
    ExactBaseIn = 2,
}

impl TradeMode {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(TradeMode::ExactBaseOut),
            1 => Some(TradeMode::ExactQuoteIn),
            2 => Some(TradeMode::ExactBaseIn),
            _ => None,
        }
    }

    pub const fn side(self) -> Side {
        match self {
            TradeMode::ExactBaseOut | TradeMode::ExactQuoteIn => Side::Buy,
            TradeMode::ExactBaseIn => Side::Sell,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_are_frozen() {
        let expected = [
            (HookCallback::BeforeBuy, 0),
            (HookCallback::AfterBuy, 1),
            (HookCallback::BeforeSell, 2),
            (HookCallback::AfterSell, 3),
            (HookCallback::BeforeSwap, 4),
            (HookCallback::AfterSwap, 5),
            (HookCallback::BeforeLiquidity, 6),
            (HookCallback::AfterLiquidity, 7),
            (HookCallback::BeforeFeeAction, 8),
            (HookCallback::AfterFeeAction, 9),
            (HookCallback::OnGraduation, 10),
            (HookCallback::OnStrategySignal, 11),
        ];
        for (cb, v) in expected {
            assert_eq!(cb as u8, v);
            assert_eq!(HookCallback::from_u8(v), Some(cb));
        }
        assert_eq!(HookCallback::from_u8(12), None);
        assert_eq!(HookCallback::from_u8(255), None);
    }

    #[test]
    fn v1_mask_covers_trade_callbacks_only() {
        for cb in HookCallback::ALL {
            assert_eq!(cb.is_v1(), cb.mask() & HookCallback::V1_MASK != 0);
        }
    }

    #[test]
    fn phases() {
        assert_eq!(
            HookCallback::phase(Side::Sell, true),
            [HookCallback::BeforeSell, HookCallback::BeforeSwap]
        );
        assert!(HookCallback::phase(Side::Buy, true).iter().all(|c| c.is_before()));
        assert!(HookCallback::phase(Side::Buy, false).iter().all(|c| !c.is_before()));
    }

    #[test]
    fn modes_map_to_sides() {
        assert_eq!(TradeMode::ExactQuoteIn.side(), Side::Buy);
        assert_eq!(TradeMode::ExactBaseIn.side(), Side::Sell);
        assert_eq!(TradeMode::from_u8(3), None);
    }
}
