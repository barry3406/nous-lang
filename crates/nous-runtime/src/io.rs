//! I/O built-in functions: file system, network, database.
//!
//! These are the effect handlers that give Nous programs access to
//! the outside world. Every I/O operation is an explicit effect —
//! Nous functions must declare `effect Fs.read`, `effect Http.request`, etc.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use crate::value::Value;
use crate::error::RuntimeError;
use crate::builtins::NativeFn;

/// Register all I/O builtins into a provided map.
pub fn register_io_builtins(fns: &mut BTreeMap<String, NativeFn>) {
    // ── File system ──────────────────────────────────
    fns.insert("fs_read".into(), builtin_fs_read);
    fns.insert("fs_write".into(), builtin_fs_write);
    fns.insert("fs_exists".into(), builtin_fs_exists);
    fns.insert("fs_delete".into(), builtin_fs_delete);
    fns.insert("fs_list_dir".into(), builtin_fs_list_dir);

    // ── Network (HTTP client) ────────────────────────
    fns.insert("http_get".into(), builtin_http_get);
    fns.insert("http_post".into(), builtin_http_post);

    // ── Database (SQLite) ────────────────────────────
    fns.insert("db_open".into(), builtin_db_open);
    fns.insert("db_execute".into(), builtin_db_execute);
    fns.insert("db_query".into(), builtin_db_query);

    // ── JSON ─────────────────────────────────────────
    fns.insert("json_parse".into(), builtin_json_parse);
    fns.insert("json_stringify".into(), builtin_json_stringify);

    // ── Environment ──────────────────────────────────
    fns.insert("env_get".into(), builtin_env_get);
    fns.insert("env_args".into(), builtin_env_args);
}

// ── File system ──────────────────────────────────────

fn builtin_fs_read(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "fs_read")?;
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Value::Ok(Box::new(Value::Text(content)))),
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_fs_write(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "fs_write")?;
    let content = require_text(args, 1, "fs_write")?;
    match std::fs::write(path, content) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Void))),
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_fs_exists(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "fs_exists")?;
    Ok(Value::Bool(std::path::Path::new(path).exists()))
}

fn builtin_fs_delete(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "fs_delete")?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(Value::Ok(Box::new(Value::Void))),
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_fs_list_dir(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "fs_list_dir")?;
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .filter_map(|e| e.ok())
                .map(|e| Value::Text(e.path().display().to_string()))
                .collect();
            Ok(Value::Ok(Box::new(Value::List(items))))
        }
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

// ── Network (HTTP — synchronous for now) ─────────────

fn builtin_http_get(args: &[Value]) -> Result<Value, RuntimeError> {
    let url = require_text(args, 0, "http_get")?;

    // Minimal HTTP GET using std::net
    let response = match simple_http_get(url) {
        Ok(body) => Value::Ok(Box::new(Value::Text(body))),
        Err(e) => Value::Err(Box::new(Value::Text(e))),
    };
    Ok(response)
}

fn builtin_http_post(args: &[Value]) -> Result<Value, RuntimeError> {
    let url = require_text(args, 0, "http_post")?;
    let body = require_text(args, 1, "http_post")?;

    let response = match simple_http_post(url, body) {
        Ok(resp) => Value::Ok(Box::new(Value::Text(resp))),
        Err(e) => Value::Err(Box::new(Value::Text(e))),
    };
    Ok(response)
}

/// Minimal HTTP GET using std::net (no external deps).
fn simple_http_get(url: &str) -> Result<String, String> {
    use std::net::TcpStream;

    let (host, port, path) = parse_url(url)?;
    let mut stream = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| format!("connection failed: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(|e| format!("write failed: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| format!("read failed: {e}"))?;

    // Extract body (after \r\n\r\n)
    if let Some(idx) = response.find("\r\n\r\n") {
        Ok(response[idx + 4..].to_string())
    } else {
        Ok(response)
    }
}

fn simple_http_post(url: &str, body: &str) -> Result<String, String> {
    use std::net::TcpStream;

    let (host, port, path) = parse_url(url)?;
    let mut stream = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| format!("connection failed: {e}"))?;

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(|e| format!("write failed: {e}"))?;

    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| format!("read failed: {e}"))?;

    if let Some(idx) = response.find("\r\n\r\n") {
        Ok(response[idx + 4..].to_string())
    } else {
        Ok(response)
    }
}

fn parse_url(url: &str) -> Result<(&str, u16, &str), String> {
    let url = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = url.split_once('/').unwrap_or((url, "/"));
    let path = if path.is_empty() { "/" } else { &url[url.find('/').unwrap_or(url.len())..] };
    let (host, port) = if let Some((h, p)) = host_port.split_once(':') {
        (h, p.parse::<u16>().map_err(|_| "invalid port".to_string())?)
    } else {
        (host_port, 80)
    };
    Ok((host, port, path))
}

// ── Database (SQLite) ────────────────────────────────

fn builtin_db_open(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "db_open")?;
    match rusqlite::Connection::open(path) {
        Ok(_) => Ok(Value::Ok(Box::new(Value::Text(path.to_string())))),
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_db_execute(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "db_execute")?;
    let sql = require_text(args, 1, "db_execute")?;
    match rusqlite::Connection::open(path) {
        Ok(conn) => match conn.execute_batch(sql) {
            Ok(()) => Ok(Value::Ok(Box::new(Value::Void))),
            Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_db_query(args: &[Value]) -> Result<Value, RuntimeError> {
    let path = require_text(args, 0, "db_query")?;
    let sql = require_text(args, 1, "db_query")?;
    match rusqlite::Connection::open(path) {
        Ok(conn) => {
            let mut stmt = match conn.prepare(sql) {
                Ok(s) => s,
                Err(e) => return Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
            };
            let column_count = stmt.column_count();
            let column_names: Vec<String> = (0..column_count)
                .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                .collect();

            let rows: Result<Vec<Value>, _> = stmt.query_map([], |row| {
                let mut fields = BTreeMap::new();
                for (i, name) in column_names.iter().enumerate() {
                    let val: String = row.get::<_, String>(i).unwrap_or_default();
                    fields.insert(name.clone(), Value::Text(val));
                }
                Ok(Value::Record { name: "Row".to_string(), fields })
            }).and_then(|mapped| mapped.collect());

            match rows {
                Ok(items) => Ok(Value::Ok(Box::new(Value::List(items)))),
                Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
            }
        }
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

// ── JSON ─────────────────────────────────────────────

fn builtin_json_parse(args: &[Value]) -> Result<Value, RuntimeError> {
    let text = require_text(args, 0, "json_parse")?;
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(json) => Ok(Value::Ok(Box::new(json_to_value(&json)))),
        Err(e) => Ok(Value::Err(Box::new(Value::Text(e.to_string())))),
    }
}

fn builtin_json_stringify(args: &[Value]) -> Result<Value, RuntimeError> {
    let val = args.first().unwrap_or(&Value::Void);
    let json = value_to_json(val);
    Ok(Value::Text(serde_json::to_string(&json).unwrap_or_default()))
}

fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Void,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => Value::Int(n.as_i64().unwrap_or(0)),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Array(arr) => {
            Value::List(arr.iter().map(json_to_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let fields: BTreeMap<String, Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect();
            Value::Record { name: "Object".into(), fields }
        }
    }
}

fn value_to_json(val: &Value) -> serde_json::Value {
    match val {
        Value::Void => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(n) => serde_json::json!(*n),
        Value::Nat(n) => serde_json::json!(*n),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::List(items) => {
            serde_json::Value::Array(items.iter().map(value_to_json).collect())
        }
        Value::Record { fields, .. } => {
            let obj: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Ok(inner) => value_to_json(inner),
        Value::Err(inner) => serde_json::json!({"error": value_to_json(inner)}),
        _ => serde_json::Value::String(format!("{val}")),
    }
}

// ── Environment ──────────────────────────────────────

fn builtin_env_get(args: &[Value]) -> Result<Value, RuntimeError> {
    let key = require_text(args, 0, "env_get")?;
    match std::env::var(key) {
        Ok(val) => Ok(Value::Ok(Box::new(Value::Text(val)))),
        Err(_) => Ok(Value::Err(Box::new(Value::Text(format!("{key} not set"))))),
    }
}

fn builtin_env_args(_args: &[Value]) -> Result<Value, RuntimeError> {
    let args: Vec<Value> = std::env::args().map(Value::Text).collect();
    Ok(Value::List(args))
}

// ── Helpers ──────────────────────────────────────────

fn require_text<'a>(args: &'a [Value], idx: usize, fn_name: &str) -> Result<&'a str, RuntimeError> {
    match args.get(idx) {
        Some(Value::Text(s)) => Ok(s.as_str()),
        _ => Err(RuntimeError::TypeMismatch {
            expected: format!("Text at arg {idx} of {fn_name}"),
            got: args.get(idx).map_or("missing", |v| v.type_name()).into(),
        }),
    }
}
