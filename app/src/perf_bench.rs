//! Large-session performance benchmark suite (H1).
//!
//! Measures population, filtering, statistics, facet aggregation, row projection
//! (cold & incremental warm +1 flow), and memory footprint at 10k / 100k flows.

use std::time::Instant;

use crate::i18n::Lang;
use crate::layout_spec::{LayoutContext, LayoutSpec, LayoutTarget};
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

/// Latency metrics for TSX transpilation, cold vs. warm QuickJS evaluation, and layout fast paths.
#[derive(Debug, Clone)]
pub struct ScriptBenchReport {
    pub tsx_transpile_us: f64,
    pub qjs_cold_eval_ms: f64,
    pub qjs_warm_eval_ms: f64,
    pub qjs_hook_warm_us: f64,
    pub topology_diff_us: f64,
    pub active_cache_hit_us: f64,
    pub warm_speedup: f64,
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

    let mut cache = RowCache::new(Lang::En);
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

const BENCH_LAYOUT_TSX: &str = r#"
import { Layout, Row, Col, Pane } from "zniff/layout";
import type { LayoutContext } from "zniff/layout";

export default ({ width, target }: LayoutContext) => (
  <Layout name="Bench Focus" footer={target === "web" ? "hidden" : "bottom"}>
    {width < 920 ? (
      <Col>
        <Pane name="flow_list" size="46%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="sidebar" size={220} />
        <Col>
          <Pane name="flow_list" size="1fr" />
          <Pane name="summary" size={156} />
        </Col>
        <Pane name="detail" size={width >= 1280 ? "52%" : "45%"} />
      </Row>
    )}
  </Layout>
);
"#;

/// Benchmarks TSX transpilation, uncached cold QuickJS startup, warm worker reuse,
/// topology equality fast-path, and `.active-cache.json` cold-boot loading.
pub fn run_script_benchmark(iterations: usize) -> ScriptBenchReport {
    let iters = iterations.max(1);
    let inner_micro_loops = 200usize;
    let wide_ctx = LayoutContext {
        width: 1360.0,
        height: 840.0,
        target: LayoutTarget::Desktop,
    };
    let resize_ctx = LayoutContext {
        width: 1120.0,
        height: 840.0,
        target: LayoutTarget::Desktop,
    };

    let (tsx_ms_batch, _) = measure_median(iters, || {
        let mut bytes = 0usize;
        for _ in 0..inner_micro_loops {
            let js = crate::layout_tsx::transpile_layout_tsx(BENCH_LAYOUT_TSX)
                .expect("bench TSX transpile");
            bytes ^= js.len();
        }
        bytes
    });
    let tsx_transpile_us = (tsx_ms_batch * 1000.0) / inner_micro_loops as f64;

    let (qjs_cold_eval_ms, spec_wide) = measure_median(iters, || {
        crate::layout_runtime::eval_layout_tsx_cold_ephemeral(BENCH_LAYOUT_TSX, wide_ctx)
            .expect("cold QuickJS eval")
    });

    // Prime the warm worker once before measuring warm steady-state evals.
    let spec_resized =
        crate::layout_runtime::eval_layout_tsx(BENCH_LAYOUT_TSX, resize_ctx).expect("warm prime");
    let (qjs_warm_eval_ms, _) = measure_median(iters, || {
        crate::layout_runtime::eval_layout_tsx(BENCH_LAYOUT_TSX, resize_ctx)
            .expect("warm QuickJS eval")
    });

    let (topo_ms_batch, _) = measure_median(iters, || {
        let mut same_count = 0usize;
        for _ in 0..inner_micro_loops {
            let t1 = LayoutSpec::topology(&spec_wide);
            let t2 = LayoutSpec::topology(&spec_resized);
            if t1 == t2 {
                same_count += 1;
            }
        }
        same_count
    });
    let topology_diff_us = (topo_ms_batch * 1000.0) / inner_micro_loops as f64;

    let tmp_dir = std::env::temp_dir().join(format!(
        "zniff-script-bench-{}-{}",
        std::process::id(),
        crate::clock::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    let _ = crate::layout_runtime::save_custom_layout(&tmp_dir, "bench-focus", BENCH_LAYOUT_TSX);
    crate::layout_runtime::store_cached_layout_spec(&tmp_dir, "bench-focus", wide_ctx, &spec_wide);

    let (cache_ms_batch, _) = measure_median(iters, || {
        let mut hits = 0usize;
        for _ in 0..inner_micro_loops {
            if crate::layout_runtime::try_load_cached_layout_spec(&tmp_dir, "bench-focus", wide_ctx)
                .is_some()
            {
                hits += 1;
            }
        }
        hits
    });
    let active_cache_hit_us = (cache_ms_batch * 1000.0) / inner_micro_loops as f64;
    let warm_speedup = if qjs_warm_eval_ms > 0.0 {
        qjs_cold_eval_ms / qjs_warm_eval_ms
    } else {
        0.0
    };

    // Measure warm onRequest hook evaluation with `zniff.flows.ids` + `responseHeader` lookup.
    let ca = std::sync::Arc::new(crate::ca::Ca::open(&tmp_dir).expect("bench ca"));
    let hub = std::sync::Arc::new(crate::proxy::Hub::new(ca).expect("bench hub"));
    let seed_id = hub.open(FlowDraft {
        protocol: Protocol::Http11,
        method: "POST".into(),
        host: "api.vicanso.dev".into(),
        path: "/v1/auth".into(),
        url: "https://api.vicanso.dev/v1/auth".into(),
        request_headers: vec![],
    });
    hub.mark_finished(seed_id);
    let hooks_dir = crate::hook_runtime::hooks_dir(&tmp_dir);
    let _ = std::fs::create_dir_all(&hooks_dir);
    let _ = std::fs::write(
        hooks_dir.join("bench-hook.tsx"),
        r#"
export function onRequest(ctx) {
  const ids = zniff.flows.ids("host:api.vicanso.dev", 5);
  return { headers: [...ctx.headers, ["x-seen", String(ids.length)]] };
}
"#,
    );
    let resolved_hook = crate::hook_runtime::resolve_script_hook(
        &tmp_dir,
        &[],
        "bench-hook",
        &std::collections::BTreeMap::new(),
        80,
    )
    .expect("resolve bench hook");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("bench tokio rt");
    let hook_in = crate::hook_runtime::HookRequestInputOwned {
        flow_id: 99,
        method: "GET".into(),
        scheme: "https".into(),
        host: "api.vicanso.dev".into(),
        url: "https://api.vicanso.dev/v1/items".into(),
        path: "/v1/items".into(),
        headers: vec![("Accept".into(), "application/json".into())],
        body: Vec::new(),
    };
    let _ = rt.block_on(crate::hook_runtime::eval_request_hook_async(
        hub.hook_session_host(),
        resolved_hook.clone(),
        hook_in.clone(),
    ));
    let (hook_warm_ms, _) = measure_median(iters, || {
        rt.block_on(crate::hook_runtime::eval_request_hook_async(
            hub.hook_session_host(),
            resolved_hook.clone(),
            hook_in.clone(),
        ))
        .expect("bench warm hook")
    });
    let qjs_hook_warm_us = hook_warm_ms * 1000.0;

    let _ = std::fs::remove_dir_all(&tmp_dir);

    ScriptBenchReport {
        tsx_transpile_us,
        qjs_cold_eval_ms,
        qjs_warm_eval_ms,
        qjs_hook_warm_us,
        topology_diff_us,
        active_cache_hit_us,
        warm_speedup,
    }
}

/// Formats script runtime benchmark results as a monospace summary table.
pub fn format_script_report_table(r: &ScriptBenchReport) -> String {
    let mut out = String::new();
    out.push_str("\nzniff script, layout & hook benchmark (median)\n");
    out.push_str("===========================================================================================================\n");
    out.push_str(&format!(
        "{:>12} | {:>12} | {:>12} | {:>12} | {:>10} | {:>12} | {:>14}\n",
        "transpile(us)",
        "qjs:cold(ms)",
        "qjs:warm(ms)",
        "hook:warm(us)",
        "speedup",
        "topo_cmp(us)",
        "boot_cache(us)"
    ));
    out.push_str("-----------------------------------------------------------------------------------------------------------\n");
    out.push_str(&format!(
        "{:>12.2} | {:>12.3} | {:>12.3} | {:>12.2} | {:>9.1}x | {:>12.2} | {:>14.2}\n",
        r.tsx_transpile_us,
        r.qjs_cold_eval_ms,
        r.qjs_warm_eval_ms,
        r.qjs_hook_warm_us,
        r.warm_speedup,
        r.topology_diff_us,
        r.active_cache_hit_us
    ));
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

    #[test]
    fn smoke_script_benchmark_collects_cold_warm_and_cache_metrics() {
        let report = run_script_benchmark(2);
        assert!(report.tsx_transpile_us >= 0.0);
        assert!(report.qjs_cold_eval_ms > 0.0);
        assert!(report.qjs_warm_eval_ms > 0.0);
        assert!(report.qjs_hook_warm_us > 0.0);
        assert!(report.warm_speedup > 0.0);
        let table = format_script_report_table(&report);
        assert!(table.contains("zniff script, layout & hook benchmark"));
        assert!(table.contains("hook:warm(us)"));
    }
}
