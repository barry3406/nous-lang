use std::collections::BTreeMap;
use std::fmt;

/// The set of values that can exist at runtime in a Nous program.
///
/// `Dec` stores the decimal as a string together with a precision byte so
/// that exact decimal arithmetic can be delegated to a fixed-point library
/// without lossy float conversions.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Natural number (non-negative integer).
    Nat(u64),
    /// Signed integer.
    Int(i64),
    /// Fixed-precision decimal: `(digits_as_string, precision)`.
    /// E.g. `Dec("31415".to_string(), 4)` represents `3.1415`.
    Dec(String, u8),
    /// Boolean.
    Bool(bool),
    /// UTF-8 text.
    Text(String),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// A named record (struct) with ordered fields.
    Record {
        name: String,
        fields: BTreeMap<String, Value>,
    },
    /// An enum variant carrying positional payload fields.
    Enum {
        variant: String,
        fields: Vec<Value>,
    },
    /// Homogeneous list.
    List(Vec<Value>),
    /// Heterogeneous tuple.
    Tuple(Vec<Value>),
    /// The unit / void value.
    Void,
    /// Successful result wrapper.
    Ok(Box<Value>),
    /// Error result wrapper.
    Err(Box<Value>),
    /// A first-class function reference.
    Fn { name: String, arity: usize },
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nat(n) => write!(f, "{n}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Dec(digits, precision) => {
                let p = *precision as usize;
                if p == 0 || digits.len() <= p {
                    // If all digits are fractional, pad with leading zeros.
                    let padded = format!("{:0>width$}", digits, width = p + 1);
                    let (int_part, frac_part) = padded.split_at(padded.len() - p);
                    if p == 0 {
                        write!(f, "{int_part}")
                    } else {
                        write!(f, "{int_part}.{frac_part}")
                    }
                } else {
                    let (int_part, frac_part) = digits.split_at(digits.len() - p);
                    if p == 0 {
                        write!(f, "{int_part}")
                    } else {
                        write!(f, "{int_part}.{frac_part}")
                    }
                }
            }
            Value::Bool(b) => write!(f, "{b}"),
            Value::Text(s) => write!(f, "{s}"),
            Value::Bytes(bs) => {
                write!(f, "0x")?;
                for b in bs {
                    write!(f, "{b:02x}")?;
                }
                Ok(())
            }
            Value::Record { name, fields } => {
                write!(f, "{name} {{ ")?;
                let mut first = true;
                for (k, v) in fields {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                    first = false;
                }
                write!(f, " }}")
            }
            Value::Enum { variant, fields } => {
                if fields.is_empty() {
                    write!(f, "{variant}")
                } else {
                    write!(f, "{variant}(")?;
                    for (i, fld) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{fld}")?;
                    }
                    write!(f, ")")
                }
            }
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Value::Void => write!(f, "void"),
            Value::Ok(inner) => write!(f, "Ok({inner})"),
            Value::Err(inner) => write!(f, "Err({inner})"),
            Value::Fn { name, arity } => write!(f, "<fn {name}/{arity}>"),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl Value {
    /// Return the type name as a short string, used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nat(_) => "Nat",
            Value::Int(_) => "Int",
            Value::Dec(_, _) => "Dec",
            Value::Bool(_) => "Bool",
            Value::Text(_) => "Text",
            Value::Bytes(_) => "Bytes",
            Value::Record { .. } => "Record",
            Value::Enum { .. } => "Enum",
            Value::List(_) => "List",
            Value::Tuple(_) => "Tuple",
            Value::Void => "Void",
            Value::Ok(_) => "Ok",
            Value::Err(_) => "Err",
            Value::Fn { .. } => "Fn",
        }
    }

    /// Convenience: is this value truthy for use in `JumpIfFalse`?
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Void => false,
            _ => true,
        }
    }
}
