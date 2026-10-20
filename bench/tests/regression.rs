//! Fails when a measured case regresses past the committed baseline, or a case disappears.

const TOLERANCE_PCT: u64 = 5;

#[test]
fn compute_units_stay_within_the_baseline() {
    let path = hook_bench::root().join("bench/baseline.json");
    let baseline: hook_bench::Results = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let now = hook_bench::measure();
    let mut failures = vec![];
    for b in baseline.unit.iter().chain(&baseline.transaction) {
        let Some(m) = now.get(&b.name) else {
            failures.push(format!("{}: case missing", b.name));
            continue;
        };
        if !m.ok {
            failures.push(format!("{}: execution failed", b.name));
        }
        let limit = b.compute_units + b.compute_units * TOLERANCE_PCT / 100;
        if m.compute_units > limit {
            failures.push(format!("{}: {} CU > baseline {} (+{}%)", b.name, m.compute_units, b.compute_units, TOLERANCE_PCT));
        }
    }
    assert!(failures.is_empty(), "compute regressions:\n{}", failures.join("\n"));
}

#[test]
fn every_case_succeeds_and_hooks_cost_more_than_none() {
    let r = hook_bench::measure();
    assert!(r.unit.iter().chain(&r.transaction).all(|m| m.ok), "{r:#?}");
    let none = r.get("transfer_checked: Token-2022, no hook").unwrap().compute_units;
    let empty = r.get("transfer_checked: token-hooks, empty policy").unwrap().compute_units;
    let seven = r.get("transfer_checked: token-hooks, 7 builtins").unwrap().compute_units;
    assert!(none < empty && empty < seven, "{none} {empty} {seven}");
}

#[test]
fn each_measured_builtin_actually_evaluates() {
    use hook_kernel::builtin::*;
    let m = hook_bench::mollusk();
    let p = hook_bench::Parties::default();
    for (name, e) in hook_bench::builtin_cases(&p) {
        if name == "accounting" {
            continue; // records only; never denies
        }
        let mut deny = e;
        match e.builtin_id {
            ALLOWLIST => deny.config[16..48].copy_from_slice(&[0xAA; 32]),
            DENYLIST => deny.config[8..40].copy_from_slice(&p.bob.to_bytes()),
            MAX_SIZE => deny.config[32..40].copy_from_slice(&99u64.to_le_bytes()),
            SPENDING_LIMIT => deny.config[32..40].copy_from_slice(&99u64.to_le_bytes()),
            TIME_WINDOW => deny.config[8..16].copy_from_slice(&(-1i64).to_le_bytes()),
            TIMELOCK => deny.config[8..16].copy_from_slice(&i64::MAX.to_le_bytes()),
            CREDENTIAL => deny.config[8..40].copy_from_slice(&[0xBB; 32]),
            id => panic!("unhandled builtin {id}"),
        }
        assert!(hook_bench::execute_case(&m, &p, name, &[e], None).ok, "{name} should pass");
        assert!(!hook_bench::execute_case(&m, &p, name, &[deny], None).ok, "{name} should deny");
    }
}
