//! JSX element, prop, and child parser used by [`crate::layout_tsx`].

use crate::layout_tsx::{TsxCompiler, is_ident_continue, is_ident_start, transpile_expr};

pub(crate) fn parse_jsx_element(compiler: &mut TsxCompiler<'_>) -> Result<String, String> {
    if compiler.peek() != Some(b'<') {
        return Err("expected `<` at JSX element".to_string());
    }
    compiler.advance(1);
    if compiler.peek() == Some(b'>') {
        compiler.advance(1);
        let children = parse_jsx_children(compiler, "")?;
        let mut out = String::from("__zniff_h(__zniff_fragment, null");
        for c in children {
            out.push_str(", ");
            out.push_str(&c);
        }
        out.push(')');
        return Ok(out);
    }

    let tag = read_jsx_name(compiler)?;
    let props = parse_jsx_props(compiler)?;
    compiler.skip_ws_and_comments()?;
    if compiler.starts_with("/>") {
        compiler.advance(2);
        return Ok(format!("__zniff_h({tag}, {props})"));
    }
    if compiler.peek() != Some(b'>') {
        return Err(format!("expected `>` or `/>` in `<{tag}>`"));
    }
    compiler.advance(1);
    let children = parse_jsx_children(compiler, &tag)?;
    let mut out = format!("__zniff_h({tag}, {props}");
    for c in children {
        out.push_str(", ");
        out.push_str(&c);
    }
    out.push(')');
    Ok(out)
}

fn parse_jsx_props(compiler: &mut TsxCompiler<'_>) -> Result<String, String> {
    let mut entries = Vec::new();
    loop {
        compiler.skip_ws_and_comments()?;
        if compiler.starts_with("/>") || compiler.peek() == Some(b'>') {
            break;
        }
        if compiler.peek() == Some(b'{') {
            let expr = compiler.read_braced_inner()?;
            let trimmed = expr.trim();
            let spread = trimmed
                .strip_prefix("...")
                .ok_or_else(|| format!("invalid JSX spread prop `{{{trimmed}}}`"))?;
            let compiled = transpile_expr(spread)?;
            entries.push(format!("...({compiled})"));
            continue;
        }
        let key = read_jsx_attr_name(compiler)?;
        compiler.skip_ws_and_comments()?;
        if compiler.peek() == Some(b'=') {
            compiler.advance(1);
            compiler.skip_ws_and_comments()?;
            let val = match compiler.peek() {
                Some(b'\'' | b'"') => compiler.read_quoted_json()?,
                Some(b'{') => {
                    let inner = compiler.read_braced_inner()?;
                    let compiled = transpile_expr(&inner)?;
                    format!("({compiled})")
                }
                _ => {
                    return Err(format!(
                        "expected quoted string or `{{expr}}` for prop `{key}`"
                    ));
                }
            };
            entries.push(format!("{key}: {val}"));
        } else {
            entries.push(format!("{key}: true"));
        }
    }
    if entries.is_empty() {
        Ok("null".to_string())
    } else {
        Ok(format!("{{ {} }}", entries.join(", ")))
    }
}

fn parse_jsx_children(
    compiler: &mut TsxCompiler<'_>,
    closing_tag: &str,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    while !compiler.is_eof() {
        if compiler.starts_with("</") {
            compiler.advance(2);
            let end_tag = if compiler.peek() == Some(b'>') {
                String::new()
            } else {
                read_jsx_name(compiler)?
            };
            compiler.skip_ws_and_comments()?;
            if compiler.peek() != Some(b'>') {
                return Err(format!("expected `>` in closing tag `</{end_tag}>`"));
            }
            compiler.advance(1);
            if end_tag != closing_tag {
                return Err(format!(
                    "mismatched JSX closing tag `</{end_tag}>`, expected `</{closing_tag}>`"
                ));
            }
            return Ok(out);
        }
        if compiler.peek() == Some(b'<') {
            out.push(parse_jsx_element(compiler)?);
            continue;
        }
        if compiler.peek() == Some(b'{') {
            let inner = compiler.read_braced_inner()?;
            let compiled = transpile_expr(&inner)?;
            if !compiled.trim().is_empty() {
                out.push(format!("({compiled})"));
            }
            continue;
        }
        let raw = compiler.take_jsx_text();
        if let Some(clean) = clean_jsx_text(raw) {
            out.push(serde_json::to_string(&clean).unwrap_or_else(|_| "\"\"".to_string()));
        }
    }
    Err(format!("unclosed JSX tag `<{closing_tag}>`"))
}

fn read_jsx_name(compiler: &mut TsxCompiler<'_>) -> Result<String, String> {
    compiler.skip_ws_and_comments()?;
    let mut name = String::new();
    if !compiler.peek().is_some_and(is_ident_start) {
        return Err("expected JSX tag name".to_string());
    }
    while let Some(b) = compiler.peek() {
        if is_ident_continue(b) || b == b'.' {
            name.push(b as char);
            compiler.advance(1);
        } else {
            break;
        }
    }
    Ok(name)
}

fn read_jsx_attr_name(compiler: &mut TsxCompiler<'_>) -> Result<String, String> {
    let mut name = String::new();
    if !compiler.peek().is_some_and(is_ident_start) {
        return Err("expected JSX attribute name".to_string());
    }
    while let Some(b) = compiler.peek() {
        if is_ident_continue(b) {
            name.push(b as char);
            compiler.advance(1);
        } else {
            break;
        }
    }
    Ok(name)
}

fn clean_jsx_text(raw: &str) -> Option<String> {
    if !raw.contains('\n') {
        return (!raw.is_empty()).then(|| raw.to_string());
    }
    let lines: Vec<&str> = raw.split('\n').collect();
    let mut parts = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = if idx == 0 {
            line.trim_end()
        } else if idx + 1 == lines.len() {
            line.trim_start()
        } else {
            line.trim()
        };
        if !trimmed.is_empty() {
            parts.push(trimmed);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}
