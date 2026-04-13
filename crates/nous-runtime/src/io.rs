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

        // ── Auth endpoints ─────────────────────────
        ("POST", "/api/auth/login") => {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let email = j.get("email").and_then(|v| v.as_str()).unwrap_or("");
                let password = j.get("password").and_then(|v| v.as_str()).unwrap_or("");
                let tenant_id = j.get("tenant_id").and_then(|v| v.as_i64()).unwrap_or(1);
                if email.is_empty() || password.is_empty() {
                    return ("400 Bad Request".into(), "application/json".into(), r#"{"error":"email and password required"}"#.into());
                }
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    use sha2::{Sha256, Digest as _};
                    let pw_hash = hex::encode(Sha256::digest(password.as_bytes()));
                    let mut stmt = conn.prepare("SELECT id, name, role, status FROM users WHERE tenant_id = ?1 AND email = ?2 AND password_hash = ?3").unwrap();
                    let user: Option<(i64, String, String, String)> = stmt.query_row(
                        rusqlite::params![tenant_id, email, &pw_hash],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                    ).ok();
                    match user {
                        Some((uid, name, role, status)) if status == "active" => {
                            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                            let token = hex::encode(Sha256::digest(format!("{email}{ts}{pw_hash}").as_bytes()));
                            let expires = ts + 86400;
                            conn.execute(
                                "INSERT INTO sessions (token, user_id, tenant_id, role, expires_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                rusqlite::params![&token, uid, tenant_id, &role, expires, ts],
                            ).ok();
                            conn.execute("UPDATE users SET last_login = ?1 WHERE id = ?2", rusqlite::params![ts, uid]).ok();
                            conn.execute(
                                "INSERT INTO audit_log (tenant_id, user_id, action, resource, created_at) VALUES (?1, ?2, 'login', 'auth', ?3)",
                                rusqlite::params![tenant_id, uid, ts],
                            ).ok();
                            let result = serde_json::json!({"ok":true,"token":token,"user":{"id":uid,"name":name,"email":email,"role":role,"tenant_id":tenant_id}});
                            ("200 OK".into(), "application/json".into(), result.to_string())
                        }
                        Some((_, _, _, _)) => ("403 Forbidden".into(), "application/json".into(), r#"{"error":"account disabled"}"#.into()),
                        None => ("401 Unauthorized".into(), "application/json".into(), r#"{"error":"invalid credentials"}"#.into()),
                    }
                } else {
                    ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                }
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("POST", "/api/auth/logout") => {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let token = j.get("token").and_then(|v| v.as_str()).unwrap_or("");
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    conn.execute("DELETE FROM sessions WHERE token = ?1", rusqlite::params![token]).ok();
                }
                ("200 OK".into(), "application/json".into(), r#"{"ok":true}"#.into())
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("GET", "/api/auth/session") => {
            // Token from query string: /api/auth/session?token=xxx
            // Or we check a simple approach - parse token from path
            ("200 OK".into(), "application/json".into(), r#"{"error":"use POST with token"}"#.into())
        }

        ("POST", "/api/auth/session") => {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let token = j.get("token").and_then(|v| v.as_str()).unwrap_or("");
                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                    let mut stmt = conn.prepare(
                        "SELECT s.user_id, s.tenant_id, s.role, u.name, u.email, t.name as tenant_name FROM sessions s JOIN users u ON s.user_id = u.id JOIN tenants t ON s.tenant_id = t.id WHERE s.token = ?1 AND s.expires_at > ?2"
                    ).unwrap();
                    let session = stmt.query_row(
                        rusqlite::params![token, ts],
                        |row| {
                            Ok(serde_json::json!({
                                "user_id": row.get::<_, i64>(0)?,
                                "tenant_id": row.get::<_, i64>(1)?,
                                "role": row.get::<_, String>(2)?,
                                "name": row.get::<_, String>(3)?,
                                "email": row.get::<_, String>(4)?,
                                "tenant_name": row.get::<_, String>(5)?
                            }))
                        },
                    );
                    match session {
                        Ok(s) => ("200 OK".into(), "application/json".into(), serde_json::json!({"ok":true,"session":s}).to_string()),
                        Err(_) => ("401 Unauthorized".into(), "application/json".into(), r#"{"error":"invalid or expired session"}"#.into()),
                    }
                } else {
                    ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                }
            } else {
                ("400 Bad Request".into(), "application/json".into(), r#"{"error":"invalid json"}"#.into())
            }
        }

        ("GET", "/api/tenants") => {
            let result = query_db(db_path, "SELECT id, name, slug, plan, max_users FROM tenants");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/permissions") => {
            let result = query_db(db_path, "SELECT role, resource, action FROM permissions ORDER BY role, resource");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/audit_log") => {
            let result = query_db(db_path, "SELECT tenant_id, user_id, action, resource, resource_id, created_at FROM audit_log ORDER BY created_at DESC LIMIT 50");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/users") => {
            let result = query_db(db_path, "SELECT id, tenant_id, email, name, role, status, last_login, created_at FROM users ORDER BY id");
            ("200 OK".into(), "application/json".into(), result)
        }

        ("GET", "/api/sessions") => {
            let result = query_db(db_path, "SELECT token, user_id, tenant_id, role, expires_at FROM sessions ORDER BY created_at DESC");
            ("200 OK".into(), "application/json".into(), result)
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

        // ── Agora proposal verification pipeline ──
        // POST /api/proposals runs nous_verify on the source code
        // and auto-merges if verification passes.
        ("POST", "/api/proposals") => {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(body) {
                let namespace = j.get("namespace").and_then(|v| v.as_str()).unwrap_or("default");
                let source = j.get("source").and_then(|v| v.as_str()).unwrap_or("");
                if source.is_empty() {
                    return ("400 Bad Request".into(), "application/json".into(),
                        r#"{"error":"source required"}"#.into());
                }

                // Generate proposal ID
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default().as_secs() as i64;
                use sha2::{Sha256, Digest as _};
                let id = hex::encode(Sha256::digest(
                    format!("{namespace}{source}{ts}").as_bytes()
                ));

                if let Ok(conn) = rusqlite::Connection::open(db_path) {
                    // Store proposal as pending
                    conn.execute(
                        "INSERT INTO proposals (id, namespace, source, status, submitted_at) VALUES (?1, ?2, ?3, 'pending', ?4)",
                        rusqlite::params![&id, namespace, source, ts],
                    ).ok();

                    // ═══ RUN NOUS VERIFICATION PIPELINE ═══
                    let verify_result = run_nous_verify(source);

                    if verify_result.passed {
                        // AUTO-MERGE: verification passed
                        conn.execute(
                            "UPDATE proposals SET status = 'merged' WHERE id = ?1",
                            rusqlite::params![&id],
                        ).ok();

                        // Store content-addressed blob
                        let blob_hash = hex::encode(Sha256::digest(source.as_bytes()));
                        conn.execute(
                            "INSERT OR IGNORE INTO blobs (hash, content, created_at) VALUES (?1, ?2, ?3)",
                            rusqlite::params![&blob_hash, source, ts],
                        ).ok();

                        // Extract and store constraints
                        for constraint in &verify_result.constraints {
                            conn.execute(
                                "INSERT INTO constraints (namespace, constraint_text, kind, added_by_proposal) VALUES (?1, ?2, ?3, ?4)",
                                rusqlite::params![namespace, &constraint.text, &constraint.kind, &id],
                            ).ok();
                        }

                        // Post to chat
                        conn.execute(
                            "INSERT INTO chat (author, message, created_at) VALUES ('Agora', ?1, ?2)",
                            rusqlite::params![
                                format!("Proposal {} merged. {} constraint(s) verified by Z3. Proof, not persuasion.",
                                    &id[..16], verify_result.verified_count),
                                ts
                            ],
                        ).ok();

                        let result = serde_json::json!({
                            "ok": true,
                            "id": id,
                            "status": "merged",
                            "verified": verify_result.verified_count,
                            "unverified": verify_result.unverified_count,
                            "blob_hash": blob_hash,
                            "message": "verification passed — auto-merged"
                        });
                        ("201 Created".into(), "application/json".into(), result.to_string())
                    } else {
                        // REJECT: verification failed
                        conn.execute(
                            "UPDATE proposals SET status = 'rejected' WHERE id = ?1",
                            rusqlite::params![&id],
                        ).ok();

                        // Post rejection to chat
                        conn.execute(
                            "INSERT INTO chat (author, message, created_at) VALUES ('Agora', ?1, ?2)",
                            rusqlite::params![
                                format!("Proposal {} rejected at {} phase: {}",
                                    &id[..16], verify_result.failed_phase, verify_result.error_message),
                                ts
                            ],
                        ).ok();

                        let result = serde_json::json!({
                            "ok": false,
                            "id": id,
                            "status": "rejected",
                            "phase": verify_result.failed_phase,
                            "error": verify_result.error_message,
                            "message": "verification failed — rejected"
                        });
                        ("200 OK".into(), "application/json".into(), result.to_string())
                    }
                } else {
                    ("500 Internal Server Error".into(), "application/json".into(), r#"{"error":"db"}"#.into())
                }
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

// ── Nous Verification Pipeline ────────────────────────

struct VerifyPipelineResult {
    passed: bool,
    verified_count: usize,
    unverified_count: usize,
    constraints: Vec<ExtractedConstraint>,
    failed_phase: String,
    error_message: String,
}

struct ExtractedConstraint {
    text: String,
    kind: String,
}

fn run_nous_verify(source: &str) -> VerifyPipelineResult {
    // Phase 1: Parse
    let program = match nous_parser::parse(source) {
        Ok(p) => p,
        Err(e) => return VerifyPipelineResult {
            passed: false, verified_count: 0, unverified_count: 0,
            constraints: vec![], failed_phase: "parse".into(),
            error_message: e.to_string(),
        },
    };

    // Phase 2: Type check
    let mut checker = nous_types::TypeChecker::new();
    if let Err(errors) = checker.check(&program) {
        return VerifyPipelineResult {
            passed: false, verified_count: 0, unverified_count: 0,
            constraints: vec![], failed_phase: "typecheck".into(),
            error_message: errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
        };
    }

    // Phase 3: Z3 verification
    let verifier = nous_verify::Verifier::new();
    match verifier.verify(&program) {
        Ok(result) => {
            // Extract constraints from the AST
            let mut constraints = Vec::new();
            for decl in &program.declarations {
                match &decl.node {
                    nous_ast::decl::Decl::Fn(f) => {
                        for req in &f.contract.requires {
                            constraints.push(ExtractedConstraint {
                                text: format!("{:?}", req.condition.node).chars().take(100).collect(),
                                kind: "require".into(),
                            });
                        }
                        for ens in &f.contract.ensures {
                            let is_synth = matches!(&f.body.node, nous_ast::expr::Expr::Void)
                                || matches!(&f.body.node, nous_ast::expr::Expr::Block(s) if s.is_empty());
                            constraints.push(ExtractedConstraint {
                                text: format!("{:?}", ens.node).chars().take(100).collect(),
                                kind: if is_synth { "ensure (synthesized)".into() } else { "ensure".into() },
                            });
                        }
                    }
                    _ => {}
                }
            }

            VerifyPipelineResult {
                passed: true,
                verified_count: result.verified_count,
                unverified_count: result.unverified_count,
                constraints,
                failed_phase: String::new(),
                error_message: String::new(),
            }
        }
        Err(errors) => VerifyPipelineResult {
            passed: false, verified_count: 0, unverified_count: 0,
            constraints: vec![], failed_phase: "verify".into(),
            error_message: errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "),
        },
    }
}

fn require_text<'a>(args: &'a [Value], idx: usize, fn_name: &str) -> Result<&'a str, RuntimeError> {
    match args.get(idx) {
        Some(Value::Text(s)) => Ok(s.as_str()),
        _ => Err(RuntimeError::TypeMismatch {
            expected: format!("Text at arg {idx} of {fn_name}"),
            got: args.get(idx).map_or("missing", |v| v.type_name()).into(),
        }),
    }
}
