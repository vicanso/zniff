//! Zero-dependency `.proto` (proto2/proto3) schema parser and Protobuf/gRPC wire decoder.
//!
//! Supports mounting a single `.proto` file or a directory of `.proto` files via the
//! `proto <host[/path]> <schema_path> [req=<pkg.Msg>] [res=<pkg.Msg>]` rule. For standard
//! gRPC `:path` values (`/package.Service/Method`), automatically resolves request and
//! response message definitions without needing external `protoc`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::{Map, Value, json};

use crate::rules::ProtoSpec;
use crate::session::base64_encode;

const MAX_PROTO_FILES: usize = 256;
const MAX_PROTO_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DECODE_DEPTH: usize = 32;
const REGISTRY_CACHE_CAP: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoDecodeOutput {
    pub message_name: String,
    pub json_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
enum FieldType {
    Double,
    Float,
    Int32,
    Int64,
    Uint32,
    Uint64,
    Sint32,
    Sint64,
    Fixed32,
    Fixed64,
    Sfixed32,
    Sfixed64,
    Bool,
    String,
    Bytes,
    Named(String),
    Map {
        key: Box<FieldType>,
        val: Box<FieldType>,
    },
}

impl FieldType {
    fn from_scalar_or_named(name: &str) -> Self {
        match name {
            "double" => Self::Double,
            "float" => Self::Float,
            "int32" => Self::Int32,
            "int64" => Self::Int64,
            "uint32" => Self::Uint32,
            "uint64" => Self::Uint64,
            "sint32" => Self::Sint32,
            "sint64" => Self::Sint64,
            "fixed32" => Self::Fixed32,
            "fixed64" => Self::Fixed64,
            "sfixed32" => Self::Sfixed32,
            "sfixed64" => Self::Sfixed64,
            "bool" => Self::Bool,
            "string" => Self::String,
            "bytes" => Self::Bytes,
            other => Self::Named(other.to_string()),
        }
    }

    fn is_packable_scalar(&self, reg: &ProtoRegistry, scope: &str) -> bool {
        match self {
            Self::Double
            | Self::Float
            | Self::Int32
            | Self::Int64
            | Self::Uint32
            | Self::Uint64
            | Self::Sint32
            | Self::Sint64
            | Self::Fixed32
            | Self::Fixed64
            | Self::Sfixed32
            | Self::Sfixed64
            | Self::Bool => true,
            Self::Named(raw) => reg.resolve_enum(scope, raw).is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct FieldDef {
    name: String,
    number: u32,
    ty: FieldType,
    repeated: bool,
}

#[derive(Debug, Clone)]
struct MessageDef {
    full_name: String,
    fields: HashMap<u32, FieldDef>,
}

#[derive(Debug, Clone)]
struct EnumDef {
    values: HashMap<i32, String>,
}

#[derive(Debug, Clone)]
struct RpcDef {
    req_type: String,
    res_type: String,
    scope: String,
}

#[derive(Debug, Default)]
pub struct ProtoRegistry {
    messages: HashMap<String, MessageDef>,
    enums: HashMap<String, EnumDef>,
    /// Keyed by `/package.Service/Method` (or `/Service/Method` when no package).
    rpcs: HashMap<String, RpcDef>,
}

impl ProtoRegistry {
    pub fn from_proto_sources<'a>(sources: impl IntoIterator<Item = &'a str>) -> Self {
        let mut reg = Self::default();
        for src in sources {
            let tokens = tokenize(src);
            let mut parser = Parser {
                tokens: &tokens,
                pos: 0,
            };
            parser.parse_file(&mut reg);
        }
        reg
    }

    fn resolve_qualified<'a, T>(
        map: &'a HashMap<String, T>,
        scope: &str,
        raw: &str,
    ) -> Option<(&'a str, &'a T)> {
        let trimmed = raw.trim();
        if let Some(abs) = trimmed.strip_prefix('.') {
            return map.get_key_value(abs).map(|(k, v)| (k.as_str(), v));
        }
        if !scope.is_empty() {
            let mut cur = scope;
            loop {
                let candidate = format!("{cur}.{trimmed}");
                if let Some((k, v)) = map.get_key_value(&candidate) {
                    return Some((k.as_str(), v));
                }
                match cur.rfind('.') {
                    Some(idx) => cur = &cur[..idx],
                    None => break,
                }
            }
        }
        if let Some((k, v)) = map.get_key_value(trimmed) {
            return Some((k.as_str(), v));
        }
        // Fallback suffix match when user omits package prefix in `req=MyMsg`.
        let suffix = format!(".{trimmed}");
        map.iter()
            .find(|(k, _)| k.ends_with(&suffix))
            .map(|(k, v)| (k.as_str(), v))
    }

    fn resolve_message(&self, scope: &str, raw: &str) -> Option<(&str, &MessageDef)> {
        Self::resolve_qualified(&self.messages, scope, raw)
    }

    fn resolve_enum(&self, scope: &str, raw: &str) -> Option<(&str, &EnumDef)> {
        Self::resolve_qualified(&self.enums, scope, raw)
    }

    fn resolve_rpc_message(
        &self,
        flow_path: &str,
        is_response: bool,
    ) -> Option<(&str, &MessageDef)> {
        let clean = flow_path.split('?').next().unwrap_or(flow_path).trim();
        let rpc = self.rpcs.get(clean).or_else(|| {
            // Allow matching `/Service/Method` suffix against `/pkg.Service/Method`.
            let tail = clean.trim_start_matches('/');
            self.rpcs
                .iter()
                .find(|(k, _)| k.trim_start_matches('/').ends_with(tail))
                .map(|(_, v)| v)
        })?;
        let raw_ty = if is_response {
            &rpc.res_type
        } else {
            &rpc.req_type
        };
        self.resolve_message(&rpc.scope, raw_ty)
    }

    pub fn decode_body(
        &self,
        flow_path: &str,
        body: &[u8],
        explicit_type: Option<&str>,
        is_response: bool,
    ) -> Option<ProtoDecodeOutput> {
        if body.is_empty() {
            return None;
        }
        let (msg_name, msg_def) = match explicit_type {
            Some(ty) if !ty.trim().is_empty() => self.resolve_message("", ty)?,
            _ => self.resolve_rpc_message(flow_path, is_response)?,
        };
        let value = if let Some(frames) = split_grpc_frames(body) {
            if frames.len() == 1 {
                decode_message(self, msg_def, frames[0], 0)?
            } else {
                let mut arr = Vec::with_capacity(frames.len());
                for frame in frames {
                    arr.push(decode_message(self, msg_def, frame, 0)?);
                }
                Value::Array(arr)
            }
        } else {
            decode_message(self, msg_def, body, 0)?
        };
        let json_bytes = serde_json::to_vec_pretty(&value).ok()?;
        Some(ProtoDecodeOutput {
            message_name: msg_name.to_string(),
            json_bytes,
        })
    }
}

/// Decodes `body` using the first matching `ProtoSpec` that resolves a message definition.
pub fn decode_with_specs(
    specs: &[ProtoSpec],
    flow_path: &str,
    body: &[u8],
    is_response: bool,
) -> Option<ProtoDecodeOutput> {
    if specs.is_empty() || body.is_empty() {
        return None;
    }
    for spec in specs {
        let Some(reg) = load_registry_cached(&spec.schema_path) else {
            continue;
        };
        let explicit = if is_response {
            spec.res_type.as_deref()
        } else {
            spec.req_type.as_deref()
        };
        if let Some(decoded) = reg.decode_body(flow_path, body, explicit, is_response) {
            return Some(decoded);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// File / Directory Loading with mtime Cache
// ---------------------------------------------------------------------------

struct CachedRegistry {
    path: PathBuf,
    fingerprint: (usize, SystemTime),
    registry: Arc<ProtoRegistry>,
}

fn registry_cache() -> &'static Mutex<Vec<CachedRegistry>> {
    static CACHE: OnceLock<Mutex<Vec<CachedRegistry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn collect_proto_files(root: &Path, out: &mut Vec<(PathBuf, SystemTime)>, depth: usize) {
    if depth > 8 || out.len() >= MAX_PROTO_FILES {
        return;
    }
    if root.is_file() {
        if let Ok(meta) = fs::metadata(root)
            && meta.len() <= MAX_PROTO_FILE_BYTES
        {
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((root.to_path_buf(), mtime));
        }
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        if out.len() >= MAX_PROTO_FILES {
            break;
        }
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            dirs.push(path);
        } else if ft.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("proto"))
            && let Ok(meta) = entry.metadata()
            && meta.len() <= MAX_PROTO_FILE_BYTES
        {
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push((path, mtime));
        }
    }
    dirs.sort();
    for dir in dirs {
        collect_proto_files(&dir, out, depth + 1);
    }
}

fn load_registry_cached(path: &Path) -> Option<Arc<ProtoRegistry>> {
    let mut files = Vec::new();
    collect_proto_files(path, &mut files, 0);
    if files.is_empty() {
        return None;
    }
    let max_mtime = files
        .iter()
        .map(|(_, t)| *t)
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let fp = (files.len(), max_mtime);
    {
        let cache = registry_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = cache.iter().find(|c| c.path == path && c.fingerprint == fp) {
            return Some(Arc::clone(&hit.registry));
        }
    }
    let mut texts = Vec::with_capacity(files.len());
    for (file, _) in &files {
        if let Ok(content) = fs::read_to_string(file) {
            texts.push(content);
        }
    }
    let registry = Arc::new(ProtoRegistry::from_proto_sources(
        texts.iter().map(String::as_str),
    ));
    {
        let mut cache = registry_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|c| c.path != path);
        if cache.len() >= REGISTRY_CACHE_CAP {
            cache.remove(0);
        }
        cache.push(CachedRegistry {
            path: path.to_path_buf(),
            fingerprint: fp,
            registry: Arc::clone(&registry),
        });
    }
    Some(registry)
}

// ---------------------------------------------------------------------------
// Minimal .proto Lexer & Recursive Descent Parser (proto2 / proto3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Sym(char),
}

fn tokenize(src: &str) -> Vec<Tok> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Line comment `// ...`
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment `/* ... */`
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // String literal `"..."` or `'...'`
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            let start = i;
            let mut val = String::new();
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    val.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    val.push(bytes[i] as char);
                    i += 1;
                }
            }
            let _ = start;
            if i < bytes.len() {
                i += 1;
            }
            out.push(Tok::Str(val));
            continue;
        }
        // Identifier / qualified name / number (including negative numbers & dots)
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-') {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'.'))
            {
                i += 1;
            }
            out.push(Tok::Ident(src[start..i].to_string()));
            continue;
        }
        out.push(Tok::Sym(b as char));
        i += 1;
    }
    out
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<&'a Tok> {
        let tok = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(tok)
    }

    fn peek_ident(&self) -> Option<&'a str> {
        match self.peek() {
            Some(Tok::Ident(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn expect_ident(&mut self) -> Option<String> {
        match self.next() {
            Some(Tok::Ident(s)) => Some(s.clone()),
            _ => None,
        }
    }

    fn eat_sym(&mut self, ch: char) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(c)) if *c == ch) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_balanced(&mut self, open: char, close: char) {
        if !self.eat_sym(open) {
            return;
        }
        let mut depth = 1usize;
        while let Some(tok) = self.next() {
            if let Tok::Sym(c) = tok {
                if *c == open {
                    depth += 1;
                } else if *c == close {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
        }
    }

    fn skip_statement(&mut self) {
        while let Some(tok) = self.peek() {
            match tok {
                Tok::Sym(';') => {
                    self.pos += 1;
                    break;
                }
                Tok::Sym('{') => {
                    self.skip_balanced('{', '}');
                    break;
                }
                Tok::Sym('}') => break,
                _ => self.pos += 1,
            }
        }
    }

    fn parse_file(&mut self, reg: &mut ProtoRegistry) {
        let mut pkg = String::new();
        while let Some(tok) = self.peek() {
            match tok {
                Tok::Ident(kw) => match kw.as_str() {
                    "package" => {
                        self.pos += 1;
                        if let Some(name) = self.expect_ident() {
                            pkg = name;
                        }
                        self.skip_statement();
                    }
                    "message" => {
                        self.pos += 1;
                        self.parse_message(&pkg, reg);
                    }
                    "enum" => {
                        self.pos += 1;
                        self.parse_enum(&pkg, reg);
                    }
                    "service" => {
                        self.pos += 1;
                        self.parse_service(&pkg, reg);
                    }
                    _ => {
                        self.pos += 1;
                        self.skip_statement();
                    }
                },
                _ => {
                    self.pos += 1;
                }
            }
        }
    }

    fn qualify(scope: &str, name: &str) -> String {
        if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}.{name}")
        }
    }

    fn parse_enum(&mut self, scope: &str, reg: &mut ProtoRegistry) {
        let Some(name) = self.expect_ident() else {
            return;
        };
        let full_name = Self::qualify(scope, &name);
        if !self.eat_sym('{') {
            self.skip_statement();
            return;
        }
        let mut values = HashMap::new();
        while let Some(tok) = self.peek() {
            if matches!(tok, Tok::Sym('}')) {
                self.pos += 1;
                break;
            }
            if let Some(kw) = self.peek_ident() {
                if matches!(kw, "option" | "reserved") {
                    self.pos += 1;
                    self.skip_statement();
                    continue;
                }
                let val_name = kw.to_string();
                self.pos += 1;
                if self.eat_sym('=')
                    && let Some(num_str) = self.expect_ident()
                    && let Ok(num) = num_str.parse::<i32>()
                {
                    values.entry(num).or_insert(val_name);
                }
                self.skip_statement();
            } else {
                self.pos += 1;
            }
        }
        reg.enums.insert(full_name, EnumDef { values });
    }

    fn parse_message(&mut self, scope: &str, reg: &mut ProtoRegistry) {
        let Some(name) = self.expect_ident() else {
            return;
        };
        let full_name = Self::qualify(scope, &name);
        if !self.eat_sym('{') {
            self.skip_statement();
            return;
        }
        let mut fields = HashMap::new();
        self.parse_message_body(&full_name, &mut fields, reg);
        reg.messages
            .insert(full_name.clone(), MessageDef { full_name, fields });
    }

    fn parse_message_body(
        &mut self,
        msg_scope: &str,
        fields: &mut HashMap<u32, FieldDef>,
        reg: &mut ProtoRegistry,
    ) {
        while let Some(tok) = self.peek() {
            if matches!(tok, Tok::Sym('}')) {
                self.pos += 1;
                break;
            }
            if matches!(tok, Tok::Sym(';')) {
                self.pos += 1;
                continue;
            }
            let Some(kw) = self.peek_ident() else {
                self.pos += 1;
                continue;
            };
            match kw {
                "message" => {
                    self.pos += 1;
                    self.parse_message(msg_scope, reg);
                }
                "enum" => {
                    self.pos += 1;
                    self.parse_enum(msg_scope, reg);
                }
                "oneof" => {
                    self.pos += 1;
                    let _ = self.expect_ident();
                    if self.eat_sym('{') {
                        self.parse_message_body(msg_scope, fields, reg);
                    } else {
                        self.skip_statement();
                    }
                }
                "option" | "reserved" | "extensions" | "extend" => {
                    self.pos += 1;
                    self.skip_statement();
                }
                "map" => {
                    self.pos += 1;
                    if let Some(field) = self.parse_map_field() {
                        fields.insert(field.number, field);
                    }
                }
                _ => {
                    if let Some(field) = self.parse_regular_field() {
                        fields.insert(field.number, field);
                    }
                }
            }
        }
    }

    fn parse_map_field(&mut self) -> Option<FieldDef> {
        if !self.eat_sym('<') {
            self.skip_statement();
            return None;
        }
        let key_str = self.expect_ident()?;
        let _ = self.eat_sym(',');
        let val_str = self.expect_ident()?;
        let _ = self.eat_sym('>');
        let name = self.expect_ident()?;
        if !self.eat_sym('=') {
            self.skip_statement();
            return None;
        }
        let num_str = self.expect_ident()?;
        let number = num_str.parse::<u32>().ok()?;
        if matches!(self.peek(), Some(Tok::Sym('['))) {
            self.skip_balanced('[', ']');
        }
        let _ = self.eat_sym(';');
        Some(FieldDef {
            name,
            number,
            ty: FieldType::Map {
                key: Box::new(FieldType::from_scalar_or_named(&key_str)),
                val: Box::new(FieldType::from_scalar_or_named(&val_str)),
            },
            repeated: false,
        })
    }

    fn parse_regular_field(&mut self) -> Option<FieldDef> {
        let mut repeated = false;
        let first = self.expect_ident()?;
        let ty_name = if matches!(first.as_str(), "repeated" | "optional" | "required") {
            repeated = first == "repeated";
            self.expect_ident()?
        } else {
            first
        };
        let name = self.expect_ident()?;
        if !self.eat_sym('=') {
            self.skip_statement();
            return None;
        }
        let num_str = self.expect_ident()?;
        let Ok(number) = num_str.parse::<u32>() else {
            self.skip_statement();
            return None;
        };
        if matches!(self.peek(), Some(Tok::Sym('['))) {
            self.skip_balanced('[', ']');
        }
        let _ = self.eat_sym(';');
        Some(FieldDef {
            name,
            number,
            ty: FieldType::from_scalar_or_named(&ty_name),
            repeated,
        })
    }

    fn parse_service(&mut self, pkg: &str, reg: &mut ProtoRegistry) {
        let Some(svc_name) = self.expect_ident() else {
            return;
        };
        let full_svc = Self::qualify(pkg, &svc_name);
        if !self.eat_sym('{') {
            self.skip_statement();
            return;
        }
        while let Some(tok) = self.peek() {
            if matches!(tok, Tok::Sym('}')) {
                self.pos += 1;
                break;
            }
            if self.peek_ident() == Some("rpc") {
                self.pos += 1;
                if let Some((method, req_type, res_type)) = self.parse_rpc() {
                    reg.rpcs.insert(
                        format!("/{full_svc}/{method}"),
                        RpcDef {
                            req_type,
                            res_type,
                            scope: pkg.to_string(),
                        },
                    );
                }
            } else {
                self.pos += 1;
                self.skip_statement();
            }
        }
    }

    fn parse_rpc(&mut self) -> Option<(String, String, String)> {
        let method = self.expect_ident()?;
        if !self.eat_sym('(') {
            self.skip_statement();
            return None;
        }
        let mut req = self.expect_ident()?;
        if req == "stream" {
            req = self.expect_ident()?;
        }
        let _ = self.eat_sym(')');
        if self.peek_ident() == Some("returns") {
            self.pos += 1;
        }
        if !self.eat_sym('(') {
            self.skip_statement();
            return None;
        }
        let mut res = self.expect_ident()?;
        if res == "stream" {
            res = self.expect_ident()?;
        }
        let _ = self.eat_sym(')');
        self.skip_statement();
        Some((method, req, res))
    }
}

// ---------------------------------------------------------------------------
// Wire Format Decoding
// ---------------------------------------------------------------------------

fn split_grpc_frames(body: &[u8]) -> Option<Vec<&[u8]>> {
    if body.len() < 5 {
        return None;
    }
    let mut frames = Vec::new();
    let mut pos = 0usize;
    while pos < body.len() {
        if pos + 5 > body.len() {
            return None;
        }
        let compressed = body[pos];
        if compressed > 1 {
            return None;
        }
        let len = u32::from_be_bytes([body[pos + 1], body[pos + 2], body[pos + 3], body[pos + 4]])
            as usize;
        pos += 5;
        if pos + len > body.len() {
            return None;
        }
        if compressed != 0 {
            return None;
        }
        frames.push(&body[pos..pos + len]);
        pos += len;
    }
    if frames.is_empty() {
        None
    } else {
        Some(frames)
    }
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while *pos < buf.len() && shift < 64 {
        let byte = buf[*pos];
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

fn zigzag_i32(v: u32) -> i32 {
    ((v >> 1) as i32) ^ -((v & 1) as i32)
}

fn zigzag_i64(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

fn decode_scalar_varint(reg: &ProtoRegistry, scope: &str, ty: &FieldType, raw: u64) -> Value {
    match ty {
        FieldType::Int32 => json!(raw as i32),
        FieldType::Int64 => json!(raw as i64),
        FieldType::Uint32 => json!(raw as u32),
        FieldType::Uint64 => json!(raw),
        FieldType::Sint32 => json!(zigzag_i32(raw as u32)),
        FieldType::Sint64 => json!(zigzag_i64(raw)),
        FieldType::Bool => json!(raw != 0),
        FieldType::Named(name) => {
            if let Some((_, edef)) = reg.resolve_enum(scope, name)
                && let Some(label) = edef.values.get(&(raw as i32))
            {
                return json!(label);
            }
            json!(raw as i32)
        }
        _ => json!(raw),
    }
}

fn decode_scalar_fixed64(ty: &FieldType, bytes: [u8; 8]) -> Value {
    match ty {
        FieldType::Double => json!(f64::from_le_bytes(bytes)),
        FieldType::Sfixed64 => json!(i64::from_le_bytes(bytes)),
        _ => json!(u64::from_le_bytes(bytes)),
    }
}

fn decode_scalar_fixed32(ty: &FieldType, bytes: [u8; 4]) -> Value {
    match ty {
        FieldType::Float => json!(f32::from_le_bytes(bytes)),
        FieldType::Sfixed32 => json!(i32::from_le_bytes(bytes)),
        _ => json!(u32::from_le_bytes(bytes)),
    }
}

fn decode_packed(
    reg: &ProtoRegistry,
    scope: &str,
    ty: &FieldType,
    slice: &[u8],
) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    let mut pos = 0;
    match ty {
        FieldType::Double | FieldType::Fixed64 | FieldType::Sfixed64 => {
            while pos + 8 <= slice.len() {
                let bytes: [u8; 8] = slice[pos..pos + 8].try_into().ok()?;
                out.push(decode_scalar_fixed64(ty, bytes));
                pos += 8;
            }
            (pos == slice.len()).then_some(out)
        }
        FieldType::Float | FieldType::Fixed32 | FieldType::Sfixed32 => {
            while pos + 4 <= slice.len() {
                let bytes: [u8; 4] = slice[pos..pos + 4].try_into().ok()?;
                out.push(decode_scalar_fixed32(ty, bytes));
                pos += 4;
            }
            (pos == slice.len()).then_some(out)
        }
        _ => {
            while pos < slice.len() {
                let v = read_varint(slice, &mut pos)?;
                out.push(decode_scalar_varint(reg, scope, ty, v));
            }
            Some(out)
        }
    }
}

fn decode_map_entry(
    reg: &ProtoRegistry,
    scope: &str,
    key_ty: &FieldType,
    val_ty: &FieldType,
    slice: &[u8],
    depth: usize,
) -> Option<(String, Value)> {
    let mut pos = 0;
    let mut key_val = Value::String(String::new());
    let mut val_val = Value::Null;
    while pos < slice.len() {
        let tag = read_varint(slice, &mut pos)?;
        let field_num = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        let target_ty = match field_num {
            1 => Some(key_ty),
            2 => Some(val_ty),
            _ => None,
        };
        let decoded = decode_wire_value(reg, scope, target_ty, wire, slice, &mut pos, depth + 1)?;
        match field_num {
            1 => key_val = decoded,
            2 => val_val = decoded,
            _ => {}
        }
    }
    let key_str = match key_val {
        Value::String(s) => s,
        other => other.to_string(),
    };
    Some((key_str, val_val))
}

fn decode_wire_value(
    reg: &ProtoRegistry,
    scope: &str,
    ty: Option<&FieldType>,
    wire: u8,
    buf: &[u8],
    pos: &mut usize,
    depth: usize,
) -> Option<Value> {
    match wire {
        0 => {
            let v = read_varint(buf, pos)?;
            Some(match ty {
                Some(t) => decode_scalar_varint(reg, scope, t, v),
                None => json!(v),
            })
        }
        1 => {
            if *pos + 8 > buf.len() {
                return None;
            }
            let bytes: [u8; 8] = buf[*pos..*pos + 8].try_into().ok()?;
            *pos += 8;
            Some(match ty {
                Some(t) => decode_scalar_fixed64(t, bytes),
                None => json!(u64::from_le_bytes(bytes)),
            })
        }
        2 => {
            let len = read_varint(buf, pos)? as usize;
            if *pos + len > buf.len() {
                return None;
            }
            let slice = &buf[*pos..*pos + len];
            *pos += len;
            match ty {
                Some(FieldType::String) => {
                    Some(Value::String(String::from_utf8_lossy(slice).into_owned()))
                }
                Some(FieldType::Bytes) => Some(Value::String(base64_encode(slice))),
                Some(FieldType::Named(name)) => {
                    if let Some((_, sub_msg)) = reg.resolve_message(scope, name) {
                        decode_message(reg, sub_msg, slice, depth + 1)
                    } else {
                        Some(Value::String(base64_encode(slice)))
                    }
                }
                _ => match std::str::from_utf8(slice) {
                    Ok(s)
                        if !s
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
                    {
                        Some(Value::String(s.to_string()))
                    }
                    _ => Some(Value::String(base64_encode(slice))),
                },
            }
        }
        5 => {
            if *pos + 4 > buf.len() {
                return None;
            }
            let bytes: [u8; 4] = buf[*pos..*pos + 4].try_into().ok()?;
            *pos += 4;
            Some(match ty {
                Some(t) => decode_scalar_fixed32(t, bytes),
                None => json!(u32::from_le_bytes(bytes)),
            })
        }
        _ => None,
    }
}

fn push_field_value(obj: &mut Map<String, Value>, key: String, val: Value, force_array: bool) {
    if let Some(existing) = obj.get_mut(&key) {
        if let Value::Array(arr) = existing {
            arr.push(val);
        } else {
            let prev = existing.take();
            *existing = Value::Array(vec![prev, val]);
        }
    } else if force_array {
        obj.insert(key, Value::Array(vec![val]));
    } else {
        obj.insert(key, val);
    }
}

fn decode_message(
    reg: &ProtoRegistry,
    msg: &MessageDef,
    buf: &[u8],
    depth: usize,
) -> Option<Value> {
    if depth > MAX_DECODE_DEPTH {
        return None;
    }
    let mut obj = Map::new();
    let mut maps: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut pos = 0usize;

    while pos < buf.len() {
        let tag = read_varint(buf, &mut pos)?;
        let field_num = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        if field_num == 0 {
            return None;
        }
        if let Some(field) = msg.fields.get(&field_num) {
            if wire == 2 {
                if let FieldType::Map { key, val } = &field.ty {
                    let len = read_varint(buf, &mut pos)? as usize;
                    if pos + len > buf.len() {
                        return None;
                    }
                    let slice = &buf[pos..pos + len];
                    pos += len;
                    let (mk, mv) = decode_map_entry(reg, &msg.full_name, key, val, slice, depth)?;
                    maps.entry(field.name.clone()).or_default().insert(mk, mv);
                    continue;
                }
                if field.repeated && field.ty.is_packable_scalar(reg, &msg.full_name) {
                    let len = read_varint(buf, &mut pos)? as usize;
                    if pos + len > buf.len() {
                        return None;
                    }
                    let slice = &buf[pos..pos + len];
                    pos += len;
                    let vals = decode_packed(reg, &msg.full_name, &field.ty, slice)?;
                    for v in vals {
                        push_field_value(&mut obj, field.name.clone(), v, true);
                    }
                    continue;
                }
            }
            let val = decode_wire_value(
                reg,
                &msg.full_name,
                Some(&field.ty),
                wire,
                buf,
                &mut pos,
                depth,
            )?;
            push_field_value(&mut obj, field.name.clone(), val, field.repeated);
        } else {
            let val = decode_wire_value(reg, &msg.full_name, None, wire, buf, &mut pos, depth)?;
            push_field_value(&mut obj, format!("_unknown_{field_num}"), val, false);
        }
    }

    for (name, map_obj) in maps {
        obj.insert(name, Value::Object(map_obj));
    }
    Some(Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn encode_ld(field: u32, payload: &[u8], out: &mut Vec<u8>) {
        encode_varint(((field as u64) << 3) | 2, out);
        encode_varint(payload.len() as u64, out);
        out.extend_from_slice(payload);
    }

    fn grpc_frame(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + payload.len());
        out.push(0);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn decodes_grpc_service_rpc_nested_enum_packed_and_map() {
        let proto = r#"
            syntax = "proto3";
            package vicanso.user.v1;

            enum Status {
                STATUS_UNSPECIFIED = 0;
                STATUS_ACTIVE = 1;
            }

            message Address {
                string city = 1;
            }

            message GetUserReply {
                sint32 delta = 1;
                string name = 2;
                Status status = 3;
                Address address = 4;
                repeated int32 scores = 5 [packed = true];
                map<string, string> tags = 6;
            }

            service UserService {
                rpc GetUser (Address) returns (GetUserReply);
            }
        "#;

        let reg = ProtoRegistry::from_proto_sources([proto]);

        let mut addr = Vec::new();
        encode_ld(1, b"Shenzhen", &mut addr);

        let mut packed = Vec::new();
        encode_varint(10, &mut packed);
        encode_varint(20, &mut packed);

        let mut map_entry = Vec::new();
        encode_ld(1, b"env", &mut map_entry);
        encode_ld(2, b"prod", &mut map_entry);

        let mut reply = Vec::new();
        // field 1: sint32 -5 => zigzag = 9
        encode_varint(1 << 3, &mut reply);
        encode_varint(9, &mut reply);
        // field 2: string "Alice"
        encode_ld(2, b"Alice", &mut reply);
        // field 3: enum STATUS_ACTIVE (1)
        encode_varint(3 << 3, &mut reply);
        encode_varint(1, &mut reply);
        // field 4: nested Address
        encode_ld(4, &addr, &mut reply);
        // field 5: packed repeated int32 [10, 20]
        encode_ld(5, &packed, &mut reply);
        // field 6: map<string, string> {"env": "prod"}
        encode_ld(6, &map_entry, &mut reply);

        let framed = grpc_frame(&reply);
        let decoded = reg
            .decode_body("/vicanso.user.v1.UserService/GetUser", &framed, None, true)
            .expect("should resolve gRPC RPC reply and decode");

        assert_eq!(decoded.message_name, "vicanso.user.v1.GetUserReply");
        let val: Value = serde_json::from_slice(&decoded.json_bytes).unwrap();
        assert_eq!(val["delta"], -5);
        assert_eq!(val["name"], "Alice");
        assert_eq!(val["status"], "STATUS_ACTIVE");
        assert_eq!(val["address"]["city"], "Shenzhen");
        assert_eq!(val["scores"], json!([10, 20]));
        assert_eq!(val["tags"]["env"], "prod");
    }
}
