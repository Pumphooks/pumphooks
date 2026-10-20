//! `cargo run -p hook-bench --release [-- --baseline]`: measures, writes `bench/results.json` and
//! `docs/benchmarks/report.md`; `--baseline` also replaces `bench/baseline.json`.

fn main() {
    let r = hook_bench::measure();
    let root = hook_bench::root();
    let json = serde_json::to_string_pretty(&r).unwrap() + "\n";
    std::fs::write(root.join("bench/results.json"), &json).unwrap();
    std::fs::create_dir_all(root.join("docs/benchmarks")).unwrap();
    std::fs::write(root.join("docs/benchmarks/report.md"), hook_bench::report(&r)).unwrap();
    if std::env::args().any(|a| a == "--baseline") {
        std::fs::write(root.join("bench/baseline.json"), &json).unwrap();
    }
    print!("{}", hook_bench::report(&r));
}
