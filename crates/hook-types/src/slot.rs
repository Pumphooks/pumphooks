use bytemuck::{Pod, Zeroable};

pub const SLOT_CONFIG_LEN: usize = 48;
pub const SLOT_STATE_LEN: usize = 48;

pub mod slot_flags {
    pub const ENABLED: u16 = 1 << 0;
    /// Required hooks fail closed. All v1 hooks are required.
    pub const REQUIRED: u16 = 1 << 1;
}

/// One installed module, stored inline in `MarketConfig`. 112 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
#[repr(C)]
pub struct HookSlot {
    pub module_id: u16,
    pub priority: u16,
    pub callbacks: u16,
    pub flags: u16,
    pub capabilities: u32,
    pub version: u16,
    pub _pad: u16,
    pub config: [u8; SLOT_CONFIG_LEN],
    pub state: [u8; SLOT_STATE_LEN],
}

const _: () = assert!(core::mem::size_of::<HookSlot>() == 112);

impl HookSlot {
    pub fn is_empty(&self) -> bool {
        self.module_id == 0
    }
    pub fn is_enabled(&self) -> bool {
        self.flags & slot_flags::ENABLED != 0
    }
    pub fn is_runtime(&self) -> bool {
        self.callbacks != 0
    }
    pub fn subscribes(&self, callback_mask: u16) -> bool {
        self.callbacks & callback_mask != 0
    }
}

/// Little-endian field access for fixed config/state blobs, so modules never transmute
/// unaligned bytes.
pub mod le {
    #[inline]
    pub fn u16_at(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    #[inline]
    pub fn u32_at(b: &[u8], o: usize) -> u32 {
        let mut a = [0u8; 4];
        a.copy_from_slice(&b[o..o + 4]);
        u32::from_le_bytes(a)
    }
    #[inline]
    pub fn u64_at(b: &[u8], o: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[o..o + 8]);
        u64::from_le_bytes(a)
    }
    #[inline]
    pub fn i64_at(b: &[u8], o: usize) -> i64 {
        u64_at(b, o) as i64
    }
    #[inline]
    pub fn u128_at(b: &[u8], o: usize) -> u128 {
        let mut a = [0u8; 16];
        a.copy_from_slice(&b[o..o + 16]);
        u128::from_le_bytes(a)
    }
    #[inline]
    pub fn put_u16(b: &mut [u8], o: usize, v: u16) {
        b[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn put_u32(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn put_u64(b: &mut [u8], o: usize, v: u64) {
        b[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn put_i64(b: &mut [u8], o: usize, v: i64) {
        put_u64(b, o, v as u64)
    }
    #[inline]
    pub fn put_u128(b: &mut [u8], o: usize, v: u128) {
        b[o..o + 16].copy_from_slice(&v.to_le_bytes());
    }
}
