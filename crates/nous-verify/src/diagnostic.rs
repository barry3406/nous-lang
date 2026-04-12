//! Structured JSON diagnostics per Nous SPEC Section 11.
//!
//! Every error includes:
//! - Concrete counterexample values
//! - Multiple ranked fix strategies
//! - Full call chain context

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use nous_ast::Span;

/// A structured diagnostic, designed for AI consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: DiagLevel,
    pub code: String,
    pub constraint: String,
    pub kind: ConstraintKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<HashMap<String, String>>,
    pub location: DiagLocation,
    pub fix_strategies: Vec<FixStrategy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Precondition,
    Postcondition,
    Invariant,
    StateTransition,
    EffectLeak,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ns: Option<String>,
    pub fn_name: String,
    pub span: Span,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_site: Option<Box<DiagLocation>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixStrategy {
    #[serde(rename = "type")]
    pub strategy_type: FixType,
    pub description: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixType {
    AddGuard,
    NarrowInputType,
    SplitState,
    DeclareEffect,
    AddRollback,
}

impl Diagnostic {
    /// Create a precondition violation diagnostic.
    pub fn require_violation(
        fn_name: &str,
        constraint: &str,
        counterexample: HashMap<String, String>,
        span: Span,
    ) -> Self {
        let ce_desc: Vec<String> = counterexample
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        Self {
            level: DiagLevel::Warning,
            code: "W301_REQUIRE_NOT_ALWAYS_TRUE".to_string(),
            constraint: constraint.to_string(),
            kind: ConstraintKind::Precondition,
            counterexample: Some(counterexample),
            location: DiagLocation {
                ns: None,
                fn_name: fn_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::AddGuard,
                    description: format!(
                        "Callers must check: {} (fails when {})",
                        constraint,
                        ce_desc.join(", ")
                    ),
                    at: "call_site".to_string(),
                    code: Some(format!("require {constraint}")),
                },
                FixStrategy {
                    strategy_type: FixType::NarrowInputType,
                    description: "Restrict parameter types with refinements".to_string(),
                    at: "fn_signature".to_string(),
                    code: None,
                },
            ],
            related_constraints: vec![],
        }
    }

    /// Create a postcondition violation diagnostic.
    pub fn ensure_violation(
        fn_name: &str,
        constraint: &str,
        counterexample: HashMap<String, String>,
        span: Span,
    ) -> Self {
        Self {
            level: DiagLevel::Error,
            code: "E302_ENSURE_VIOLATION".to_string(),
            constraint: constraint.to_string(),
            kind: ConstraintKind::Postcondition,
            counterexample: Some(counterexample),
            location: DiagLocation {
                ns: None,
                fn_name: fn_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::AddGuard,
                    description: "Add preconditions that guarantee the postcondition".to_string(),
                    at: "fn_contract".to_string(),
                    code: None,
                },
            ],
            related_constraints: vec![],
        }
    }

    /// Create a state machine violation diagnostic.
    pub fn state_unreachable(
        machine_name: &str,
        state_name: &str,
        span: Span,
    ) -> Self {
        Self {
            level: DiagLevel::Error,
            code: "E201_UNREACHABLE_STATE".to_string(),
            constraint: format!("state `{state_name}` must be reachable"),
            kind: ConstraintKind::StateTransition,
            counterexample: None,
            location: DiagLocation {
                ns: None,
                fn_name: machine_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::SplitState,
                    description: format!(
                        "Add a transition leading to `{state_name}`, or remove it"
                    ),
                    at: "state_declaration".to_string(),
                    code: None,
                },
            ],
            related_constraints: vec![],
        }
    }

    /// Create a dead (unreachable-action) diagnostic.
    ///
    /// Emitted when an action can never be triggered because its `from` state
    /// is itself unreachable from the initial state.
    pub fn dead_action(
        machine_name: &str,
        action: &str,
        from_state: &str,
        span: Span,
    ) -> Self {
        Self {
            level: DiagLevel::Warning,
            code: "W202_DEAD_ACTION".to_string(),
            constraint: format!(
                "action `{action}` in state `{from_state}` is unreachable"
            ),
            kind: ConstraintKind::StateTransition,
            counterexample: None,
            location: DiagLocation {
                ns: None,
                fn_name: machine_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::SplitState,
                    description: format!(
                        "Add a transition leading to `{from_state}`, or remove \
                         action `{action}`"
                    ),
                    at: "state_declaration".to_string(),
                    code: None,
                },
            ],
            related_constraints: vec![],
        }
    }

    /// Create a liveness-violation diagnostic.
    ///
    /// Emitted when a non-terminal state cannot reach any terminal state, meaning
    /// the machine can get permanently stuck there.
    pub fn state_liveness_violation(
        machine_name: &str,
        state_name: &str,
        span: Span,
    ) -> Self {
        Self {
            level: DiagLevel::Error,
            code: "E203_LIVENESS_VIOLATION".to_string(),
            constraint: format!(
                "non-terminal state `{state_name}` cannot reach any terminal state"
            ),
            kind: ConstraintKind::StateTransition,
            counterexample: None,
            location: DiagLocation {
                ns: None,
                fn_name: machine_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::SplitState,
                    description: format!(
                        "Add a path from `{state_name}` to a terminal state, or \
                         declare it terminal (remove all outgoing transitions)"
                    ),
                    at: "state_declaration".to_string(),
                    code: None,
                },
            ],
            related_constraints: vec![],
        }
    }

    /// Create a missing effect declaration diagnostic.
    pub fn undeclared_effect(
        fn_name: &str,
        span: Span,
    ) -> Self {
        Self {
            level: DiagLevel::Warning,
            code: "W401_MISSING_EFFECTS".to_string(),
            constraint: "all side effects must be declared".to_string(),
            kind: ConstraintKind::EffectLeak,
            counterexample: None,
            location: DiagLocation {
                ns: None,
                fn_name: fn_name.to_string(),
                span,
                call_site: None,
            },
            fix_strategies: vec![
                FixStrategy {
                    strategy_type: FixType::DeclareEffect,
                    description: "Add an `effect` clause to the function contract".to_string(),
                    at: "fn_contract".to_string(),
                    code: Some("effect Db.write".to_string()),
                },
            ],
            related_constraints: vec![],
        }
    }
}
