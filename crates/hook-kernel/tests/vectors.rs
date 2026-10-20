//! Shared vectors for the TypeScript mirrors of the kernel (`packages/sdk/src/v1`,
//! `packages/hook-analyzer`). `UPDATE_VECTORS=1 cargo test -p hook-kernel --test vectors`
//! rewrites the file; otherwise the committed file must match the kernel.

use bytemuck::Zeroable;
use hook_kernel::builtin::{self, *};
use hook_standard::{Environment, StackEntry, IMPL_BUILTIN};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn entry<C: bytemuck::Pod>(id: u8, callbacks: u16, mandatory: bool, cfg: C) -> StackEntry {
    let mut e = StackEntry::zeroed();
    e.hook_version = [id; 32];
    e.impl_kind = IMPL_BUILTIN;
    e.builtin_id = id;
    e.mandatory = mandatory as u8;
    e.callbacks = callbacks;
    e.granted = builtin::required_caps(id);
    e.config[..core::mem::size_of::<C>()].copy_from_slice(bytemuck::bytes_of(&cfg));
    e
}

fn generate() -> String {
    let envs = [Environment::Token, Environment::Router, Environment::Pool, Environment::Account];
    let mut builtins = vec![];
    for id in 1..=SLOT_LIMIT {
        let cbs: Vec<String> = envs.iter().map(|e| supported_callbacks(id, *e).to_string()).collect();
        builtins.push(format!(
            "    {{ \"id\": {id}, \"caps\": {}, \"estimate\": {}, \"callbacks\": [{}] }}",
            required_caps(id),
            compute_estimate(id),
            cbs.join(", ")
        ));
    }
    let k = |b: u8| [b; 32];
    let entries: Vec<(&str, StackEntry)> = vec![
        ("allowlist", entry(ALLOWLIST, 1, true, AllowlistConfig { target: TARGET_COUNTERPARTY, source: SOURCE_INLINE, _pad: [0; 6], max_age_slots: 0, key_a: k(1), key_b: k(2), _rest: [0; 16] })),
        ("denylist", entry(DENYLIST, 1, true, DenylistConfig { target: TARGET_ACTOR, _pad: [0; 7], key_a: k(3), key_b: [0; 32], _rest: [0; 24] })),
        ("max-size", entry(MAX_SIZE, 1, false, MaxSizeConfig { asset: k(4), max_amount: 1_000_000, _rest: [0; 56] })),
        ("spending-limit", entry(SPENDING_LIMIT, 1, true, SpendingLimitConfig { asset: k(4), limit: 5_000, period_secs: 86_400, _pad: 0, _rest: [0; 48] })),
        ("time-window", entry(TIME_WINDOW, 1, false, TimeWindowConfig { not_before: 1_700_000_000, not_after: 1_900_000_000, daily_start: 3_600, daily_end: 7_200, _rest: [0; 72] })),
        ("timelock", entry(TIMELOCK, 9, true, TimelockConfig { delay_secs: 600, _pad: 0, not_before: 1_800_000_000, _rest: [0; 80] })),
        ("accounting", entry(ACCOUNTING, 13, false, [0u8; 96])),
        ("credential", entry(CREDENTIAL, 1, true, {
            let mut c = CredentialConfig::zeroed();
            c.target = TARGET_COUNTERPARTY;
            c.gatekeeper_network = k(5);
            c
        })),
        ("slot-limit", entry(SLOT_LIMIT, 4, false, SlotLimitConfig { max_per_slot: 3, flags: 0, _pad: [0; 3], _rest: [0; 88] })),
    ];
    let entries: Vec<String> =
        entries.iter().map(|(n, e)| format!("    {{ \"name\": \"{n}\", \"hex\": \"{}\" }}", hex(bytemuck::bytes_of(e)))).collect();
    format!("{{\n  \"builtins\": [\n{}\n  ],\n  \"entries\": [\n{}\n  ]\n}}\n", builtins.join(",\n"), entries.join(",\n"))
}

#[test]
fn typescript_vectors_match_the_kernel() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/sdk/test/vectors/kernel.json");
    let now = generate();
    if std::env::var("UPDATE_VECTORS").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &now).unwrap();
    }
    let committed = std::fs::read_to_string(&path).expect("run with UPDATE_VECTORS=1 once");
    assert_eq!(committed, now, "kernel changed: regenerate with UPDATE_VECTORS=1 and update the TS mirrors");
}
