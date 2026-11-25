//! Filter box prefix & value autocomplete from current session + inline syntax validation (D2).

#[allow(unused_imports)]
use super::*;

const CATALOG_CAP: usize = 24;
const SUGGESTION_CAP: usize = 8;

/// Session-derived values used to suggest filter completions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FilterSuggestionsCatalog {
    pub(crate) hosts: Vec<String>,
    pub(crate) statuses: Vec<String>,
    pub(crate) apps: Vec<String>,
    pub(crate) graphql_ops: Vec<String>,
    pub(crate) methods: Vec<String>,
    pub(crate) rule_tokens: Vec<String>,
    pub(crate) markers: Vec<String>,
    /// Per-rule-line hit counts keyed by `(lowercase_group_name, 1-based line_no)`.
    pub(crate) rule_line_hits: HashMap<(Option<String>, usize), usize>,
}

impl FilterSuggestionsCatalog {
    pub(crate) fn hits_for_rule_line(&self, group_name: Option<&str>, line_no: usize) -> usize {
        if line_no == 0 {
            return 0;
        }
        let key = (group_name.map(str::to_ascii_lowercase), line_no);
        self.rule_line_hits.get(&key).copied().unwrap_or(0)
    }

    pub(crate) fn from_flows<'a>(
        flows: impl IntoIterator<Item = &'a session::Flow>,
        facets: &session::SessionFacets,
    ) -> Self {
        let hosts: Vec<String> = facets
            .domains
            .iter()
            .take(CATALOG_CAP)
            .map(|entry| entry.label.clone())
            .collect();
        let apps: Vec<String> = facets
            .apps
            .iter()
            .take(CATALOG_CAP)
            .map(|entry| entry.token.clone())
            .collect();

        let mut status_counts: HashMap<String, usize> = HashMap::new();
        let mut method_counts: HashMap<String, usize> = HashMap::new();
        let mut op_counts: HashMap<String, usize> = HashMap::new();
        let mut rule_token_counts: HashMap<String, usize> = HashMap::new();
        let mut rule_line_hits: HashMap<(Option<String>, usize), usize> = HashMap::new();
        let mut markers: Vec<String> = Vec::new();

        for flow in flows {
            if flow.is_marker() {
                let label = flow.path.trim();
                if !label.is_empty() && !markers.iter().any(|m| m.eq_ignore_ascii_case(label)) {
                    markers.push(label.to_string());
                }
                continue;
            }
            if let Some(code) = flow.status {
                *status_counts.entry(code.to_string()).or_default() += 1;
            } else if flow.error.is_some() {
                *status_counts.entry("err".to_string()).or_default() += 1;
            }
            let method = flow.method.trim();
            if !method.is_empty() {
                *method_counts
                    .entry(method.to_ascii_uppercase())
                    .or_default() += 1;
            }
            if !flow.request_body.is_empty() || flow.path.contains('?') {
                for op in present::graphql_operations(&flow.path, &flow.request_body) {
                    if !op.is_empty() {
                        *op_counts.entry(op).or_default() += 1;
                    }
                }
            }
            if !flow.rule_hits.is_empty() {
                for hit in &flow.rule_hits {
                    if !hit.verb.is_empty() {
                        *rule_token_counts.entry(hit.verb.clone()).or_default() += 1;
                    }
                    if hit.line_no > 0 {
                        let key = (
                            hit.group_name.as_deref().map(str::to_ascii_lowercase),
                            hit.line_no,
                        );
                        *rule_line_hits.entry(key).or_default() += 1;
                        let line_tok = match hit.group_name.as_deref() {
                            Some(g) => format!("{g}:{}", hit.line_no),
                            None => hit.line_no.to_string(),
                        };
                        *rule_token_counts.entry(line_tok).or_default() += 1;
                    }
                }
            } else if !flow.rule_hit.is_empty() {
                for verb in flow.rule_hit.split_whitespace() {
                    *rule_token_counts.entry(verb.to_string()).or_default() += 1;
                }
            }
        }

        Self {
            hosts,
            statuses: sort_counts(status_counts),
            apps,
            graphql_ops: sort_counts(op_counts),
            methods: sort_counts(method_counts),
            rule_tokens: sort_counts(rule_token_counts),
            markers,
            rule_line_hits,
        }
    }
}

fn sort_counts(map: HashMap<String, usize>) -> Vec<String> {
    let mut items: Vec<(String, usize)> = map.into_iter().collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items
        .into_iter()
        .take(CATALOG_CAP)
        .map(|(k, _)| k)
        .collect()
}

struct PrefixMeta {
    prefix: &'static str,
    zh: &'static str,
    en: &'static str,
    flag_only: bool,
    canonical: bool,
}

const fn pmeta(
    prefix: &'static str,
    zh: &'static str,
    en: &'static str,
    flag_only: bool,
    canonical: bool,
) -> PrefixMeta {
    PrefixMeta {
        prefix,
        zh,
        en,
        flag_only,
        canonical,
    }
}

const PREFIX_SPECS: &[PrefixMeta] = &[
    pmeta(
        "host:",
        "精确或通配域名",
        "Exact or wildcard host",
        false,
        true,
    ),
    pmeta("path:", "路径前缀", "Path prefix", false, true),
    pmeta(
        "status:",
        "状态码 / 2xx-5xx / err",
        "Status code / 2xx-5xx / err",
        false,
        true,
    ),
    pmeta("method:", "HTTP 方法", "HTTP method", false, true),
    pmeta(
        "type:",
        "内容类型 (json, html…)",
        "Content-Type (json, html…)",
        false,
        true,
    ),
    pmeta(
        "app:",
        "来源应用或客户端 IP",
        "Source app or client IP",
        false,
        true,
    ),
    pmeta(
        "op:",
        "GraphQL 操作名",
        "GraphQL operation name",
        false,
        true,
    ),
    pmeta(
        "body:",
        "请求/响应正文或 WS 消息",
        "Request/response body or WS",
        false,
        true,
    ),
    pmeta(
        "header:",
        "请求/响应头名或值",
        "Header name or value",
        false,
        true,
    ),
    pmeta("note:", "流量备注", "Flow comment / note", false, true),
    pmeta(
        "repeat:",
        "批量重放批次或源 ID",
        "Batch repeat tag or source ID",
        false,
        true,
    ),
    pmeta(
        "rule:",
        "命中的规则行号或动词",
        "Matched rule line or verb",
        false,
        true,
    ),
    pmeta(
        "mark:",
        "会话时间标记段",
        "Session marker section",
        false,
        true,
    ),
    pmeta("pin:", "仅看星标流量", "Pinned flows only", true, true),
    pmeta(
        "slow:",
        "仅看耗时 ≥ 1s 流量",
        "Slow flows (≥ 1s) only",
        true,
        true,
    ),
    pmeta(
        "mute:",
        "排除域名或关键词",
        "Exclude host or keyword",
        false,
        true,
    ),
    pmeta("domain:", "同 host:", "Alias for host:", false, false),
    pmeta("ct:", "同 type:", "Alias for type:", false, false),
    pmeta("client:", "同 app:", "Alias for app:", false, false),
    pmeta("proc:", "同 app:", "Alias for app:", false, false),
    pmeta("ip:", "同 app:", "Alias for app:", false, false),
    pmeta("gql:", "同 op:", "Alias for op:", false, false),
    pmeta("operation:", "同 op:", "Alias for op:", false, false),
    pmeta("comment:", "同 note:", "Alias for note:", false, false),
];

const STANDARD_STATUSES: &[&str] = &["2xx", "3xx", "4xx", "5xx", "err", "200", "404", "500"];
const STANDARD_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD"];
const STANDARD_TYPES: &[&str] = &[
    "json", "html", "xml", "text", "js", "css", "image", "grpc", "protobuf", "ws", "sse",
];
const STANDARD_RULE_VERBS: &[&str] = &[
    "replace",
    "map-local",
    "map-remote",
    "header",
    "break",
    "break-res",
    "block",
    "delay",
    "cors",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilterSyntaxErrorKind {
    UnknownPrefix(String),
    InvalidStatus(String),
    MissingValue(String),
    UnexpectedValue(String),
    EmptyOrSegment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterSyntaxError {
    pub(crate) token: String,
    pub(crate) kind: FilterSyntaxErrorKind,
}

fn is_known_prefix(prefix_with_colon: &str, negated: bool) -> Option<&'static PrefixMeta> {
    if negated && prefix_with_colon == "mute:" {
        return None;
    }
    PREFIX_SPECS
        .iter()
        .find(|spec| spec.prefix == prefix_with_colon)
}

fn is_valid_status_term(term: &str) -> bool {
    let lower = term.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "err" | "error" | "fail" | "failed" | "失败" | "1xx" | "2xx" | "3xx" | "4xx" | "5xx"
    ) {
        return true;
    }
    lower
        .parse::<u16>()
        .is_ok_and(|code| (100..=599).contains(&code))
}

fn looks_like_url_or_host_port(token: &str, prefix: &str, rest: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ws://")
        || lower.starts_with("wss://")
        || token.starts_with('[')
    {
        return true;
    }
    // Treat `example.com:8080` or `localhost:3000` (all-digit port) as host:port, not a filter prefix.
    if prefix.contains('.') || !prefix.bytes().all(|b| b.is_ascii_alphabetic()) {
        return true;
    }
    if prefix.eq_ignore_ascii_case("localhost")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

/// Validates filter syntax and returns any inline syntax errors.
/// Trailing in-progress tokens like `host:` (with no trailing whitespace) are not flagged
/// so value autocomplete can guide the user without flashing a false error.
pub(crate) fn validate_filter_syntax(filter: &str) -> Vec<FilterSyntaxError> {
    let trailing_ws = filter.ends_with(char::is_whitespace);
    let tokens = split_filter_tokens(filter);
    let last_idx = tokens.len().saturating_sub(1);
    let mut errors = Vec::new();

    for (idx, token) in tokens.iter().enumerate() {
        let in_progress = idx == last_idx && !trailing_ws;
        if let Some(err) = validate_single_token(token, in_progress) {
            errors.push(err);
        }
    }
    errors
}

fn validate_single_token(token: &str, in_progress: bool) -> Option<FilterSyntaxError> {
    let (negated, body) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    if body.is_empty() {
        return (!in_progress).then(|| FilterSyntaxError {
            token: token.to_string(),
            kind: FilterSyntaxErrorKind::MissingValue("-".to_string()),
        });
    }
    let colon = body.find(':')?;
    let prefix = &body[..colon];
    let rest = &body[colon + 1..];
    if looks_like_url_or_host_port(body, prefix, rest) {
        return None;
    }
    let prefix_with_colon = format!("{}:", prefix.to_ascii_lowercase());
    let Some(spec) = is_known_prefix(&prefix_with_colon, negated) else {
        let display_prefix = if negated {
            format!("-{prefix_with_colon}")
        } else {
            prefix_with_colon
        };
        return Some(FilterSyntaxError {
            token: token.to_string(),
            kind: FilterSyntaxErrorKind::UnknownPrefix(display_prefix),
        });
    };
    if spec.flag_only {
        if !rest.is_empty() {
            return Some(FilterSyntaxError {
                token: token.to_string(),
                kind: FilterSyntaxErrorKind::UnexpectedValue(spec.prefix.to_string()),
            });
        }
        return None;
    }
    if rest.is_empty() {
        if spec.prefix == "repeat:"
            || spec.prefix == "rule:"
            || spec.prefix == "mark:"
            || in_progress
        {
            return None;
        }
        return Some(FilterSyntaxError {
            token: token.to_string(),
            kind: FilterSyntaxErrorKind::MissingValue(spec.prefix.to_string()),
        });
    }
    if rest.starts_with('|')
        || rest.starts_with(',')
        || rest.contains("||")
        || rest.contains(",,")
        || (!in_progress && (rest.ends_with('|') || rest.ends_with(',')))
    {
        return Some(FilterSyntaxError {
            token: token.to_string(),
            kind: FilterSyntaxErrorKind::EmptyOrSegment(token.to_string()),
        });
    }
    if spec.prefix == "status:" {
        for part in rest.split(['|', ',']).filter(|p| !p.is_empty()) {
            if !is_valid_status_term(part) {
                return Some(FilterSyntaxError {
                    token: token.to_string(),
                    kind: FilterSyntaxErrorKind::InvalidStatus(part.to_string()),
                });
            }
        }
    }
    None
}

pub(crate) fn format_filter_syntax_error(lang: Lang, err: &FilterSyntaxError) -> String {
    match &err.kind {
        FilterSyntaxErrorKind::UnknownPrefix(prefix) => match lang {
            Lang::Zh => format!("未知过滤前缀「{prefix}」"),
            Lang::En => format!("Unknown filter prefix \"{prefix}\""),
        },
        FilterSyntaxErrorKind::InvalidStatus(val) => match lang {
            Lang::Zh => format!("无效状态码「{val}」（支持 100–599、1xx–5xx、err）"),
            Lang::En => format!("Invalid status \"{val}\" (use 100–599, 1xx–5xx, or err)"),
        },
        FilterSyntaxErrorKind::MissingValue(prefix) => match lang {
            Lang::Zh => format!("「{prefix}」缺少取值"),
            Lang::En => format!("Missing value after \"{prefix}\""),
        },
        FilterSyntaxErrorKind::UnexpectedValue(prefix) => match lang {
            Lang::Zh => format!("「{prefix}」不需要附加取值"),
            Lang::En => format!("\"{prefix}\" does not take a value"),
        },
        FilterSyntaxErrorKind::EmptyOrSegment(tok) => match lang {
            Lang::Zh => format!("「{tok}」包含空的 OR 分隔项"),
            Lang::En => format!("Empty OR item in \"{tok}\""),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterSuggestion {
    /// Complete replacement for the active token (e.g. `host:`, `host:api.local`).
    pub(crate) insert_token: String,
    /// Primary text displayed in the popup list.
    pub(crate) label: String,
    /// Secondary category/description text.
    pub(crate) hint: String,
    /// True for completed values/flags (`host:api.local`, `pin:`), false for prefixes (`host:`).
    pub(crate) append_space: bool,
}

/// Computes prefix or session-value autocomplete suggestions for the trailing token in `input`.
pub(crate) fn suggest_filter_completions(
    input: &str,
    catalog: &FilterSuggestionsCatalog,
    lang: Lang,
) -> Vec<FilterSuggestion> {
    if input.is_empty() || input.ends_with(char::is_whitespace) {
        return PREFIX_SPECS
            .iter()
            .filter(|spec| spec.canonical)
            .map(|spec| prefix_suggestion("", spec, lang))
            .collect();
    }
    let token = active_token_slice(input);
    let (neg_str, body) = match token.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", token),
    };
    if let Some(colon) = body.find(':') {
        let prefix = body[..colon].to_ascii_lowercase();
        let rest = &body[colon + 1..];
        return suggest_values(neg_str, &prefix, rest, catalog, lang);
    }
    suggest_prefixes(neg_str, body, lang)
}

fn active_token_slice(input: &str) -> &str {
    match input.rfind(char::is_whitespace) {
        Some(pos) => &input[pos + 1..],
        None => input,
    }
}

fn prefix_suggestion(neg: &str, spec: &PrefixMeta, lang: Lang) -> FilterSuggestion {
    let insert_token = format!("{neg}{}", spec.prefix);
    FilterSuggestion {
        label: insert_token.clone(),
        insert_token,
        hint: t(lang, spec.zh, spec.en).to_string(),
        append_space: spec.flag_only,
    }
}

fn suggest_prefixes(neg: &str, partial: &str, lang: Lang) -> Vec<FilterSuggestion> {
    let query = partial.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for spec in PREFIX_SPECS {
        if !neg.is_empty() && spec.prefix == "mute:" {
            continue;
        }
        let name = spec.prefix.trim_end_matches(':');
        if query.is_empty() {
            if spec.canonical {
                starts.push(prefix_suggestion(neg, spec, lang));
            }
        } else if name.starts_with(&query) {
            starts.push(prefix_suggestion(neg, spec, lang));
        } else if spec.canonical && name.contains(&query) {
            contains.push(prefix_suggestion(neg, spec, lang));
        }
    }
    starts.extend(contains);
    if !query.is_empty() {
        starts.truncate(SUGGESTION_CAP);
    }
    starts
}

fn suggest_values(
    neg: &str,
    prefix: &str,
    rest: &str,
    catalog: &FilterSuggestionsCatalog,
    lang: Lang,
) -> Vec<FilterSuggestion> {
    let (or_head, partial) = match rest.rfind(['|', ',']) {
        Some(pos) => (&rest[..=pos], &rest[pos + 1..]),
        None => ("", rest),
    };
    let existing: Vec<String> = or_head
        .split(['|', ','])
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let (candidates, hint): (Vec<String>, &'static str) = match prefix {
        "host" | "domain" | "mute" => (
            catalog.hosts.clone(),
            t(lang, "当前会话域名", "Session host"),
        ),
        "status" => (
            merge_candidates(&catalog.statuses, STANDARD_STATUSES),
            t(lang, "状态码", "Status"),
        ),
        "app" | "proc" | "client" | "ip" => {
            (catalog.apps.clone(), t(lang, "当前会话应用", "Session app"))
        }
        "op" | "gql" | "operation" => (
            catalog.graphql_ops.clone(),
            t(lang, "GraphQL 操作", "GraphQL op"),
        ),
        "mark" => (
            catalog.markers.clone(),
            t(lang, "会话标记", "Session marker"),
        ),
        "method" => (
            merge_candidates(&catalog.methods, STANDARD_METHODS),
            t(lang, "HTTP 方法", "Method"),
        ),
        "type" | "ct" => (
            STANDARD_TYPES.iter().map(|s| (*s).to_string()).collect(),
            t(lang, "内容类型", "Content-Type"),
        ),
        "rule" => (
            merge_candidates(&catalog.rule_tokens, STANDARD_RULE_VERBS),
            t(lang, "命中规则", "Matched rule"),
        ),
        _ => return Vec::new(),
    };

    let query = partial.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for cand in candidates {
        let lower = cand.to_ascii_lowercase();
        if existing.contains(&lower) {
            continue;
        }
        if !query.is_empty() && lower == query {
            continue;
        }
        let (value_tok, item_hint) = if prefix == "mark" {
            let marker_tok = marker_filter_token(&cand);
            let stripped = marker_tok.strip_prefix("mark:").unwrap_or(&marker_tok);
            let full_hint = if stripped == cand {
                hint.to_string()
            } else {
                format!("{hint}: {cand}")
            };
            (stripped.to_string(), full_hint)
        } else {
            (cand.clone(), hint.to_string())
        };
        if existing.contains(&value_tok.to_ascii_lowercase()) {
            continue;
        }
        let item = FilterSuggestion {
            insert_token: format!("{neg}{prefix}:{or_head}{value_tok}"),
            label: cand,
            hint: item_hint,
            append_space: true,
        };
        if query.is_empty() || lower.starts_with(&query) {
            starts.push(item);
        } else if lower.contains(&query) {
            contains.push(item);
        }
    }
    starts.extend(contains);
    starts.truncate(SUGGESTION_CAP);
    starts
}

fn merge_candidates(primary: &[String], fallback: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(primary.len() + fallback.len());
    for item in primary {
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(item))
        {
            out.push(item.clone());
        }
    }
    for item in fallback {
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(item))
        {
            out.push((*item).to_string());
        }
    }
    out
}

/// Applies `suggestion` to `input`, replacing the trailing partial token (or appending after whitespace).
pub(crate) fn apply_filter_suggestion(input: &str, suggestion: &FilterSuggestion) -> String {
    let prefix = if input.is_empty() || input.ends_with(char::is_whitespace) {
        input
    } else {
        match input.rfind(char::is_whitespace) {
            Some(pos) => &input[..=pos],
            None => "",
        }
    };
    let mut out = String::with_capacity(prefix.len() + suggestion.insert_token.len() + 1);
    out.push_str(prefix);
    out.push_str(&suggestion.insert_token);
    if suggestion.append_space {
        out.push(' ');
    }
    out
}

pub(crate) fn filter_catalog_context() -> Signal<FilterSuggestionsCatalog> {
    use_context::<Signal<FilterSuggestionsCatalog>>()
        .unwrap_or_else(|| signal(FilterSuggestionsCatalog::default()))
}

/// Compact inline syntax error strip shown directly below a filter input when syntax errors exist.
pub(crate) fn filter_syntax_error_strip(value: Signal<String>) -> impl IntoElement {
    let langs = ui_langs();
    let first_error = computed(move || validate_filter_syntax(&value.get()).into_iter().next());
    when(
        move || first_error.get().is_some(),
        move || {
            view()
                .w_full()
                .px(px(LIST_INSET))
                .py_1()
                .flex_row()
                .items_center()
                .gap_1p5()
                .bg(status_err_bg())
                .border_b_1()
                .border_b_color(border_strong())
                .child(
                    text(move || -> SharedString {
                        let lang = ui_lang(langs);
                        first_error
                            .get()
                            .map(|err| format_filter_syntax_error(lang, &err))
                            .unwrap_or_default()
                            .into()
                    })
                    .number_of_lines(1)
                    .text_size(px(TEXT_XS))
                    .text_color(status_err_fg()),
                )
        },
    )
}

/// Compact inline syntax error text for inside popovers (like the primary Filter Expression field).
pub(crate) fn filter_syntax_error_inline(value: Signal<String>) -> impl IntoElement {
    let langs = ui_langs();
    let first_error = computed(move || validate_filter_syntax(&value.get()).into_iter().next());
    when(
        move || first_error.get().is_some(),
        move || {
            text(move || -> SharedString {
                let lang = ui_lang(langs);
                first_error
                    .get()
                    .map(|err| format_filter_syntax_error(lang, &err))
                    .unwrap_or_default()
                    .into()
            })
            .number_of_lines(1)
            .text_size(px(TEXT_XS))
            .text_color(status_err_fg())
        },
    )
}

/// Filter input with inline red syntax error border and anchored prefix/value autocomplete dropdown.
pub(crate) fn filter_assist_input(
    value: Signal<String>,
    placeholder: impl IntoReactive<String> + 'static,
    height: f32,
    pad_right_when_non_empty: f32,
    focus_ref: ElementRef,
) -> impl IntoElement {
    let langs = ui_langs();
    let catalog = filter_catalog_context();
    let anchor = element_ref();
    let open = signal(false);
    let selected_idx = signal(None::<usize>);
    let has_error = computed(move || !validate_filter_syntax(&value.get()).is_empty());
    let suggestions = computed(move || {
        let text = value.get();
        let lang = ui_lang(langs);
        catalog.with(|cat| suggest_filter_completions(&text, cat, lang))
    });

    let accept_index = move |idx: usize| {
        let picked = suggestions.with_untracked(|list| list.get(idx).cloned());
        if let Some(item) = picked {
            let next = apply_filter_suggestion(&value.get_untracked(), &item);
            value.set(next);
            selected_idx.set(None);
            open.set(!item.append_space);
            focus_ref.focus();
        }
    };

    view()
        .element_ref(&anchor)
        .w_full()
        .relative()
        .flex_row()
        .items_center()
        .child(
            input()
                .element_ref(&focus_ref)
                .value(value)
                .placeholder(placeholder)
                .placeholder_color(move || color(fg_muted()))
                .w_full()
                .h(px(height))
                .pl_2p5()
                .pr(move || {
                    if value.get().is_empty() {
                        px(10.0)
                    } else {
                        px(pad_right_when_non_empty)
                    }
                })
                .rounded(px(RADIUS_MD))
                .bg(bg_code())
                .border_1()
                .border_color(move || {
                    if has_error.get() {
                        status_err_fg()
                    } else {
                        border_control()
                    }
                })
                .focus(move |style| {
                    style.border_color(move || {
                        if has_error.get() {
                            status_err_fg()
                        } else {
                            accent_blue()
                        }
                    })
                })
                .text_size(px(TEXT_SM))
                .text_color(fg_primary())
                .on_focus(move || {
                    selected_idx.set(None);
                    open.set(true);
                })
                .on_change(move |text: &str| {
                    value.set(text.to_string());
                    selected_idx.set(None);
                    open.set(true);
                })
                .on_key_press(move |key: &str| {
                    if key == "Escape" {
                        open.set(false);
                        selected_idx.set(None);
                    } else if key == "ArrowDown" {
                        open.set(true);
                        let count = suggestions.with_untracked(Vec::len);
                        if count > 0 {
                            selected_idx.update(|s| {
                                *s = Some(match *s {
                                    Some(i) => (i + 1) % count,
                                    None => 0,
                                });
                            });
                        }
                    } else if key == "ArrowUp" {
                        open.set(true);
                        let count = suggestions.with_untracked(Vec::len);
                        if count > 0 {
                            selected_idx.update(|s| {
                                *s = Some(match *s {
                                    Some(0) | None => count - 1,
                                    Some(i) => i - 1,
                                });
                            });
                        }
                    } else if key == "Tab" && open.get_untracked() {
                        accept_index(selected_idx.get_untracked().unwrap_or(0));
                    }
                })
                .on_submit(move |_: &str| {
                    if open.get_untracked()
                        && let Some(idx) = selected_idx.get_untracked()
                    {
                        accept_index(idx);
                    } else {
                        open.set(false);
                        selected_idx.set(None);
                    }
                }),
        )
        .child(when(
            move || open.get() && !suggestions.with(Vec::is_empty),
            move || {
                overlay()
                    .anchor(&anchor)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.0))
                    .margin(px(8.0))
                    .on_press_outside(move |_: &Point<Points>| {
                        open.set(false);
                        selected_idx.set(None);
                    })
                    .on_cancel(move || {
                        open.set(false);
                        selected_idx.set(None);
                    })
                    .child(filter_autocomplete_popup(
                        suggestions,
                        selected_idx,
                        value,
                        open,
                        focus_ref,
                    ))
            },
        ))
}

fn filter_autocomplete_popup(
    suggestions: Memo<Vec<FilterSuggestion>>,
    selected_idx: Signal<Option<usize>>,
    value: Signal<String>,
    open: Signal<bool>,
    focus_ref: ElementRef,
) -> impl IntoElement {
    view()
        .w(px(300.0))
        .max_w(move || px((window().width.0 - 24.0).max(200.0)))
        .p_1()
        .rounded(px(RADIUS_LG))
        .bg(bg_card())
        .border_1()
        .border_color(border_strong())
        .shadow_lg()
        .flex_col()
        .child(scroll().w_full().max_h(px(260.0)).child(dynamic(move || {
            let items = suggestions.get();
            let mut col = view().w_full().flex_col().gap_0p5();
            for (idx, item) in items.into_iter().enumerate() {
                let pick = item.clone();
                let is_selected = move || selected_idx.get() == Some(idx);
                col = col.child(
                    button()
                        .selected(is_selected)
                        .w_full()
                        .h(px(26.0))
                        .px_2()
                        .rounded(px(RADIUS_MD))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .bg(move || {
                            if is_selected() {
                                accent_blue_soft()
                            } else {
                                AdaptiveColor::TRANSPARENT
                            }
                        })
                        .hover(|style| {
                            style.bg(move || {
                                if is_selected() {
                                    accent_blue_soft_hover()
                                } else {
                                    bg_control_hover()
                                }
                            })
                        })
                        .active(|style| style.bg(bg_control_active()))
                        .transition_colors()
                        .duration(MOTION_INSTANT_MS)
                        .ease_out()
                        .focus_visible(apply_focus_ring)
                        .accessibility_label(item.insert_token.clone())
                        .on_press(move |_: &()| {
                            let next = apply_filter_suggestion(&value.get_untracked(), &pick);
                            value.set(next);
                            selected_idx.set(None);
                            open.set(!pick.append_space);
                            focus_ref.focus();
                        })
                        .child(
                            mono(item.label)
                                .flex_1()
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail)
                                .text_size(px(TEXT_SM))
                                .text_color(move || {
                                    if is_selected() {
                                        accent_blue_text()
                                    } else {
                                        fg_primary()
                                    }
                                }),
                        )
                        .child(
                            text(item.hint)
                                .flex_shrink_0()
                                .number_of_lines(1)
                                .text_size(px(TEXT_XS))
                                .text_color(fg_muted()),
                        ),
                );
            }
            Some(col)
        })))
}
