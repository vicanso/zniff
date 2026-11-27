//! Sidecar byte-bounded LRU storage for per-flow live hook views (`onRequest` /
//! `onResponse` / `onPreview`), keeping [`crate::session::Flow`] zero-bloat.

use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::hook_spec::PreviewCardNode;
use crate::hook_view_spec::estimate_view_nodes_bytes;

const MAX_STORE_FLOWS: usize = 256;
const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;

/// Per-flow views inserted into Overview, Request, and Response detail regions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowHookViews {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overview: Vec<PreviewCardNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub req: Vec<PreviewCardNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub res: Vec<PreviewCardNode>,
}

impl FlowHookViews {
    pub fn is_empty(&self) -> bool {
        self.overview.is_empty() && self.req.is_empty() && self.res.is_empty()
    }

    pub fn estimated_bytes(&self) -> usize {
        let mut total = 64usize;
        if !self.overview.is_empty() {
            total = total.saturating_add(estimate_view_nodes_bytes(&self.overview));
        }
        if !self.req.is_empty() {
            total = total.saturating_add(estimate_view_nodes_bytes(&self.req));
        }
        if !self.res.is_empty() {
            total = total.saturating_add(estimate_view_nodes_bytes(&self.res));
        }
        total
    }
}

/// Concatenates `base` and `extra` in execution order, skipping consecutive duplicates.
pub fn merge_view_nodes(
    base: &[PreviewCardNode],
    extra: &[PreviewCardNode],
) -> Vec<PreviewCardNode> {
    let mut out = Vec::with_capacity(base.len() + extra.len());
    for node in base.iter().chain(extra.iter()) {
        if out.last() != Some(node) {
            out.push(node.clone());
        }
    }
    out
}

#[derive(Default)]
struct StoreInner {
    total_bytes: usize,
    entries: VecDeque<(u64, FlowHookViews, usize)>,
}

impl StoreInner {
    fn touch_or_insert(&mut self, id: u64, mutate: impl FnOnce(&mut FlowHookViews)) {
        let mut views = if let Some(pos) = self.entries.iter().position(|(fid, _, _)| *fid == id) {
            let (_, existing, bytes) = self.entries.remove(pos).expect("valid index");
            self.total_bytes = self.total_bytes.saturating_sub(bytes);
            existing
        } else {
            FlowHookViews::default()
        };
        mutate(&mut views);
        if views.is_empty() {
            return;
        }
        let next_bytes = views.estimated_bytes();
        if next_bytes > MAX_STORE_BYTES {
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_STORE_FLOWS
                || self.total_bytes.saturating_add(next_bytes) > MAX_STORE_BYTES)
        {
            if let Some((_, _, old_bytes)) = self.entries.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(old_bytes);
            }
        }
        self.total_bytes = self.total_bytes.saturating_add(next_bytes);
        self.entries.push_back((id, views, next_bytes));
    }

    fn get_cloned(&mut self, id: u64) -> FlowHookViews {
        let Some(pos) = self.entries.iter().position(|(fid, _, _)| *fid == id) else {
            return FlowHookViews::default();
        };
        let (fid, views, bytes) = self.entries.remove(pos).expect("valid index");
        let cloned = views.clone();
        self.entries.push_back((fid, views, bytes));
        cloned
    }
}

/// Thread-safe sidecar store holding live hook views keyed by `flow_id`.
#[derive(Default)]
pub struct HookViewStore {
    inner: Mutex<StoreInner>,
}

impl HookViewStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append_side(&self, id: u64, is_response: bool, nodes: Vec<PreviewCardNode>) {
        if nodes.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.touch_or_insert(id, |views| {
            if is_response {
                views.res = merge_view_nodes(&views.res, &nodes);
            } else {
                views.req = merge_view_nodes(&views.req, &nodes);
            }
        });
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn set_all(&self, id: u64, next: FlowHookViews) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.touch_or_insert(id, |views| *views = next);
    }

    pub fn get(&self, id: u64) -> FlowHookViews {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_cloned(id)
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn drop_flow(&self, id: u64) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = guard.entries.iter().position(|(fid, _, _)| *fid == id)
            && let Some((_, _, bytes)) = guard.entries.remove(pos)
        {
            guard.total_bytes = guard.total_bytes.saturating_sub(bytes);
        }
    }

    pub fn retain_ids(&self, keep: &HashSet<u64>) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.entries.retain(|(fid, _, _)| keep.contains(fid));
        guard.total_bytes = guard.entries.iter().map(|(_, _, b)| *b).sum();
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.entries.clear();
        guard.total_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_spec::{HookViewAction, PreviewTone};

    #[test]
    fn store_merges_sides_dedups_and_evicts_by_flow_cap() {
        let store = HookViewStore::new();
        let req_badge = PreviewCardNode::Badge {
            tone: PreviewTone::Ok,
            text: "REQ".into(),
            action: HookViewAction::default(),
        };
        let res_badge = PreviewCardNode::Badge {
            tone: PreviewTone::Accent,
            text: "RES".into(),
            action: HookViewAction::default(),
        };
        store.append_side(1, false, vec![req_badge.clone()]);
        store.append_side(1, true, vec![res_badge.clone()]);
        let got = store.get(1);
        assert_eq!(got.req, vec![req_badge]);
        assert_eq!(got.res, vec![res_badge]);

        for id in 10..=(10 + MAX_STORE_FLOWS as u64 + 8) {
            store.append_side(id, false, vec![PreviewCardNode::Divider]);
        }
        assert!(store.get(1).is_empty(), "oldest flow must evict on cap");

        store.clear();
        assert!(store.get(10 + MAX_STORE_FLOWS as u64).is_empty());
    }
}
