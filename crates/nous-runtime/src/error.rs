use thiserror::Error;

// ---------------------------------------------------------------------------
// Compile-time errors
// ---------------------------------------------------------------------------

/// Errors produced while compiling Nous AST to bytecode.
#[derive(Debug, Clone, Error)]
pub enum CompileError {
    /// A name was used but not defined in any reachable scope.
    #[error("undefined name: `{name}`")]
    UndefinedName { name: String },

    /// A function was called with the wrong number of arguments.
    #[error("arity mismatch calling `{name}`: expected {expected}, got {got}")]
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },

    /// An expression form that the compiler does not yet handle.
    #[error("unsupported expression in compiler: {description}")]
    Unsupported { description: String },

    /// An internal compiler invariant was violated (should not happen).
    #[error("internal compiler error: {message}")]
    Internal { message: String },

    /// A type annotation referenced a type that was not declared.
    #[error("unknown type: `{name}`")]
    UnknownType { name: String },

    /// A `require` clause referenced an identifier not in scope.
    #[error("contract references out-of-scope name `{name}` in function `{fn_name}`")]
    ContractScopeError { fn_name: String, name: String },
}

// ---------------------------------------------------------------------------
// Runtime errors
// ---------------------------------------------------------------------------

/// Errors produced during bytecode execution in the VM.
#[derive(Debug, Clone, Error)]
pub enum RuntimeError {
    /// The value stack was empty when an instruction tried to pop from it.
    #[error("stack underflow in chunk `{chunk_name}` at ip {ip}")]
    StackUnderflow { chunk_name: String, ip: usize },

    /// A `LoadLocal` or `StoreLocal` used an index beyond the current frame.
    #[error("local variable index {index} out of range (frame has {frame_size} locals)")]
    LocalOutOfRange { index: usize, frame_size: usize },

    /// A `Jump` or `JumpIfFalse` targeted an instruction that does not exist.
    #[error("jump to invalid instruction {target} (chunk `{chunk_name}` has {len} ops)")]
    InvalidJump {
        target: usize,
        chunk_name: String,
        len: usize,
    },

    /// Division or modulo by zero.
    #[error("division by zero")]
    DivisionByZero,

    /// A field was accessed on a non-record value.
    #[error("field access on non-record value (field `{field}`)")]
    NotARecord { field: String },

    /// A record field did not exist.
    #[error("no field `{field}` on record `{record}`")]
    MissingField { field: String, record: String },

    /// `Unwrap` (the `?` operator) was called on an `Err` value.
    #[error("unwrap of Err value: {message}")]
    UnwrappedErr { message: String },

    /// A `CheckRequire` assertion failed at runtime.
    #[error("require violated: {message}")]
    RequireViolated { message: String },

    /// A `CheckEnsure` assertion failed at runtime.
    #[error("ensure violated: {message}")]
    EnsureViolated { message: String },

    /// A `Call` instruction referenced a chunk index that does not exist.
    #[error("call to undefined chunk index {index}")]
    UndefinedChunk { index: usize },

    /// A type mismatch between the expected and actual runtime value.
    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch { expected: String, got: String },

    /// An arithmetic overflow occurred.
    #[error("arithmetic overflow")]
    Overflow,

    /// An internal VM invariant was violated (should not happen).
    #[error("internal VM error: {message}")]
    Internal { message: String },

    /// A `match` expression was non-exhaustive: no arm matched the scrutinee.
    #[error("non-exhaustive match: no arm matched value `{value}`")]
    NonExhaustiveMatch { value: String },
}
