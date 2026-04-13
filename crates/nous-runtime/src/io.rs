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
    // ── HTTP Server ──────────────────────────────────
    fns.insert("http_serve_static".into(), builtin_http_serve_static);

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

// ── HTTP Server ──────────────────────────────────────

/// Start a simple HTTP server that serves static files and a JSON API.
/// Args: (port: Int, db_path: Text, html_dir: Text)
/// The server handles:
///   GET /                → serves index.html from html_dir
///   GET /api/namespaces  → JSON list of namespaces
///   GET /api/proposals   → JSON list of proposals
///   GET /api/constraints → JSON list of constraints
///   GET /api/functions   → JSON list of functions
///   GET /api/history     → JSON transition history
///   POST /api/chat       → store a chat message
///   GET /api/chat        → JSON list of chat messages
///   GET /*               → serves static file from html_dir
fn builtin_http_serve_static(args: &[Value]) -> Result<Value, RuntimeError> {
    use std::net::TcpListener;
    use std::io::{Read, Write, BufRead, BufReader};

    let port = match args.first() {
        Some(Value::Int(p)) => *p as u16,
        _ => 8080,
    };
    let db_path = require_text(args, 1, "http_serve_static")?.to_string();
    let html_dir = require_text(args, 2, "http_serve_static")?.to_string();

    // Ensure chat table exists
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chat (id INTEGER PRIMARY KEY AUTOINCREMENT, author TEXT NOT NULL, message TEXT NOT NULL, created_at INTEGER NOT NULL)"
        ).ok();
    }

    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| RuntimeError::TypeMismatch {
        expected: "bind address".into(),
        got: e.to_string(),
    })?;

    eprintln!("Agora serving on http://localhost:{port}");

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() { continue; }

        let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
        if parts.len() < 2 { continue; }
        let method = parts[0];
        let path = parts[1];

        // Read headers (skip body for GET, read body for POST)
        let mut content_length = 0usize;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).is_err() { break; }
            if header.trim().is_empty() { break; }
            if header.to_lowercase().starts_with("content-length:") {
                content_length = header.split(':').nth(1)
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
            }
        }

        let body = if content_length > 0 {
            let mut buf = vec![0u8; content_length];
            reader.read_exact(&mut buf).ok();
            String::from_utf8_lossy(&buf).to_string()
        } else {
            String::new()
        };

        let (status, content_type, response_body) = handle_http_request(
            method, path, &body, &db_path, &html_dir,
        );

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).ok();
    }

    Ok(Value::Void)
}

fn handle_http_request(
    method: &str, path: &str, body: &str,
    db_path: &str, html_dir: &str,
) -> (String, String, String) {
    // CORS preflight
    if method == "OPTIONS" {
        return ("204 No Content".into(), "text/plain".into(), String::new());
    }

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html_path = format!("{html_dir}/index.html");
            match std::fs::read_to_string(&html_path) {
                Ok(html) => ("200 OK".into(), "text/html; charset=utf-8".into(), html),
                Err(_) => ("404 Not Found".into(), "text/plain".into(), "index.html not found".into()),
            }
        }

        ("GET", "/api/namespaces") => {
            let result = query_db(db_path, "SELECT name, description FROM namespaces");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/proposals") => {
            let result = query_db(db_path, "SELECT id, namespace, status, submitted_at FROM proposals ORDER BY submitted_at DESC LIMIT 50");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/constraints") => {
            let result = query_db(db_path, "SELECT namespace, constraint_text, kind, added_by_proposal FROM constraints");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/functions") => {
            let result = query_db(db_path, "SELECT fn_name, current_hash, namespace FROM graph_heads");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/history") => {
            let result = query_db(db_path, "SELECT fn_name, old_hash, new_hash, reason, created_at FROM transitions ORDER BY created_at DESC LIMIT 50");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/chat") => {
            let result = query_db(db_path, "SELECT id, author, message, created_at FROM chat ORDER BY created_at DESC LIMIT 100");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("POST", "/api/chat") => {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
                let author = json.get("author").and_then(|v| v.as_str()).unwrap_or("anonymous");
                let message = json.get("message").and_then(|v| v.as_str()).unwrap_or("");
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    conn.execute(
                        "INSERT INTO chat (author, message, created_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![author, message, ts as i64],
                    ).ok();
                }
                ("200 OK".into(), "application/json".into(), r#"{"ok":true}"#.into())
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        // ── Generic table API ─────────────────────
        // Any table can be queried via /api/{table_name}
        // CRUD operations via GET/POST/PUT/DELETE
        ("GET", p) if p.starts_with("/api/") => {
            let table = &p[5..]; // strip "/api/"
            if table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                let sql = format!("SELECT * FROM [{table}] ORDER BY rowid DESC LIMIT 200");
                let result = query_db(db_path, &sql);
                ("200 OK".into(), "application/json".into(), result)
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid table name"}"#.into())
            }
        }

        ("POST", p) if p.starts_with("/api/") => {
            let table = &p[5..];
            if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid table name"}"#.into());
            }
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                if let Some(obj) = j.as_object() {
                    if let Ok(conn) = rusqlite::Connection::open(db_path) {
                        let cols: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                        let placeholders: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
                        let sql = format!("INSERT INTO [{table}] ({}) VALUES ({})", cols.join(","), placeholders.join(","));
                        let values: Vec<String> = obj.values().map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            _ => v.to_string(),
                        }).collect();
                        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
                        match conn.execute(&sql, params.as_slice()) {
                            Ok(_) => ("201 Created".into(), "application/json".into(), format!(r#"{{"ok":true,"id":{}}}"#, conn.last_insert_rowid())),
                            Err(e) => ("400 Bad Request".into(), "application/json".into(), format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "'"))),
                        }
                    } else {
                        ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                    }
                } else {
                    ("400 Bad Request".into(), "application/json".into(), r#"{"error":"expected object"}"#.into())
                }
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("PUT", p) if p.starts_with("/api/") => {
            let table = &p[5..];
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let id = j.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                if id == 0 {
                    return ("400 Bad Request".into(), "application/json".into(), r#"{"error":"id required"}"#.into());
                }
                if let Some(obj) = j.as_object() {
                    if let Ok(conn) = rusqlite::Connection::open(db_path) {
                        let sets: Vec<String> = obj.iter()
                            .filter(|(k, _)| k.as_str() != "id")
                            .enumerate()
                            .map(|(i, (k, _))| format!("{k} = ?{}", i + 1))
                            .collect();
                        if sets.is_empty() {
                            return ("400 Bad Request".into(), "application/json".into(), r#"{"error":"no fields"}"#.into());
                        }
                        let values: Vec<String> = obj.iter()
                            .filter(|(k, _)| k.as_str() != "id")
                            .map(|(_, v)| match v {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => v.to_string(),
                            }).collect();
                        let sql = format!("UPDATE [{table}] SET {} WHERE id = {id}", sets.join(", "));
                        let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
                        match conn.execute(&sql, params.as_slice()) {
                            Ok(n) => ("200 OK".into(), "application/json".into(), format!(r#"{{"ok":true,"updated":{n}}}"#)),
                            Err(e) => ("400 Bad Request".into(), "application/json".into(), format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "'"))),
                        }
                    } else {
                        ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                    }
                } else {
                    ("400 Bad Request".into(), "application/json".into(), r#"{"error":"expected object"}"#.into())
                }
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("DELETE", p) if p.starts_with("/api/") => {
            let table = &p[5..];
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let id = j.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                if id == 0 {
                    return ("400 Bad Request".into(), "application/json".into(), r#"{"error":"id required"}"#.into());
                }
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    match conn.execute(&format!("DELETE FROM [{table}] WHERE id = ?1"), rusqlite::params![id]) {
                        Ok(n) => ("200 OK".into(), "application/json".into(), format!(r#"{{"ok":true,"deleted":{n}}}"#)),
                        Err(e) => ("400 Bad Request".into(), "application/json".into(), format!(r#"{{"error":"{}"}}"#, e.to_string().replace('"', "'"))),
                    }
                } else {
                    ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                }
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("GET", p) => {
            // Serve static files
            let file_path = format!("{html_dir}{p}");
            let content_type = if p.ends_with(".js") { "application/javascript" }
                else if p.ends_with(".css") { "text/css" }
                else if p.ends_with(".html") { "text/html" }
                else { "text/plain" };
            match std::fs::read_to_string(&file_path) {
                Ok(content) => ("200 OK".into(), format!("{content_type}; charset=utf-8"), content),
                Err(_) => ("404 Not Found".into(), "text/plain".into(), "not found".into()),
            }
        }

        _ => ("405 Method Not Allowed".into(), "text/plain".into(), "method not allowed".into()),
    }
}

fn query_db(db_path: &str, sql: &str) -> String {
    match rusqlite::Connection::open(db_path) {
        Ok(conn) => {
            match conn.prepare(sql) {
                Ok(mut stmt) => {
                    let col_count = stmt.column_count();
                    let col_names: Vec<String> = (0..col_count)
                        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                        .collect();

                    let rows: Vec<serde_json::Value> = stmt.query_map([], |row| {
                        let mut obj = serde_json::Map::new();
                        for (i, name) in col_names.iter().enumerate() {
                            let val = match row.get_ref(i) {
                                Ok(rusqlite::types::ValueRef::Integer(n)) => serde_json::json!(n),
                                Ok(rusqlite::types::ValueRef::Real(f)) => serde_json::json!(f),
                                Ok(rusqlite::types::ValueRef::Text(s)) => {
                                    serde_json::Value::String(String::from_utf8_lossy(s).to_string())
                                }
                                Ok(rusqlite::types::ValueRef::Null) => serde_json::Value::Null,
                                Ok(rusqlite::types::ValueRef::Blob(_)) => serde_json::Value::Null,
                                Err(_) => serde_json::Value::Null,
                            };
                            obj.insert(name.clone(), val);
                        }
                        Ok(serde_json::Value::Object(obj))
                    }).unwrap().filter_map(|r| r.ok()).collect();

                    serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
                }
                Err(e) => format!(r#"[{{"error": "{}"}}]"#, e),
            }
        }
        Err(e) => format!(r#"[{{"error": "{}"}}]"#, e),
    }
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
