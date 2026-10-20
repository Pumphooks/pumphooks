//! Shared vectors for the TypeScript mirrors of the kernel (`packages/sdk/src/v1`,
//! `packages/hook-analyzer`). `UPDATE_VECTORS=1 cargo test -p hook-kernel --test vectors`
//! rewrites the file; otherwise the committed file must match the kernel.

use bytemuck::Zeroable;
use hook_kernel::builtin::{self, *};
use hook_standard::{Environment, HookStack, StackEntry, IMPL_BUILTIN};

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
        ("multisig", entry(MULTISIG, 1 << 2, true, MultisigConfig { threshold: 2, member_mask: 0b111, _rest: [0; 94] })),
        ("price-impact", entry(PRICE_IMPACT, 1 << 6, false, PriceImpactConfig { max_bps: 150, _pad: [0; 6], _rest: [0; 88] })),
        ("dynamic-fee", entry(DYNAMIC_FEE, 1 << 6, false, DynamicFeeConfig { mode: FEE_MODE_SIZE, _pad: [0; 3], base_bps: 30, max_bps: 100, pivot: 100_000_000, alpha_bps: 7, _pad2: 0, sensitivity_bps: 9, _rest: [0; 72] })),
    ];
    let stacks: Vec<String> = stack_cases(&entries)
        .iter()
        .map(|(n, s, members)| {
            let r = match hook_kernel::validate_stack(s, *members) {
                Ok(()) => "ok".to_string(),
                Err(e) => format!("{e:?}"),
            };
            format!("    {{ \"name\": \"{n}\", \"members\": {members}, \"result\": \"{r}\", \"hex\": \"{}\" }}", hex(bytemuck::bytes_of(s)))
        })
        .collect();
    let entries: Vec<String> =
        entries.iter().map(|(n, e)| format!("    {{ \"name\": \"{n}\", \"hex\": \"{}\" }}", hex(bytemuck::bytes_of(e)))).collect();
    format!(
        "{{\n  \"builtins\": [\n{}\n  ],\n  \"entries\": [\n{}\n  ],\n  \"stacks\": [\n{}\n  ]\n}}\n",
        builtins.join(",\n"),
        entries.join(",\n"),
        stacks.join(",\n")
    )
}

fn program(s: &mut HookStack, i: usize, cb: u16) {
    let e = &mut s.entries[i];
    *e = StackEntry::zeroed();
    e.hook_version = [40 + i as u8; 32];
    e.program = [9; 32];
    e.impl_kind = hook_standard::IMPL_PROGRAM;
    e.callbacks = cb;
    e.granted = hook_standard::cap::VALIDATE_OPERATION | hook_standard::cap::VETO_OPERATION;
}

/// A valid token stack and single-field mutations of it, each with the kernel's verdict.
fn stack_cases(entries: &[(&str, StackEntry)]) -> Vec<(String, HookStack, u8)> {
    let base = {
        let mut s = HookStack::empty(Environment::Token);
        let pick = ["allowlist", "max-size", "spending-limit", "timelock"];
        for (i, n) in pick.iter().enumerate() {
            s.entries[i] = entries.iter().find(|(x, _)| x == n).unwrap().1;
        }
        s.len = pick.len() as u8;
        s
    };
    type M = fn(&mut HookStack);
    let muts: Vec<(&str, M)> = vec![
        ("valid", |_| {}),
        ("bad-version", |s| s.version = 2),
        ("bad-environment", |s| s.environment = 9),
        ("too-long", |s| s.len = 9),
        ("no-callbacks", |s| s.entries[1].callbacks = 0),
        ("callback-outside-environment", |s| s.entries[1].callbacks = 1 << 5),
        ("callback-unsupported-by-builtin", |s| s.entries[2].callbacks = 1 << 2),
        ("depends-on-later", |s| s.entries[1].depends_on = 0b100),
        ("depends-on-self", |s| s.entries[1].depends_on = 0b10),
        ("depends-on-earlier", |s| s.entries[1].depends_on = 0b1),
        ("duplicate-version", |s| s.entries[2].hook_version = s.entries[0].hook_version),
        ("pad-set", |s| s.entries[0]._pad = 1),
        ("reserved-set", |s| s.entries[0]._reserved = 1),
        ("mandatory-2", |s| s.entries[0].mandatory = 2),
        ("unknown-builtin", |s| s.entries[1].builtin_id = 99),
        ("builtin-with-program", |s| s.entries[1].program = [1; 32]),
        ("builtin-pinned", |s| s.entries[1].pinned_slot = 5),
        ("wrong-capabilities", |s| s.entries[1].granted |= hook_standard::cap::PRODUCE_STATE),
        ("invalid-config-max-size-zero", |s| s.entries[1].config[32..40].copy_from_slice(&0u64.to_le_bytes())),
        ("invalid-config-period-zero", |s| s.entries[2].config[40..44].copy_from_slice(&0u32.to_le_bytes())),
        ("invalid-config-allowlist-empty", |s| s.entries[0].config[16..48].copy_from_slice(&[0; 32])),
        ("unknown-impl", |s| s.entries[0].impl_kind = 7),
        ("dirty-slot", |s| s.entries[6].builtin_id = 1),
        ("program-hook", |s| program(s, 4, 1)),
        ("program-hook-zero-program", |s| {
            program(s, 4, 1);
            s.entries[4].program = [0; 32];
        }),
        ("program-hook-with-builtin-id", |s| {
            program(s, 4, 1);
            s.entries[4].builtin_id = 1;
        }),
        ("program-hook-without-veto", |s| {
            program(s, 4, 1);
            s.entries[4].granted = hook_standard::cap::READ_ACCOUNT_STATE;
        }),
        ("program-hook-excess-caps", |s| {
            program(s, 4, 1);
            s.entries[4].granted |= hook_standard::cap::REQUEST_AUTHORIZED_TRANSFER;
        }),
        ("three-program-hooks-one-callback", |s| {
            program(s, 4, 1);
            program(s, 5, 1);
            program(s, 6, 1);
        }),
        ("program-hook-in-pool", |s| {
            *s = HookStack::empty(Environment::Pool);
            program(s, 0, 1 << 6);
            s.len = 1;
        }),
    ];
    let mut out = vec![];
    for (name, m) in muts {
        let mut s = base;
        m(&mut s);
        if name.starts_with("program-hook") && name != "program-hook-in-pool" || name.starts_with("three") {
            s.len = s.len.max(if name.starts_with("three") { 7 } else { 5 });
        }
        out.push((name.to_string(), s, 0));
    }
    let mut ms = HookStack::empty(Environment::Account);
    ms.entries[0] = entry(MULTISIG, 1 << 2, true, MultisigConfig { threshold: 2, member_mask: 0b111, _rest: [0; 94] });
    ms.len = 1;
    out.push(("multisig-3-members".into(), ms, 3));
    out.push(("multisig-2-members".into(), ms, 2));
    out
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
