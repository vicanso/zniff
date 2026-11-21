//! Code snippet generators for additional HTTP clients and languages:
//! HTTPie, PowerShell (`Invoke-RestMethod`), JavaScript `axios`, Kotlin
//! `OkHttp`, and Swift `URLSession`.

use std::fmt::Write as _;

use crate::present::shell_quote;
use crate::session::Flow;

/// Filters hop-by-hop and proxy-only headers that should not be re-emitted
/// in generated client snippets.
fn copy_headers(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case("host")
                && !name.eq_ignore_ascii_case("content-length")
                && !name.eq_ignore_ascii_case("connection")
                && !name.eq_ignore_ascii_case("proxy-connection")
                && !name.eq_ignore_ascii_case("proxy-authorization")
        })
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect()
}

fn utf8_body(flow: &Flow) -> Option<&str> {
    std::str::from_utf8(&flow.request_body)
        .ok()
        .filter(|text| !text.is_empty())
}

fn content_type<'a>(headers: &[(&'a str, &'a str)]) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| *value)
}

fn js_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn ps_string(text: &str) -> String {
    let needs_double = text
        .chars()
        .any(|ch| matches!(ch, '\n' | '\r' | '\t') || (ch as u32) < 0x20);
    if !needs_double {
        return format!("'{}'", text.replace('\'', "''"));
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '`' => out.push_str("``"),
            '"' => out.push_str("`\""),
            '$' => out.push_str("`$"),
            '\r' => out.push_str("`r"),
            '\n' => out.push_str("`n"),
            '\t' => out.push_str("`t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "$([char]0x{:02x})", c as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn kotlin_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn swift_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => {
                let _ = write!(out, "\\u{{{:02X}}}", c as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Generates an HTTPie CLI command (`http ...`).
pub fn httpie_command(flow: &Flow) -> String {
    let method = flow.method.to_ascii_uppercase();
    let mut parts = vec!["http".to_string(), method, shell_quote(&flow.url)];
    for (name, value) in copy_headers(&flow.request_headers) {
        let header = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}:{value}")
        };
        parts.push(shell_quote(&header));
    }
    if let Some(body) = utf8_body(flow) {
        parts.push("--raw".to_string());
        parts.push(shell_quote(body));
    }
    parts.join(" ")
}

/// Generates a PowerShell `Invoke-RestMethod` script.
pub fn powershell_command(flow: &Flow) -> String {
    let all_headers = copy_headers(&flow.request_headers);
    let ct = content_type(&all_headers);
    let headers: Vec<(&str, &str)> = all_headers
        .into_iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("content-type"))
        .collect();
    let mut lines = Vec::new();
    if !headers.is_empty() {
        lines.push("$headers = @{".to_string());
        for (name, value) in &headers {
            lines.push(format!("  {} = {}", ps_string(name), ps_string(value)));
        }
        lines.push("}".to_string());
    }

    let mut args = vec![
        format!("-Method {}", ps_string(&flow.method.to_ascii_uppercase())),
        format!("-Uri {}", ps_string(&flow.url)),
    ];
    if !headers.is_empty() {
        args.push("-Headers $headers".to_string());
    }
    if let Some(ct) = ct {
        args.push(format!("-ContentType {}", ps_string(ct)));
    }
    if let Some(body) = utf8_body(flow) {
        args.push(format!("-Body {}", ps_string(body)));
    }

    lines.push("Invoke-RestMethod `".to_string());
    for (idx, arg) in args.iter().enumerate() {
        let suffix = if idx + 1 < args.len() { " `" } else { "" };
        lines.push(format!("  {arg}{suffix}"));
    }
    lines.join("\n")
}

/// Generates a JavaScript `axios` request snippet.
pub fn axios_snippet(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        "import axios from \"axios\";".to_string(),
        String::new(),
        "const response = await axios({".to_string(),
        format!(
            "  method: {},",
            js_string(&flow.method.to_ascii_uppercase())
        ),
        format!("  url: {},", js_string(&flow.url)),
    ];
    if !headers.is_empty() {
        lines.push("  headers: {".to_string());
        for (name, value) in &headers {
            lines.push(format!("    {}: {},", js_string(name), js_string(value)));
        }
        lines.push("  },".to_string());
    }
    if let Some(body) = utf8_body(flow) {
        let is_json =
            content_type(&headers).is_some_and(|ct| ct.to_ascii_lowercase().contains("json"));
        if is_json
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(body)
            && let Ok(pretty) = serde_json::to_string_pretty(&val)
        {
            let mut pretty_lines = pretty.lines();
            if let Some(first) = pretty_lines.next() {
                let rest: Vec<&str> = pretty_lines.collect();
                if rest.is_empty() {
                    lines.push(format!("  data: {first},"));
                } else {
                    lines.push(format!("  data: {first}"));
                    for (idx, line) in rest.iter().enumerate() {
                        let comma = if idx + 1 == rest.len() { "," } else { "" };
                        lines.push(format!("  {line}{comma}"));
                    }
                }
            }
        } else {
            lines.push(format!("  data: {},", js_string(body)));
        }
    }
    lines.push("});".to_string());
    lines.join("\n")
}

/// Generates a Kotlin `OkHttp` request snippet.
pub fn kotlin_okhttp(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let method = flow.method.to_ascii_uppercase();
    let body = utf8_body(flow);
    let needs_body_obj = body.is_some() || matches!(method.as_str(), "POST" | "PUT" | "PATCH");
    let ct = content_type(&headers);

    let mut lines = Vec::new();
    if needs_body_obj && ct.is_some() {
        lines.push("import okhttp3.MediaType.Companion.toMediaType".to_string());
    }
    lines.push("import okhttp3.OkHttpClient".to_string());
    lines.push("import okhttp3.Request".to_string());
    if needs_body_obj {
        lines.push("import okhttp3.RequestBody.Companion.toRequestBody".to_string());
    }
    lines.push(String::new());
    lines.push("val client = OkHttpClient()".to_string());
    if needs_body_obj {
        let media_arg = if let Some(ct) = ct {
            lines.push(format!(
                "val mediaType = {}.toMediaType()",
                kotlin_string(ct)
            ));
            "mediaType"
        } else {
            "null"
        };
        lines.push(format!(
            "val body = {}.toRequestBody({media_arg})",
            kotlin_string(body.unwrap_or(""))
        ));
    }

    lines.push("val request = Request.Builder()".to_string());
    lines.push(format!("    .url({})", kotlin_string(&flow.url)));
    if needs_body_obj {
        lines.push(format!("    .method({}, body)", kotlin_string(&method)));
    } else if method == "GET" {
        lines.push("    .get()".to_string());
    } else if method == "DELETE" {
        lines.push("    .delete()".to_string());
    } else {
        lines.push(format!("    .method({}, null)", kotlin_string(&method)));
    }
    for (name, value) in &headers {
        lines.push(format!(
            "    .addHeader({}, {})",
            kotlin_string(name),
            kotlin_string(value)
        ));
    }
    lines.push("    .build()".to_string());
    lines.push("client.newCall(request).execute().use { response ->".to_string());
    lines.push("    println(response.body?.string())".to_string());
    lines.push("}".to_string());
    lines.join("\n")
}

/// Generates a Swift `URLSession` async/await request snippet.
pub fn swift_urlsession(flow: &Flow) -> String {
    let headers = copy_headers(&flow.request_headers);
    let mut lines = vec![
        "import Foundation".to_string(),
        String::new(),
        format!("let url = URL(string: {})!", swift_string(&flow.url)),
        "var request = URLRequest(url: url)".to_string(),
        format!(
            "request.httpMethod = {}",
            swift_string(&flow.method.to_ascii_uppercase())
        ),
    ];
    for (name, value) in &headers {
        lines.push(format!(
            "request.setValue({}, forHTTPHeaderField: {})",
            swift_string(value),
            swift_string(name)
        ));
    }
    if let Some(body) = utf8_body(flow) {
        lines.push(format!(
            "request.httpBody = {}.data(using: .utf8)",
            swift_string(body)
        ));
    }
    lines.push(String::new());
    lines.push("let (data, response) = try await URLSession.shared.data(for: request)".to_string());
    lines.push("print(String(data: data, encoding: .utf8) ?? \"\")".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_flow(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Flow {
        use crate::session::{Direction, FlowDraft, Protocol, Session};
        let mut session = Session::default();
        let id = session.open(FlowDraft {
            protocol: Protocol::Http11,
            method: method.to_string(),
            host: "api.example.com".to_string(),
            path: "/".to_string(),
            url: url.to_string(),
            request_headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        });
        if !body.is_empty() {
            session.append_body(id, Direction::Request, body);
        }
        session.get(id).unwrap().clone()
    }

    #[test]
    fn httpie_handles_get_query_headers_and_post_body_escaping() {
        let get = sample_flow(
            "GET",
            "https://api.example.com/items?page=2&q=a%20b",
            &[
                ("Host", "api.example.com"),
                ("Accept", "application/json"),
                ("X-Empty", ""),
            ],
            b"",
        );
        let cmd = httpie_command(&get);
        assert_eq!(
            cmd,
            "http GET 'https://api.example.com/items?page=2&q=a%20b' 'Accept:application/json' 'X-Empty;'"
        );

        let post = sample_flow(
            "post",
            "https://api.example.com/users",
            &[
                ("Content-Type", "application/json"),
                ("Content-Length", "22"),
                ("X-Note", "it's ok"),
            ],
            br#"{"name":"o'hara"}"#,
        );
        let cmd = httpie_command(&post);
        assert!(cmd.starts_with("http POST 'https://api.example.com/users'"));
        assert!(cmd.contains("'Content-Type:application/json'"));
        assert!(cmd.contains("'X-Note:it'\\''s ok'"));
        assert!(cmd.contains("--raw '{\"name\":\"o'\\''hara\"}'"));
        assert!(!cmd.contains("Content-Length"));
    }

    #[test]
    fn powershell_handles_get_post_content_type_and_escaping() {
        let get = sample_flow(
            "GET",
            "https://api.example.com/search?q=1&sort=asc",
            &[("Accept", "application/json")],
            b"",
        );
        let ps = powershell_command(&get);
        assert!(ps.contains("$headers = @{"));
        assert!(ps.contains("'Accept' = 'application/json'"));
        assert!(ps.contains("-Method 'GET'"));
        assert!(ps.contains("-Uri 'https://api.example.com/search?q=1&sort=asc'"));
        assert!(ps.contains("-Headers $headers"));
        assert!(!ps.contains("-Body"));

        let post = sample_flow(
            "POST",
            "https://api.example.com/submit",
            &[
                ("Content-Type", "text/plain"),
                ("X-User", "O'Brien"),
                ("Proxy-Authorization", "Basic secret"),
            ],
            b"line1 $var\n\"line2\" `tick",
        );
        let ps = powershell_command(&post);
        assert!(ps.contains("'X-User' = 'O''Brien'"));
        assert!(ps.contains("-ContentType 'text/plain'"));
        assert!(ps.contains("-Body \"line1 `$var`n`\"line2`\" ``tick\""));
        assert!(!ps.contains("Proxy-Authorization"));
    }

    #[test]
    fn axios_formats_json_object_and_text_bodies() {
        let get = sample_flow("GET", "https://api.example.com/v1/ping?verbose=1", &[], b"");
        let js = axios_snippet(&get);
        assert!(js.contains("import axios from \"axios\";"));
        assert!(js.contains("method: \"GET\","));
        assert!(js.contains("url: \"https://api.example.com/v1/ping?verbose=1\","));
        assert!(!js.contains("headers:"));
        assert!(!js.contains("data:"));

        let post_json = sample_flow(
            "POST",
            "https://api.example.com/v1/users",
            &[("Content-Type", "application/json; charset=utf-8")],
            br#"{"id":1,"name":"alice"}"#,
        );
        let js = axios_snippet(&post_json);
        assert!(js.contains("\"Content-Type\": \"application/json; charset=utf-8\","));
        assert!(js.contains("  data: {\n    \"id\": 1,\n    \"name\": \"alice\"\n  },"));

        let post_text = sample_flow(
            "POST",
            "https://api.example.com/v1/raw",
            &[("Content-Type", "text/plain")],
            b"hello\n\"world\"",
        );
        let js = axios_snippet(&post_text);
        assert!(js.contains(r#"  data: "hello\n\"world\"","#));
    }

    #[test]
    fn kotlin_okhttp_escapes_dollar_and_handles_methods() {
        let get = sample_flow(
            "GET",
            "https://api.example.com/items?limit=10",
            &[("Accept", "application/json")],
            b"",
        );
        let kt = kotlin_okhttp(&get);
        assert!(!kt.contains("toRequestBody"));
        assert!(kt.contains(".url(\"https://api.example.com/items?limit=10\")"));
        assert!(kt.contains(".get()"));
        assert!(kt.contains(".addHeader(\"Accept\", \"application/json\")"));

        let post = sample_flow(
            "POST",
            "https://api.example.com/pay",
            &[("Content-Type", "application/json"), ("X-Price", "$100")],
            br#"{"price":"$100","note":"a\nb"}"#,
        );
        let kt = kotlin_okhttp(&post);
        assert!(kt.contains("import okhttp3.MediaType.Companion.toMediaType"));
        assert!(kt.contains("import okhttp3.RequestBody.Companion.toRequestBody"));
        assert!(kt.contains("val mediaType = \"application/json\".toMediaType()"));
        assert!(kt.contains(
            r#"val body = "{\"price\":\"\$100\",\"note\":\"a\\nb\"}".toRequestBody(mediaType)"#
        ));
        assert!(kt.contains(".method(\"POST\", body)"));
        assert!(kt.contains(r#".addHeader("X-Price", "\$100")"#));

        let empty_post = sample_flow("POST", "https://api.example.com/trigger", &[], b"");
        let kt_empty = kotlin_okhttp(&empty_post);
        assert!(kt_empty.contains("val body = \"\".toRequestBody(null)"));
        assert!(kt_empty.contains(".method(\"POST\", body)"));
    }

    #[test]
    fn swift_urlsession_handles_get_post_headers_and_escaping() {
        let get = sample_flow(
            "GET",
            "https://api.example.com/v1/info?lang=zh",
            &[("Authorization", "Bearer abc")],
            b"",
        );
        let sw = swift_urlsession(&get);
        assert!(sw.contains("import Foundation"));
        assert!(sw.contains("let url = URL(string: \"https://api.example.com/v1/info?lang=zh\")!"));
        assert!(sw.contains("request.httpMethod = \"GET\""));
        assert!(
            sw.contains("request.setValue(\"Bearer abc\", forHTTPHeaderField: \"Authorization\")")
        );
        assert!(!sw.contains("request.httpBody"));

        let post = sample_flow(
            "POST",
            "https://api.example.com/v1/echo",
            &[("Content-Type", "text/plain")],
            b"hello \\(name)\n\"ok\"",
        );
        let sw = swift_urlsession(&post);
        assert!(sw.contains("request.httpMethod = \"POST\""));
        assert!(sw.contains(r#"request.httpBody = "hello \\(name)\n\"ok\"".data(using: .utf8)"#));
    }
}
