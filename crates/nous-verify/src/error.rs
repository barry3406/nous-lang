use nous_ast::Span;
use thiserror::Error;

/// Errors produced by the Nous verifier.
#[derive(Debug, Clone, Error)]
pub enum VerifyError {
    /// A `require` or `ensure` constraint was violated, with an optional counterexample.
    #[error("constraint violation: {constraint} (at {span:?})")]
    ConstraintViolation {
        constraint: String,
        counterexample: Option<String>,
        span: Span,
    },

    /// A `require` clause is unsatisfiable (always false), meaning the function
    /// can never legally be called.
    #[error("unsatisfiable require: {condition} (at {span:?})")]
    UnsatisfiableRequire { condition: String, span: Span },

    /// An invariant declared on an entity or state machine has been broken.
    #[error("invariant broken on {entity}: {invariant} (at {span:?})")]
    InvariantBroken {
        entity: String,
        invariant: String,
        span: Span,
    },

    /// A conservation law (e.g. money is neither created nor destroyed) was violated.
    #[error("conservation violation: {description} (at {span:?})")]
    ConservationViolation { description: String, span: Span },

    /// An effect that was not declared in the function's contract leaked out.
    #[error("undeclared effect `{effect}` in function `{fn_name}` (at {span:?})")]
    EffectLeak {
        effect: String,
        fn_name: String,
        span: Span,
    },
}
