//! Cross-environment typed state records (`docs/STANDARD.md` §7).
//!
//! A record is an account owned by its producer program, starting with [`StateHeader`]. Consumers
//! accept it only after [`verify_record`]: owner = configured producer, schema and namespace match,
//! freshness, and (optionally) a strictly increasing sequence for replay protection.

use bytemuck::{Pod, Zeroable};
use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

pub const MAGIC: [u8; 4] = *b"PHST";
pub const HEADER_LEN: usize = core::mem::size_of::<StateHeader>();

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq, Eq)]
pub struct StateHeader {
    pub magic: [u8; 4],
    pub version: u8,
    pub _pad: u8,
    pub schema: u16,
    pub namespace: [u8; 32],
    pub producer: [u8; 32],
    pub subject: [u8; 32],
    pub issuer: [u8; 32],
    pub slot: u64,
    pub timestamp: i64,
    pub sequence: u64,
}
const _: () = assert!(HEADER_LEN == 160);

/// `sha256(tag)`, e.g. `namespace(b"allowlist:v1")`.
pub fn namespace(tag: &[u8]) -> [u8; 32] {
    solana_program::hash::hash(tag).to_bytes()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    WrongOwner,
    Malformed,
    WrongSchema,
    WrongNamespace,
    WrongSubject,
    Stale,
    Replayed,
}

/// What a consumer requires of a record. Every field comes from the consumer's own configuration,
/// never from the record.
#[derive(Clone, Copy, Debug)]
pub struct Expect<'a> {
    pub producer: &'a Pubkey,
    pub schema: u16,
    pub namespace: &'a [u8; 32],
    /// Required subject, if any.
    pub subject: Option<&'a Pubkey>,
    /// Maximum age in slots (0 = no freshness requirement).
    pub max_age_slots: u64,
    pub current_slot: u64,
    /// If set, the record's sequence must be strictly greater.
    pub min_sequence_exclusive: Option<u64>,
}

pub fn verify_bytes(
    owner: &Pubkey,
    data: &[u8],
    expect: &Expect,
) -> Result<StateHeader, StateError> {
    if owner != expect.producer {
        return Err(StateError::WrongOwner);
    }
    let h: StateHeader =
        bytemuck::try_pod_read_unaligned(data.get(..HEADER_LEN).ok_or(StateError::Malformed)?)
            .map_err(|_| StateError::Malformed)?;
    if h.magic != MAGIC
        || h.version != crate::ABI_VERSION
        || h.producer != expect.producer.to_bytes()
    {
        return Err(StateError::Malformed);
    }
    if h.schema != expect.schema {
        return Err(StateError::WrongSchema);
    }
    if &h.namespace != expect.namespace {
        return Err(StateError::WrongNamespace);
    }
    if let Some(s) = expect.subject {
        if h.subject != s.to_bytes() {
            return Err(StateError::WrongSubject);
        }
    }
    if expect.max_age_slots != 0
        && expect.current_slot.saturating_sub(h.slot) > expect.max_age_slots
    {
        return Err(StateError::Stale);
    }
    if let Some(min) = expect.min_sequence_exclusive {
        if h.sequence <= min {
            return Err(StateError::Replayed);
        }
    }
    Ok(h)
}

pub fn verify_record(info: &AccountInfo, expect: &Expect) -> Result<StateHeader, StateError> {
    let data = info.try_borrow_data().map_err(|_| StateError::Malformed)?;
    verify_bytes(info.owner, &data, expect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(producer: &Pubkey, subject: &Pubkey, slot: u64, seq: u64) -> Vec<u8> {
        let mut h = StateHeader::zeroed();
        h.magic = MAGIC;
        h.version = 1;
        h.schema = 1;
        h.namespace = namespace(b"allowlist:v1");
        h.producer = producer.to_bytes();
        h.subject = subject.to_bytes();
        h.slot = slot;
        h.sequence = seq;
        let mut v = bytemuck::bytes_of(&h).to_vec();
        v.extend_from_slice(&[1]);
        v
    }

    #[test]
    fn verification_rules() {
        let p = Pubkey::new_unique();
        let s = Pubkey::new_unique();
        let ns = namespace(b"allowlist:v1");
        let data = record(&p, &s, 100, 5);
        let base = Expect {
            producer: &p,
            schema: 1,
            namespace: &ns,
            subject: Some(&s),
            max_age_slots: 50,
            current_slot: 140,
            min_sequence_exclusive: Some(4),
        };
        assert!(verify_bytes(&p, &data, &base).is_ok());

        let other = Pubkey::new_unique();
        // Forged record: correct contents, different owner.
        assert_eq!(
            verify_bytes(&other, &data, &base),
            Err(StateError::WrongOwner)
        );
        assert_eq!(
            verify_bytes(&p, &data, &Expect { schema: 2, ..base }),
            Err(StateError::WrongSchema)
        );
        let ns2 = namespace(b"other");
        assert_eq!(
            verify_bytes(
                &p,
                &data,
                &Expect {
                    namespace: &ns2,
                    ..base
                }
            ),
            Err(StateError::WrongNamespace)
        );
        assert_eq!(
            verify_bytes(
                &p,
                &data,
                &Expect {
                    subject: Some(&other),
                    ..base
                }
            ),
            Err(StateError::WrongSubject)
        );
        assert_eq!(
            verify_bytes(
                &p,
                &data,
                &Expect {
                    current_slot: 151,
                    ..base
                }
            ),
            Err(StateError::Stale)
        );
        assert_eq!(
            verify_bytes(
                &p,
                &data,
                &Expect {
                    min_sequence_exclusive: Some(5),
                    ..base
                }
            ),
            Err(StateError::Replayed)
        );
        assert_eq!(
            verify_bytes(&p, &data[..100], &base),
            Err(StateError::Malformed)
        );
        // Header claims a different producer than the owner.
        let spoof = record(&other, &s, 100, 5);
        assert_eq!(verify_bytes(&p, &spoof, &base), Err(StateError::Malformed));
    }
}
