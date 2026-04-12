//! Translates Nous AST expressions into Z3 SMT terms and checks satisfiability.

use std::collections::HashMap;

use nous_ast::expr::{BinOp, Expr, UnaryOp};
use z3::ast::{Ast, Bool, Int};
use z3::{Config, Context, SatResult, Solver};

/// Result of an SMT verification check.
#[derive(Debug)]
pub enum SmtResult {
    /// The constraint is proven to always hold.
    Verified,
    /// The constraint can be violated; includes a concrete counterexample.
    Counterexample(HashMap<String, String>),
    /// The solver could not determine the result within limits.
    Unknown(String),
}

/// Verify that a `require` condition is satisfiable (not always false).
/// Returns Verified if the condition CAN be true (not always false).
pub fn check_require_satisfiable(expr: &Expr) -> SmtResult {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    let mut env = SmtEnv::new(&ctx);

    match expr_to_z3_bool(&ctx, &mut env, expr) {
        Some(z3_expr) => {
            // Check if the negation is satisfiable (i.e., can the condition be false?)
            // If NOT(condition) is UNSAT, the condition always holds → Verified
            solver.assert(&z3_expr.not());
            match solver.check() {
                SatResult::Unsat => SmtResult::Verified,
                SatResult::Sat => {
                    let model = solver.get_model().unwrap();
                    let mut counterexample = HashMap::new();
                    for (name, var) in &env.int_vars {
                        if let Some(val) = model.eval(var, true) {
                            counterexample.insert(name.clone(), val.to_string());
                        }
                    }
                    for (name, var) in &env.bool_vars {
                        if let Some(val) = model.eval(var, true) {
                            counterexample.insert(name.clone(), val.to_string());
                        }
                    }
                    SmtResult::Counterexample(counterexample)
                }
                SatResult::Unknown => SmtResult::Unknown(
                    solver.get_reason_unknown().unwrap_or_default().to_string(),
                ),
            }
        }
        None => SmtResult::Unknown("expression too complex for SMT encoding".to_string()),
    }
}

/// Verify that given preconditions, a postcondition holds.
/// `requires` are assumed true, `ensure` must follow.
pub fn check_contract(requires: &[&Expr], ensure: &Expr) -> SmtResult {
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    let mut env = SmtEnv::new(&ctx);

    // Assert all preconditions
    for req in requires {
        if let Some(z3_req) = expr_to_z3_bool(&ctx, &mut env, req) {
            solver.assert(&z3_req);
        }
    }

    // Try to find a counterexample to the ensure clause
    match expr_to_z3_bool(&ctx, &mut env, ensure) {
        Some(z3_ensure) => {
            solver.assert(&z3_ensure.not());
            match solver.check() {
                SatResult::Unsat => SmtResult::Verified,
                SatResult::Sat => {
                    let model = solver.get_model().unwrap();
                    let mut counterexample = HashMap::new();
                    for (name, var) in &env.int_vars {
                        if let Some(val) = model.eval(var, true) {
                            counterexample.insert(name.clone(), val.to_string());
                        }
                    }
                    for (name, var) in &env.bool_vars {
                        if let Some(val) = model.eval(var, true) {
                            counterexample.insert(name.clone(), val.to_string());
                        }
                    }
                    SmtResult::Counterexample(counterexample)
                }
                SatResult::Unknown => SmtResult::Unknown(
                    solver.get_reason_unknown().unwrap_or_default().to_string(),
                ),
            }
        }
        None => SmtResult::Unknown("postcondition too complex for SMT encoding".to_string()),
    }
}

// ── Z3 environment ───────────────────────────────────

struct SmtEnv<'ctx> {
    ctx: &'ctx Context,
    int_vars: HashMap<String, Int<'ctx>>,
    bool_vars: HashMap<String, Bool<'ctx>>,
}

impl<'ctx> SmtEnv<'ctx> {
    fn new(ctx: &'ctx Context) -> Self {
        Self {
            ctx,
            int_vars: HashMap::new(),
            bool_vars: HashMap::new(),
        }
    }

    fn get_or_create_int(&mut self, name: &str) -> Int<'ctx> {
        if let Some(var) = self.int_vars.get(name) {
            var.clone()
        } else {
            let var = Int::new_const(self.ctx, name.to_string());
            self.int_vars.insert(name.to_string(), var.clone());
            var
        }
    }

    fn get_or_create_bool(&mut self, name: &str) -> Bool<'ctx> {
        if let Some(var) = self.bool_vars.get(name) {
            var.clone()
        } else {
            let var = Bool::new_const(self.ctx, name.to_string());
            self.bool_vars.insert(name.to_string(), var.clone());
            var
        }
    }
}

// ── AST → Z3 translation ────────────────────────────

/// Try to translate a Nous expression to a Z3 Bool.
fn expr_to_z3_bool<'ctx>(
    ctx: &'ctx Context,
    env: &mut SmtEnv<'ctx>,
    expr: &Expr,
) -> Option<Bool<'ctx>> {
    match expr {
        Expr::BoolLit(true) => Some(Bool::from_bool(ctx, true)),
        Expr::BoolLit(false) => Some(Bool::from_bool(ctx, false)),

        Expr::Ident(name) => Some(env.get_or_create_bool(name)),

        Expr::BinOp { op, left, right } => {
            match op {
                // Comparison ops: Int × Int → Bool
                BinOp::Eq => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l._eq(&r))
                }
                BinOp::Neq => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l._eq(&r).not())
                }
                BinOp::Lt => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l.lt(&r))
                }
                BinOp::Lte => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l.le(&r))
                }
                BinOp::Gt => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l.gt(&r))
                }
                BinOp::Gte => {
                    let l = expr_to_z3_int(ctx, env, &left.node)?;
                    let r = expr_to_z3_int(ctx, env, &right.node)?;
                    Some(l.ge(&r))
                }
                // Boolean ops: Bool × Bool → Bool
                BinOp::And => {
                    let l = expr_to_z3_bool(ctx, env, &left.node)?;
                    let r = expr_to_z3_bool(ctx, env, &right.node)?;
                    Some(Bool::and(ctx, &[&l, &r]))
                }
                BinOp::Or => {
                    let l = expr_to_z3_bool(ctx, env, &left.node)?;
                    let r = expr_to_z3_bool(ctx, env, &right.node)?;
                    Some(Bool::or(ctx, &[&l, &r]))
                }
                BinOp::Implies => {
                    let l = expr_to_z3_bool(ctx, env, &left.node)?;
                    let r = expr_to_z3_bool(ctx, env, &right.node)?;
                    Some(l.implies(&r))
                }
                _ => None,
            }
        }

        Expr::UnaryOp { op: UnaryOp::Not, operand } => {
            let inner = expr_to_z3_bool(ctx, env, &operand.node)?;
            Some(inner.not())
        }

        // Field access like `account.balance >= 0` — treat `account.balance` as
        // an integer variable named "account.balance"
        Expr::FieldAccess { .. } => {
            // This is ambiguous — could be bool or int. For now, if used in a
            // bool context (this function), treat as bool variable.
            let name = flatten_field_access(expr)?;
            Some(env.get_or_create_bool(&name))
        }

        _ => None,
    }
}

/// Try to translate a Nous expression to a Z3 Int.
fn expr_to_z3_int<'ctx>(
    ctx: &'ctx Context,
    env: &mut SmtEnv<'ctx>,
    expr: &Expr,
) -> Option<Int<'ctx>> {
    match expr {
        Expr::IntLit(n) => Some(Int::from_i64(ctx, *n)),

        Expr::Ident(name) => Some(env.get_or_create_int(name)),

        Expr::SelfRef => Some(env.get_or_create_int("self")),

        Expr::FieldAccess { .. } => {
            let name = flatten_field_access(expr)?;
            Some(env.get_or_create_int(&name))
        }

        Expr::BinOp { op, left, right } => {
            let l = expr_to_z3_int(ctx, env, &left.node)?;
            let r = expr_to_z3_int(ctx, env, &right.node)?;
            match op {
                BinOp::Add => Some(Int::add(ctx, &[&l, &r])),
                BinOp::Sub => Some(Int::sub(ctx, &[&l, &r])),
                BinOp::Mul => Some(Int::mul(ctx, &[&l, &r])),
                BinOp::Div => Some(l.div(&r)),
                BinOp::Mod => Some(l.modulo(&r)),
                _ => None,
            }
        }

        Expr::UnaryOp { op: UnaryOp::Neg, operand } => {
            let inner = expr_to_z3_int(ctx, env, &operand.node)?;
            Some(inner.unary_minus())
        }

        // Primed variables (postcondition): `from.balance'` → "from.balance'"
        Expr::Primed(inner) => {
            let base_name = match &inner.node {
                Expr::FieldAccess { .. } => flatten_field_access(&inner.node)?,
                Expr::Ident(name) => name.clone(),
                _ => return None,
            };
            Some(env.get_or_create_int(&format!("{base_name}'")))
        }

        _ => None,
    }
}

/// Flatten a chain of field accesses into a dotted name.
/// `account.balance` → "account.balance"
fn flatten_field_access(expr: &Expr) -> Option<String> {
    match expr {
        Expr::FieldAccess { object, field } => {
            let base = flatten_field_access(&object.node)?;
            Some(format!("{base}.{field}"))
        }
        Expr::Ident(name) => Some(name.clone()),
        _ => None,
    }
}
