//! Large-session benchmark harness (`make bench`).
//!
//! Run with:
//!   `make bench`
//!   `ZNIFF_BENCH_SIZES=10000,100000 ZNIFF_BENCH_ITERS=5 make bench`

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let sizes: Vec<usize> = std::env::var("ZNIFF_BENCH_SIZES")
        .ok()
        .map(|raw| {
            raw.split(',')
                .filter_map(|s| s.trim().parse::<usize>().ok())
                .filter(|&n| n > 0)
                .collect()
        })
        .filter(|v: &Vec<usize>| !v.is_empty())
        .unwrap_or_else(|| vec![10_000, 100_000]);

    let iterations: usize = std::env::var("ZNIFF_BENCH_ITERS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(5);

    let reports: Vec<_> = sizes
        .iter()
        .map(|&count| zniff::perf_bench::run_benchmark(count, iterations))
        .collect();
    print!("{}", zniff::perf_bench::format_report_table(&reports));
}

#[cfg(target_arch = "wasm32")]
fn main() {}
