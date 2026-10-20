use bytemuck::{Pod, Zeroable};

pub mod decision_flags {
    pub const STATE_WRITTEN: u32 = 1 << 0;
    pub const SIGNAL_EMITTED: u32 = 1 << 1;
    pub const POINTS_AWARDED: u32 = 1 << 2;
    /// The module was inactive for this callback (e.g. Launch Shield after its window).
    pub const INACTIVE: u32 = 1 << 3;
}

/// Deterministic hook result. 8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct HookDecision {
    pub allow: u8,
    pub _pad: u8,
    pub reason_code: u16,
    pub flags: u32,
}

const _: () = assert!(core::mem::size_of::<HookDecision>() == 8);

impl HookDecision {
    pub const fn allow() -> Self {
        Self { allow: 1, _pad: 0, reason_code: 0, flags: 0 }
    }
    pub const fn deny(reason_code: u16) -> Self {
        Self { allow: 0, _pad: 0, reason_code, flags: 0 }
    }
    pub const fn with_flags(mut self, flags: u32) -> Self {
        self.flags |= flags;
        self
    }
    pub const fn is_allowed(&self) -> bool {
        self.allow == 1
    }
}
