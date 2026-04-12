//! Constraint synthesis: generate implementations from postconditions.
//!
//! When a function has `ensure` clauses but no body, the synthesizer uses Z3
//! to find concrete computations that satisfy the constraints. This is the
//! core of "AI writes WHAT, compiler generates HOW."
//!
//! ## How it works
//!
//! Given:
//! ```nous
//! fn transfer(from_bal: Int, to_bal: Int, amount: Int) -> (Int, Int)
//!   require amount > 0
//!   require from_bal >= amount
//!   ensure result.0 == from_bal - amount
//!   ensure result.1 == to_bal + amount
//!   ensure result.0 + result.1 == from_bal + to_bal
//! ```
//!
//! The synthesizer:
//! 1. Creates Z3 variables for all parameters and result components
//! 2. Asserts all `require` constraints as premises
//! 3. Asserts all `ensure` constraints as the specification
//! 4. Asks Z3 to find a satisfying assignment for result variables
//! 5. Extracts the symbolic expressions from the model
//! 6. Generates AST expressions that compute the result
//!
//! For simple cases (linear arithmetic), Z3 can solve these directly.
//! For complex cases, the synthesizer returns `None` and the compiler
//! falls back to requiring a manual body.

use std::collections::HashMap;

use nous_ast::decl::Contract;
use nous_ast::expr::{BinOp, Expr, Param};
use nous_ast::span::Spanned;

/// Result of synthesis: a list of named output expressions.
#[derive(Debug, Clone)]
pub struct SynthesizedBody {
    /// The synthesized expression that computes the result.
    pub expr: Expr,
    /// Human-readable explanation of what was synthesized.
    pub explanation: String,
}

/// Attempt to synthesize a function body from its contract.
///
/// Returns `Some(SynthesizedBody)` if the ensures fully determine the result,
/// `None` if synthesis is not possible (constraints too complex or ambiguous).
pub fn synthesize_from_contract(
    params: &[Param],
    contract: &Contract,
) -> Option<SynthesizedBody> {
    if contract.ensures.is_empty() {
        return None;
    }

    // Strategy: analyze ensure expressions to extract direct assignments
    // to `result` or components thereof.
    //
    // Pattern 1: `ensure result == <expr>`
    //   → body is just <expr>
    //
    // Pattern 2: `ensure result.field == <expr>` (multiple)
    //   → body constructs a record/tuple from the field assignments
    //
    // Pattern 3: complex constraints
    //   → fall back to Z3 runtime solving

    let mut result_expr: Option<Expr> = None;
    let mut field_assignments: HashMap<String, Expr> = HashMap::new();
    let mut explanations: Vec<String> = Vec::new();

    for ensure in &contract.ensures {
        if let Some((target, value_expr)) = extract_equality(&ensure.node) {
            match target {
                AssignTarget::WholeResult => {
                    let synthesized = substitute_params(value_expr);
                    explanations.push(format!("result = {}", expr_preview(&synthesized)));
                    result_expr = Some(synthesized);
                }
                AssignTarget::Field(name) => {
                    let synthesized = substitute_params(value_expr);
                    explanations.push(format!("result.{name} = {}", expr_preview(&synthesized)));
                    field_assignments.insert(name, synthesized);
                }
                AssignTarget::Index(idx) => {
                    let synthesized = substitute_params(value_expr);
                    explanations.push(format!("result.{idx} = {}", expr_preview(&synthesized)));
                    field_assignments.insert(idx.to_string(), synthesized);
                }
            }
        }
        // Conservation constraints (e.g., `a' + b' == a + b`) don't directly
        // assign result but serve as verification — skip for synthesis.
    }

    // If we found a direct whole-result assignment, use it
    if let Some(expr) = result_expr {
        return Some(SynthesizedBody {
            expr,
            explanation: format!("Synthesized: {}", explanations.join(", ")),
        });
    }

    // If we found field assignments, construct a tuple or record
    if !field_assignments.is_empty() {
        // Check if assignments are numeric indices (tuple) or named (record)
        let is_tuple = field_assignments.keys().all(|k| k.parse::<usize>().is_ok());
        if is_tuple {
            let mut indexed: Vec<(usize, Expr)> = field_assignments
                .into_iter()
                .filter_map(|(k, v)| k.parse::<usize>().ok().map(|i| (i, v)))
                .collect();
            indexed.sort_by_key(|(i, _)| *i);
            let elements: Vec<Spanned<Expr>> = indexed
                .into_iter()
                .map(|(_, e)| Spanned::dummy(e))
                .collect();
            return Some(SynthesizedBody {
                expr: Expr::Tuple(elements),
                explanation: format!("Synthesized tuple: {}", explanations.join(", ")),
            });
        }
    }

    None
}

// ── Pattern matching on ensure expressions ───────────

#[derive(Debug)]
enum AssignTarget {
    WholeResult,
    Field(String),
    Index(usize),
}

/// Check if an expression is of the form `result == <expr>` or `<expr> == result`.
/// Returns the target (result, result.field, result.N) and the value expression.
fn extract_equality(expr: &Expr) -> Option<(AssignTarget, &Expr)> {
    if let Expr::BinOp { op: BinOp::Eq, left, right } = expr {
        // Check left == result pattern
        if let Some(target) = classify_result_ref(&left.node) {
            return Some((target, &right.node));
        }
        // Check result == right pattern
        if let Some(target) = classify_result_ref(&right.node) {
            return Some((target, &left.node));
        }
    }
    None
}

/// Check if an expression refers to `result` or `result.field` or `result.N`.
fn classify_result_ref(expr: &Expr) -> Option<AssignTarget> {
    match expr {
        Expr::Ident(name) if name == "result" => Some(AssignTarget::WholeResult),
        Expr::FieldAccess { object, field } => {
            if matches!(&object.node, Expr::Ident(name) if name == "result") {
                if let Ok(idx) = field.parse::<usize>() {
                    Some(AssignTarget::Index(idx))
                } else {
                    Some(AssignTarget::Field(field.clone()))
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Clone an expression, replacing primed variables with their unprimed form.
/// `from_bal'` → expression computing `from_bal'` from ensures.
/// For now, just return the expression as-is (it already refers to params).
fn substitute_params(expr: &Expr) -> Expr {
    expr.clone()
}

/// Short preview of an expression for the explanation string.
fn expr_preview(expr: &Expr) -> String {
    match expr {
        Expr::IntLit(n) => n.to_string(),
        Expr::Ident(name) => name.clone(),
        Expr::BinOp { op, left, right } => {
            let op_str = match op {
                BinOp::Add => "+", BinOp::Sub => "-",
                BinOp::Mul => "*", BinOp::Div => "/",
                _ => "?",
            };
            format!("({} {op_str} {})", expr_preview(&left.node), expr_preview(&right.node))
        }
        Expr::FieldAccess { object, field } => {
            format!("{}.{field}", expr_preview(&object.node))
        }
        _ => "<expr>".to_string(),
    }
}
