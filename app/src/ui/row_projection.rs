#[allow(unused_imports)]
use super::*;

pub(crate) fn mono(content: impl IntoReactive<SharedString>) -> Text {
    text(content).font_family(move || theme().font_mono.clone().unwrap_or_else(|| "Menlo".into()))
}

pub(crate) fn list_status(status: Option<u16>, failed: bool) -> String {
    match status {
        Some(code) => code.to_string(),
        None if failed => "ERR".to_string(),
        None => "—".to_string(),
    }
}

pub(crate) fn list_path(path: &str, error: Option<&str>, failed: bool, lang: Lang) -> String {
    if failed {
        localize_error(lang, error.unwrap_or(""))
    } else {
        path.split('?').next().unwrap_or(path).to_string()
    }
}

pub(crate) fn list_url(
    url: &str,
    path: &str,
    error: Option<&str>,
    failed: bool,
    lang: Lang,
) -> String {
    if failed {
        localize_error(lang, error.unwrap_or(""))
    } else if url.is_empty() {
        path.to_string()
    } else {
        url.to_string()
    }
}

pub(crate) struct CachedRow {
    pub(crate) rev: u64,
    pub(crate) prev: Option<SystemTime>,
    pub(crate) line: Arc<RowLine>,
}

pub(crate) struct RowCache {
    pub(crate) lang: Lang,
    pub(crate) rows: HashMap<u64, CachedRow>,
}

pub(crate) fn same_row_arcs(current: &[Arc<RowLine>], next: &[Arc<RowLine>]) -> bool {
    current.len() == next.len()
        && current
            .iter()
            .zip(next)
            .all(|(left, right)| Arc::ptr_eq(left, right))
}

pub(crate) fn project_rows(
    session: &session::Session,
    matched: &[(u64, Option<SystemTime>)],
    lang: Lang,
    sort: Option<ListSort>,
    full_url: bool,
    cache: &mut RowCache,
) -> Vec<Arc<RowLine>> {
    if cache.lang != lang {
        cache.rows.clear();
        cache.lang = lang;
    }
    let mut lines = Vec::with_capacity(matched.len());
    let mut keep = HashSet::with_capacity(matched.len());
    for (id, previous) in matched {
        keep.insert(*id);
        let Some(flow) = session.get(*id) else {
            continue;
        };
        if let Some(hit) = cache.rows.get(id)
            && hit.rev == flow.rev
            && hit.prev == *previous
        {
            lines.push(Arc::clone(&hit.line));
            continue;
        }
        let failed = flow.error.is_some() && flow.status.is_none();
        let gap_nanos = previous
            .and_then(|prev| flow.started.duration_since(prev).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let elapsed_nanos = flow
            .finished_at
            .and_then(|fin| fin.duration_since(flow.started).ok())
            .map(|duration| duration.as_nanos());
        let line = Arc::new(RowLine {
            id: *id,
            method: flow.method.clone(),
            status: flow.status,
            time: present::format_gap(*previous, flow.started),
            gap_nanos,
            clock: present::format_clock(flow.started),
            elapsed: present::format_elapsed(flow.started, flow.finished_at),
            elapsed_nanos,
            elapsed_slow: flow.finished_at.is_some()
                && present::is_slow(flow.started, flow.finished_at, clock::now()),
            host: flow.host.clone(),
            client_ip: if flow.client_ip.is_empty() {
                "—".to_string()
            } else {
                flow.client_ip.clone()
            },
            path: list_path(&flow.path, flow.error.as_deref(), failed, lang),
            url: list_url(&flow.url, &flow.path, flow.error.as_deref(), failed, lang),
            comment: flow.comment.clone(),
            protocol: flow.protocol.label_in(lang).to_string(),
            size: format_bytes(flow.response_bytes as usize),
            size_bytes: flow.response_bytes,
            failed,
            pinned: flow.pinned,
            relay_short: flow
                .relay_source
                .as_deref()
                .map(session::relay_short)
                .unwrap_or_default(),
        });
        cache.rows.insert(
            *id,
            CachedRow {
                rev: flow.rev,
                prev: *previous,
                line: Arc::clone(&line),
            },
        );
        lines.push(line);
    }
    cache.rows.retain(|id, _| keep.contains(id));
    if let Some(sort) = sort {
        sort_row_lines(&mut lines, sort, full_url);
    }
    lines
}

pub(crate) fn sort_row_lines<T: std::borrow::Borrow<RowLine>>(
    rows: &mut [T],
    sort: ListSort,
    full_url: bool,
) {
    rows.sort_by(|a, b| {
        let a = a.borrow();
        let b = b.borrow();
        let primary = match sort.column {
            ListColumn::Id => a.id.cmp(&b.id),
            ListColumn::Method => cmp_ignore_ascii_case(&a.method, &b.method),
            ListColumn::Path => {
                cmp_ignore_ascii_case(row_target_text(a, full_url), row_target_text(b, full_url))
            }
            ListColumn::Status => status_sort_rank(a).cmp(&status_sort_rank(b)),
            ListColumn::Gap => a.gap_nanos.cmp(&b.gap_nanos),
            ListColumn::Elapsed => match (a.elapsed_nanos, b.elapsed_nanos) {
                (Some(da), Some(db)) => da.cmp(&db),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
            ListColumn::Size => a.size_bytes.cmp(&b.size_bytes),
            ListColumn::Protocol => cmp_ignore_ascii_case(&a.protocol, &b.protocol),
            ListColumn::Host => cmp_ignore_ascii_case(&a.host, &b.host),
            ListColumn::ClientIp => match (a.client_ip == "—", b.client_ip == "—") {
                (false, false) => cmp_ignore_ascii_case(&a.client_ip, &b.client_ip),
                (false, true) => return std::cmp::Ordering::Less,
                (true, false) => return std::cmp::Ordering::Greater,
                (true, true) => std::cmp::Ordering::Equal,
            },
        };
        let ordered = if sort.desc {
            primary.reverse()
        } else {
            primary
        };
        ordered.then_with(|| a.id.cmp(&b.id))
    });
}

pub(crate) fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(b.bytes().map(|byte| byte.to_ascii_lowercase()))
        .then_with(|| a.cmp(b))
}

pub(crate) fn status_sort_rank(row: &RowLine) -> (u8, u16) {
    match (row.status, row.failed) {
        (Some(code), _) => (0, code),
        (None, true) => (1, u16::MAX),
        (None, false) => (2, u16::MAX),
    }
}
