use super::*;
use crate::layout_spec::{LayoutTarget, LayoutTopology};
use std::sync::Mutex;

static TEST_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

fn lock_lifecycle_tests() -> std::sync::MutexGuard<'static, ()> {
    TEST_LIFECYCLE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

const SAMPLE_TSX: &str = r#"
import { Layout, Row, Col, Pane } from "zniff/layout";
export default ({ width, target }: LayoutContext) => (
  <Layout name="Worker QA" footer="bottom">
    {width < 900 || target === "web" ? (
      <Col>
        <Pane name="flow_list" size="50%" />
        <Pane name="detail" size="1fr" />
      </Col>
    ) : (
      <Row>
        <Pane name="sidebar" size={220} />
        <Pane name="flow_list" size="1fr" />
        <Pane name="detail" size="48%" />
      </Row>
    )}
  </Layout>
);
"#;

#[test]
fn byte_bounded_lru_enforces_byte_and_entry_limits_and_clears() {
    let mut cache = ByteBoundedLru::new(10, 3);
    assert!(!cache.insert("too-big".to_string(), vec![0_u8; 11], 11));
    assert!(cache.is_empty());

    assert!(cache.insert("a".to_string(), vec![1; 4], 4));
    assert!(cache.insert("b".to_string(), vec![2; 4], 4));
    assert_eq!(cache.total_bytes(), 8);
    assert_eq!(cache.len(), 2);

    // Touch `a` so `b` becomes LRU before inserting `c` (4 bytes -> exceeds 10 bytes cap).
    assert!(cache.get(&"a".to_string()).is_some());
    assert!(cache.insert("c".to_string(), vec![3; 4], 4));
    assert_eq!(cache.total_bytes(), 8);
    assert!(cache.get(&"b".to_string()).is_none());
    assert!(cache.get(&"a".to_string()).is_some());
    assert!(cache.get(&"c".to_string()).is_some());

    // Insert tiny entries to test entry count cap (max_entries = 3).
    assert!(cache.insert("d".to_string(), vec![4; 1], 1));
    assert!(cache.insert("e".to_string(), vec![5; 1], 1));
    assert!(cache.len() <= 3);
    assert!(cache.total_bytes() <= 10);

    // Global packet cache honors clear_script_caches().
    assert_eq!(PACKET_MEM_LIMIT_BYTES, 16 * 1024 * 1024);
    assert_eq!(PACKET_STACK_LIMIT_BYTES, 256 * 1024);
    assert_eq!(PACKET_TIMEOUT_MS, 80);
    assert_eq!(PACKET_MAX_INPUT_BYTES, 1024 * 1024);
    assert_eq!(PACKET_MAX_OUTPUT_BYTES, 512 * 1024);
    assert!(store_packet_cache_entry("flow-1".into(), vec![9; 1024]));
    assert_eq!(
        get_packet_cache_entry("flow-1").map(|v| v.len()),
        Some(1024)
    );
    assert_eq!(packet_cache_stats(), (1, 1024));
    clear_script_caches();
    assert_eq!(packet_cache_stats(), (0, 0));
    assert!(get_packet_cache_entry("flow-1").is_none());
}

#[test]
fn warm_vm_reuse_shutdown_and_respawn_lifecycle() {
    let _guard = lock_lifecycle_tests();
    shutdown_script_worker_blocking();
    assert!(!is_worker_running());

    let ctx = LayoutContext {
        width: 1280.0,
        height: 800.0,
        target: LayoutTarget::Desktop,
    };

    let init_before = vm_init_count();
    let warm_before = warm_eval_count();

    let first = eval_layout_on_worker_blocking(None, SAMPLE_TSX, ctx).expect("first eval");
    assert!(is_worker_running());
    assert_eq!(first.name, "Worker QA");
    assert_eq!(vm_init_count(), init_before + 1);
    assert_eq!(warm_eval_count(), warm_before);

    // Subsequent calls reuse the warm VM without re-initializing QuickJS.
    for _ in 0..4 {
        let spec = eval_layout_on_worker_blocking(None, SAMPLE_TSX, ctx).expect("warm eval");
        assert!(matches!(spec.topology(), LayoutTopology::CustomRow(_)));
    }
    assert_eq!(vm_init_count(), init_before + 1);
    assert_eq!(warm_eval_count(), warm_before + 4);

    // Explicit shutdown terminates worker and drops VM; next call lazily respawns.
    shutdown_script_worker_blocking();
    assert!(!is_worker_running());

    let respawned = eval_layout_on_worker_blocking(None, SAMPLE_TSX, ctx).expect("respawn eval");
    assert_eq!(respawned.name, "Worker QA");
    assert!(is_worker_running());
    assert_eq!(vm_init_count(), init_before + 2);
}

#[test]
fn warm_vm_prevents_global_and_prototype_pollution_across_runs() {
    let _guard = lock_lifecycle_tests();
    let ctx = LayoutContext {
        width: 1280.0,
        height: 800.0,
        target: LayoutTarget::Desktop,
    };

    let mutator = r#"
export default () => {
  try { globalThis.sneaky = 42; } catch (_) {}
  try { Object.prototype.polluted = 99; } catch (_) {}
  const localConst = "Isolated";
  return <Layout name={localConst}><Row><Pane name="flow_list" size="1fr" /></Row></Layout>;
};
"#;
    let first = eval_layout_on_worker_blocking(None, mutator, ctx).expect("mutator script runs");
    assert_eq!(first.name, "Isolated");

    let verifier = r#"
export default () => {
  const clean = typeof globalThis.sneaky === "undefined" && ({}).polluted === undefined;
  return <Layout name={clean ? "Clean" : "Polluted"}><Row><Pane name="flow_list" size="1fr" /></Row></Layout>;
};
"#;
    let second = eval_layout_on_worker_blocking(None, verifier, ctx).expect("verifier script runs");
    assert_eq!(second.name, "Clean");
}

#[test]
fn three_consecutive_timeouts_trip_circuit_breaker_and_reset_recovers() {
    let _guard = lock_lifecycle_tests();
    let script_id = "unit_breaker_test";
    reset_script_circuit(script_id);

    let ctx = LayoutContext {
        width: 1200.0,
        height: 760.0,
        target: LayoutTarget::Desktop,
    };
    let infinite_loop = "export default () => { while (true) {} };";

    for strike in 1..=2 {
        let rep = eval_layout_report_blocking(Some(script_id), infinite_loop, ctx);
        assert!(rep.timed_out, "strike {strike} should time out");
        assert!(!rep.circuit_tripped, "strike {strike} must not trip yet");
        assert!(!is_circuit_tripped(script_id));
    }

    let rep3 = eval_layout_report_blocking(Some(script_id), infinite_loop, ctx);
    assert!(rep3.timed_out);
    assert!(rep3.circuit_tripped);
    assert!(is_circuit_tripped(script_id));

    // 4th call fails instantaneously without running QuickJS even if script is still bad or slow.
    let vm_before = vm_init_count();
    let warm_before = warm_eval_count();
    let t0 = Instant::now();
    let rep4 = eval_layout_report_blocking(Some(script_id), SAMPLE_TSX, ctx);
    assert!(t0.elapsed() < Duration::from_millis(5));
    assert!(rep4.circuit_tripped);
    assert!(!rep4.timed_out);
    assert!(rep4.result.unwrap_err().contains("circuit breaker"));
    assert_eq!(vm_init_count(), vm_before);
    assert_eq!(warm_eval_count(), warm_before);

    reset_script_circuit(script_id);
    assert!(!is_circuit_tripped(script_id));
    let recovered = eval_layout_on_worker_blocking(Some(script_id), SAMPLE_TSX, ctx)
        .expect("reset circuit recovers script");
    assert_eq!(recovered.name, "Worker QA");
    reset_all_script_circuits();
    assert!(!is_circuit_tripped(script_id));
}

#[test]
fn cargo_manifest_and_lockfile_guard_against_heavy_js_deps() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo_toml =
        std::fs::read_to_string(manifest_dir.join("Cargo.toml")).expect("read app/Cargo.toml");
    let cargo_lock =
        std::fs::read_to_string(manifest_dir.join("Cargo.lock")).expect("read app/Cargo.lock");

    // Ensure `rquickjs` stays gated behind `not(target_arch = "wasm32")` so wasm32 remains 0 B JS VM.
    let non_wasm_idx = cargo_toml
        .find("[target.'cfg(not(target_arch = \"wasm32\"))'.dependencies]")
        .expect("non-wasm dependency table");
    let rquickjs_idx = cargo_toml
        .find("rquickjs =")
        .expect("rquickjs dependency line");
    assert!(
        rquickjs_idx > non_wasm_idx,
        "rquickjs must live strictly under non-wasm32 target dependencies"
    );

    // Ensure `nsh-engine/js` and heavy compilers are NOT pulled into the dependency graph.
    for forbidden in [
        "name = \"nsh-js\"",
        "name = \"nsh-bundler\"",
        "name = \"oxc_parser\"",
        "name = \"swc_ecma_parser\"",
    ] {
        assert!(
            !cargo_lock.contains(forbidden),
            "forbidden heavy JS dependency `{forbidden}` detected in Cargo.lock"
        );
    }
}
