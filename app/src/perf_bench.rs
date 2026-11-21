//! Large-session performance benchmark suite (H1).
//!
//! Measures population, filtering, statistics, facet aggregation, row projection
//! (cold & incremental warm +1 flow), and memory footprint at 10k / 100k flows.

use std::collections::HashMap;
use std::time::Instant;

use crate::i18n::Lang;
use crate::session::{Direction, FilteredStats, FlowDraft, Protocol, Session};
use crate::ui::{EffectivePathDisplay, RowCache, project_rows};

/// Latency and memory metrics collected for one session scale.
#[derive(Debug, Clone)]
pub struct BenchReport {
    pub flow_count: usize,
    pub retained_flows: usize,
    pub session_bytes: usize,
    pub bytes_per_flow: usize,
    pub populate_ms: f64,
    pub filter_empty_ms: f64,
    pub filter_structured_ms: f64,
    pub filter_body_ms: f64,
    pub filtered_stats_ms: f64,
    pub facets_ms: f64,
    pub project_cold_ms: f64,
    pub project_warm_plus_one_ms: f64,
    pub matched_structured: usize,
    pub matched_body: usize,
    pub facet_hosts: usize,
}

const METHODS: [&str; 4] = ["GET", "POST", "PUT", "DELETE"];
const PATHS: [&str; 12] = [
    "/v1/orders",
    "/v1/orders/checkout",
    "/v1/users/profile",
    "/v1/catalog/search",
    "/v1/cart/items",
    "/v1/payments/intent",
    "/v1/notifications",
    "/v1/analytics/events",
    "/v1/auth/refresh",
    "/v1/health",
    "/static/app.js",
    "/graphql",
];
const CLIENTS: [&str; 4] = [
    "127.0.0.1 (Chrome)",
    "127.0.0.1 (curl)",
    "127.0.0.1 (node)",
    "192.168.1.42",
];

fn append_synthetic_flow(session: &mut Session, idx: usize) {
    let host_idx = idx % 40;
    let host = format!("api-{host_idx:02}.example.com");
    let method = METHODS[idx % METHODS.len()];
    let path = PATHS[idx % PATHS.len()];
    let url = format!("https://{host}{path}?page={}", idx % 10);
    let protocol = if idx.is_multiple_of(3) {
        Protocol::Http2
    } else {
        Protocol::Http11
    };
    let id = session.open(FlowDraft {
        protocol,
        method: method.to_string(),
        host,
        path: path.to_string(),
        url,
        request_headers: vec![
            ("User-Agent".into(), "zniff-bench/1.0".into()),
            ("Accept".into(), "application/json".into()),
        ],
    });
    session.set_client_ip(id, CLIENTS[idx % CLIENTS.len()]);

    if idx % 50 == 49 {
        session.set_error(id, "connection reset by peer");
        return;
    }

    let status = match idx % 20 {
        19 => 502,
        18 => 500,
        17 => 404,
        16 => 400,
        15 => 304,
        14 => 201,
        _ => 200,
    };
    let (ct, body): (&str, String) = if path.ends_with(".js") {
        (
            "application/javascript; charset=utf-8",
            format!("console.log({idx});"),
        )
    } else if idx.is_multiple_of(5) {
        (
            "application/json; charset=utf-8",
            format!(r#"{{"order_id":"ord-{idx}","status":{status},"ok":true}}"#),
        )
    } else {
        (
            "application/json; charset=utf-8",
            format!(r#"{{"id":{idx},"status":{status}}}"#),
        )
    };
    session.set_response(
        id,
        status,
        vec![
            ("Content-Type".into(), ct.into()),
            ("X-Request-Id".into(), format!("req-{idx}")),
        ],
        Some(protocol),
    );
    let bytes = body.into_bytes();
    let len = bytes.len() as u64;
    session.append_body(id, Direction::Response, &bytes);
    session.add_transfer(id, Direction::Response, len);
    let _ = session.begin_finish(id, crate::clock::now());
}

/// Populates a deterministic synthetic session with `count` realistic flows.
pub fn populate_synthetic_session(count: usize) -> Session {
    let mut session = Session::default();
    for idx in 0..count {
        append_synthetic_flow(&mut session, idx);
    }
    session
}

fn median_ms(samples: &mut [f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn measure_median<T>(iterations: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let iters = iterations.max(1);
    let mut samples = Vec::with_capacity(iters);
    let mut last = None;
    for _ in 0..iters {
        let start = Instant::now();
        let val = f();
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        last = Some(val);
    }
    (median_ms(&mut samples), last.expect("at least 1 iteration"))
}

/// Runs the full large-session benchmark suite for `flow_count` flows.
pub fn run_benchmark(flow_count: usize, iterations: usize) -> BenchReport {
    let t_pop = Instant::now();
    let mut session = populate_synthetic_session(flow_count);
    let populate_ms = t_pop.elapsed().as_secs_f64() * 1000.0;

    let retained_flows = session.len();
    let session_bytes = session.memory_bytes();
    let bytes_per_flow = session_bytes.checked_div(retained_flows).unwrap_or(0);

    let (filter_empty_ms, matched_all) =
        measure_median(iterations, || session.matching_with_previous_start(""));

    let structured = "host:api-01.example.com|api-02.example.com status:200|5xx type:json";
    let (filter_structured_ms, matched_structured_vec) = measure_median(iterations, || {
        session.matching_with_previous_start(structured)
    });

    let (filter_body_ms, matched_body_vec) = measure_median(iterations, || {
        session.matching_with_previous_start("body:order_id")
    });

    let (filtered_stats_ms, _) = measure_median(iterations, || {
        let matched = session.matching_with_previous_start("status:5xx");
        FilteredStats::from_flows(matched.iter().filter_map(|(id, _)| session.get(*id)))
    });

    let (facets_ms, facets) = measure_median(iterations, || session.facets_matching(""));

    let mut cache = RowCache {
        lang: Lang::En,
        rows: HashMap::new(),
    };
    let t_cold = Instant::now();
    let _cold_rows = project_rows(
        &session,
        &matched_all,
        Lang::En,
        None,
        EffectivePathDisplay::HostAndPath,
        &mut cache,
    );
    let project_cold_ms = t_cold.elapsed().as_secs_f64() * 1000.0;

    // Simulate a live capture tick: 1 new flow arrives, then filter + project with warm cache.
    let (project_warm_plus_one_ms, _) = measure_median(iterations, || {
        let next_idx = session.len();
        append_synthetic_flow(&mut session, next_idx);
        let matched = session.matching_with_previous_start("");
        project_rows(
            &session,
            &matched,
            Lang::En,
            None,
            EffectivePathDisplay::HostAndPath,
            &mut cache,
        )
        .len()
    });

    BenchReport {
        flow_count,
        retained_flows,
        session_bytes,
        bytes_per_flow,
        populate_ms,
        filter_empty_ms,
        filter_structured_ms,
        filter_body_ms,
        filtered_stats_ms,
        facets_ms,
        project_cold_ms,
        project_warm_plus_one_ms,
        matched_structured: matched_structured_vec.len(),
        matched_body: matched_body_vec.len(),
        facet_hosts: facets.domains.len(),
    }
}

/// Formats benchmark reports into an aligned monospace summary table.
pub fn format_report_table(reports: &[BenchReport]) -> String {
    let mut out = String::new();
    out.push_str("zniff large-session benchmark (median ms)\n");
    out.push_str("============================================================================================================================\n");
    out.push_str(&format!(
        "{:>8} | {:>9} | {:>7} | {:>9} | {:>9} | {:>9} | {:>9} | {:>8} | {:>8} | {:>9} | {:>9}\n",
        "flows",
        "mem(MB)",
        "B/flow",
        "pop(ms)",
        "flt:all",
        "flt:expr",
        "flt:body",
        "stats",
        "facets",
        "proj:cold",
        "proj:+1"
    ));
    out.push_str("----------------------------------------------------------------------------------------------------------------------------\n");
    for r in reports {
        let mem_mb = r.session_bytes as f64 / (1024.0 * 1024.0);
        out.push_str(&format!(
            "{:>8} | {:>9.2} | {:>7} | {:>9.2} | {:>9.2} | {:>9.2} | {:>9.2} | {:>8.2} | {:>8.2} | {:>9.2} | {:>9.2}\n",
            r.retained_flows,
            mem_mb,
            r.bytes_per_flow,
            r.populate_ms,
            r.filter_empty_ms,
            r.filter_structured_ms,
            r.filter_body_ms,
            r.filtered_stats_ms,
            r.facets_ms,
            r.project_cold_ms,
            r.project_warm_plus_one_ms,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_benchmark_1000_flows_collects_all_metrics() {
        let report = run_benchmark(1_000, 2);
        assert_eq!(report.retained_flows, 1_000);
        assert!(report.session_bytes > 0);
        assert!(report.bytes_per_flow > 0);
        assert!(report.matched_structured > 0);
        assert!(report.matched_body > 0);
        assert_eq!(report.facet_hosts, 40);
        let table = format_report_table(&[report]);
        assert!(table.contains("zniff large-session benchmark"));
        assert!(table.contains("1000"));
        assert!(table.contains("proj:+1"));
    }
}
