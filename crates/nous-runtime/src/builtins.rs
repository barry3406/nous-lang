//! Built-in functions for the Nous runtime.
//!
//! These provide I/O and system capabilities that can't be expressed
//! in pure Nous. They are the "effect handlers" — Rust implementations
//! bound to Nous function signatures.

use std::collections::BTreeMap;
use std::io::Write;

use crate::value::Value;
use crate::error::RuntimeError;

/// A native function takes a list of arguments and returns a Value.
pub type NativeFn = fn(&[Value]) -> Result<Value, RuntimeError>;

/// Registry of built-in functions.
pub struct Builtins {
    fns: BTreeMap<String, NativeFn>,
}

impl Builtins {
    /// Create a new registry with all standard builtins.
    pub fn new() -> Self {
        let mut b = Self { fns: BTreeMap::new() };

        // ── I/O ──────────────────────────────────────
        b.register("print", builtin_print);
        b.register("println", builtin_println);
        b.register("to_text", builtin_to_text);

        // ── Text operations ──────────────────────────
        b.register("text_len", builtin_text_len);
        b.register("text_concat", builtin_text_concat);

        // ── List operations ──────────────────────────
        b.register("list_len", builtin_list_len);
        b.register("list_get", builtin_list_get);
        b.register("list_push", builtin_list_push);

        // ── Type conversion ──────────────────────────
        b.register("int_to_text", builtin_int_to_text);
        b.register("text_to_int", builtin_text_to_int);

        // ── Hash (for content-addressing) ────────────
        b.register("sha256", builtin_sha256);

        // ── Time ─────────────────────────────────────
        b.register("now_unix", builtin_now_unix);

        // Register I/O builtins
        crate::io::register_io_builtins(&mut b.fns);

        b
    }

    fn register(&mut self, name: &str, f: NativeFn) {
        self.fns.insert(name.to_string(), f);
    }

    /// Look up a built-in function by name.
    pub fn lookup(&self, name: &str) -> Option<&NativeFn> {
        self.fns.get(name)
    }

    /// Check if a name is a builtin.
    pub fn contains(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }

    /// Get all builtin names.
    pub fn names(&self) -> Vec<&String> {
        self.fns.keys().collect()
    }
}

impl Default for Builtins {
    fn default() -> Self {
        Self::new()
    }
}

// ── Builtin implementations ──────────────────────────

fn builtin_print(args: &[Value]) -> Result<Value, RuntimeError> {
    for arg in args {
        print!("{arg}");
    }
    std::io::stdout().flush().ok();
    Ok(Value::Void)
}

fn builtin_println(args: &[Value]) -> Result<Value, RuntimeError> {
    for arg in args {
        print!("{arg}");
    }
    println!();
    Ok(Value::Void)
}

fn builtin_to_text(args: &[Value]) -> Result<Value, RuntimeError> {
    if args.is_empty() {
        return Ok(Value::Text(String::new()));
    }
    Ok(Value::Text(format!("{}", args[0])))
}

fn builtin_text_len(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Text(s)) => Ok(Value::Int(s.len() as i64)),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_text_concat(args: &[Value]) -> Result<Value, RuntimeError> {
    let mut result = String::new();
    for arg in args {
        match arg {
            Value::Text(s) => result.push_str(s),
            other => result.push_str(&format!("{other}")),
        }
    }
    Ok(Value::Text(result))
}

fn builtin_list_len(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::List(items)) => Ok(Value::Int(items.len() as i64)),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_list_get(args: &[Value]) -> Result<Value, RuntimeError> {
    match (args.first(), args.get(1)) {
        (Some(Value::List(items)), Some(Value::Int(idx))) => {
            let i = *idx as usize;
            items.get(i).cloned().ok_or(RuntimeError::TypeMismatch {
                expected: format!("index < {}", items.len()),
                got: format!("index {i}"),
            })
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List, Int".into(),
            got: "wrong types".into(),
        }),
    }
}

fn builtin_list_push(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::List(items)) => {
            let mut new_items = items.clone();
            if let Some(val) = args.get(1) {
                new_items.push(val.clone());
            }
            Ok(Value::List(new_items))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "List".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_int_to_text(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Int(n)) => Ok(Value::Text(n.to_string())),
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Int".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_text_to_int(args: &[Value]) -> Result<Value, RuntimeError> {
    match args.first() {
        Some(Value::Text(s)) => {
            let n: i64 = s.parse().map_err(|_| RuntimeError::TypeMismatch {
                expected: "numeric text".into(),
                got: format!("\"{s}\""),
            })?;
            Ok(Value::Int(n))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_sha256(args: &[Value]) -> Result<Value, RuntimeError> {
    use sha2::{Sha256, Digest};
    match args.first() {
        Some(Value::Text(s)) => {
            let mut hasher = Sha256::new();
            hasher.update(s.as_bytes());
            let result = hasher.finalize();
            Ok(Value::Text(hex::encode(result)))
        }
        _ => Err(RuntimeError::TypeMismatch {
            expected: "Text".into(),
            got: args.first().map_or("nothing", |v| v.type_name()).into(),
        }),
    }
}

fn builtin_now_unix(args: &[Value]) -> Result<Value, RuntimeError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(Value::Int(secs as i64))
}
