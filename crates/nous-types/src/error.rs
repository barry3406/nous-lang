use nous_ast::Span;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// All type errors that can be produced during type checking.
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum TypeError {
    /// A type name was used but is not defined or imported.
    #[error("unknown type `{name}` at {span:?}")]
    UnknownType { name: String, span: Span },

    /// An expression or binding has the wrong type.
    #[error("type mismatch: expected `{expected}`, got `{got}` at {span:?}")]
    TypeMismatch {
        expected: String,
        got: String,
        span: Span,
    },

    /// A state in a state machine can never be reached from the initial state.
    #[error("state `{state}` in machine `{machine}` is unreachable at {span:?}")]
    UnreachableState {
        state: String,
        machine: String,
        span: Span,
    },

    /// A state in a state machine has no outgoing transitions (and is not a
    /// terminal state), leaving it as a dead end.
    #[error("state `{state}` in machine `{machine}` has no transitions at {span:?}")]
    MissingTransition {
        state: String,
        machine: String,
        span: Span,
    },

    /// A match expression does not cover all variants of the scrutinee type.
    #[error("non-exhaustive match: missing variants {missing_variants:?} at {span:?}")]
    NonExhaustiveMatch {
        missing_variants: Vec<String>,
        span: Span,
    },

    /// A `Result`-typed value was discarded without being inspected.
    #[error("unconsumed Result value at {span:?}; handle both Ok and Err branches")]
    UnconsumedResult { span: Span },

    /// A function claims to perform an effect that is not listed in its
    /// contract's `effects` clause.
    #[error(
        "function `{fn_name}` uses undeclared effect `{effect}` at {span:?}"
    )]
    UndeclaredEffect {
        effect: String,
        fn_name: String,
        span: Span,
    },

    /// A pre-condition (`require`) or post-condition (`ensures`) was violated
    /// statically.
    #[error("contract violation ({kind}): {message} at {span:?}")]
    ContractViolation {
        kind: ContractViolationKind,
        message: String,
        span: Span,
    },
}

/// Distinguishes which part of a contract was violated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContractViolationKind {
    Precondition,
    Postcondition,
    Invariant,
}

impl std::fmt::Display for ContractViolationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContractViolationKind::Precondition => write!(f, "precondition"),
            ContractViolationKind::Postcondition => write!(f, "postcondition"),
            ContractViolationKind::Invariant => write!(f, "invariant"),
        }
    }
}
