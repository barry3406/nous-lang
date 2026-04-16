use std::collections::{HashMap, HashSet};

use nous_ast::decl::{Decl, EntityDecl, EnumDecl, FlowDecl, FnDecl, StateDecl};
use nous_ast::expr::{Expr, Pattern};
use nous_ast::span::Spanned;
use nous_ast::types::{TypeDecl, TypeExpr};
use nous_ast::{Program, Span};

use crate::env::{EntityDef, FieldDef, FnSig, ParamDef, StateDef, TransitionDef, TypeEnv};
use crate::error::TypeError;
// ContractViolationKind is re-exported for callers; used here when contract
// static-verification is implemented (see TODO comments below).
#[allow(unused_imports)]
use crate::error::ContractViolationKind;

// ---------------------------------------------------------------------------
// TypeChecker
// ---------------------------------------------------------------------------

/// The top-level type checker for a single Nous program.
///
/// Usage:
/// ```rust,ignore
/// let checker = TypeChecker::new();
/// checker.check(&program)?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct TypeChecker {
    // Stored so callers can inspect the final environment after checking.
    env: TypeEnv,
}

type LocalScope = HashMap<String, TypeExpr>;

const UNKNOWN_TYPE: &str = "_";

impl TypeChecker {
    /// Construct a fresh checker with an empty (builtin-seeded) environment.
    pub fn new() -> Self {
        Self {
            env: TypeEnv::new(),
        }
    }

    /// Return a reference to the type environment accumulated during checking.
    pub fn env(&self) -> &TypeEnv {
        &self.env
    }

    // -----------------------------------------------------------------------
    // Entry point
    // -----------------------------------------------------------------------

    /// Type-check an entire program, returning all errors found (not
    /// stopping at the first).
    ///
    /// The pass proceeds in two phases:
    ///
    /// 1. **Pre-scan** — collect every declared name so that forward
    ///    references between declarations are valid.
    /// 2. **Full check** — validate each declaration in detail.
    pub fn check(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        let mut errors: Vec<TypeError> = Vec::new();

        // --- Phase 1: pre-scan (collect names) ----------------------------
        self.prescan_declarations(program, &mut errors);

        // --- Phase 2: detailed per-declaration checks ---------------------
        for spanned_decl in &program.declarations {
            match &spanned_decl.node {
                Decl::Entity(decl) => {
                    self.check_entity(decl, spanned_decl.span, &mut errors);
                }
                Decl::State(decl) => {
                    self.check_state(decl, spanned_decl.span, &mut errors);
                }
                Decl::Fn(decl) => {
                    self.check_fn(decl, spanned_decl.span, &mut errors);
                }
                Decl::Flow(decl) => {
                    self.check_flow(decl, spanned_decl.span, &mut errors);
                }
                Decl::Enum(decl) => {
                    self.check_enum(decl, spanned_decl.span, &mut errors);
                }
                Decl::Type(decl) => {
                    self.check_type_alias(decl, spanned_decl.span, &mut errors);
                }
                // TODO: check EndpointDecl — validate field types and handler type
                // TODO: check HandlerDecl — validate each binding's effect exists
                // TODO: check EffectDecl — register effects for UndeclaredEffect checks
                // Namespace, Use, Main: no type-level checks needed yet.
                _ => {}
            }
        }

        // --- Phase 3: effect propagation checks ---------------------------
        // For each fn/flow, verify that any function it calls does not carry
        // effects that the caller has not declared.
        for spanned_decl in &program.declarations {
            match &spanned_decl.node {
                Decl::Fn(decl) => {
                    self.check_fn_effects(decl, spanned_decl.span, &mut errors);
                }
                Decl::Flow(decl) => {
                    self.check_flow_effects(decl, spanned_decl.span, &mut errors);
                }
                _ => {}
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    // -----------------------------------------------------------------------
    // Phase 1: prescan
    // -----------------------------------------------------------------------

    /// Walk declarations once to register all top-level names in the
    /// environment, enabling forward references.
    fn prescan_declarations(&mut self, program: &Program, _errors: &mut Vec<TypeError>) {
        for spanned_decl in &program.declarations {
            match &spanned_decl.node {
                Decl::Entity(decl) => {
                    self.env.register_type_name(&decl.name);
                }
                Decl::Enum(decl) => {
                    self.env.register_type_name(&decl.name);
                }
                Decl::Type(decl) => {
                    self.env.register_type_name(&decl.name);
                }
                Decl::Fn(decl) => {
                    self.env.define_fn(FnSig {
                        name: decl.name.clone(),
                        params: decl
                            .params
                            .iter()
                            .map(|param| ParamDef {
                                name: param.name.clone(),
                                ty: param.ty.node.clone(),
                            })
                            .collect(),
                        return_type: decl.return_type.node.clone(),
                        effects: decl.contract.effects.clone(),
                        declared_trust: decl.contract.trust.clone(),
                    });
                }
                Decl::Flow(decl) => {
                    self.env.define_fn(FnSig {
                        name: decl.name.clone(),
                        params: decl
                            .params
                            .iter()
                            .map(|param| ParamDef {
                                name: param.name.clone(),
                                ty: param.ty.node.clone(),
                            })
                            .collect(),
                        return_type: decl.return_type.node.clone(),
                        effects: decl.contract.effects.clone(),
                        declared_trust: decl.contract.trust.clone(),
                    });
                }
                _ => {}
            }
        }
    }

    fn check_type_alias(&mut self, decl: &TypeDecl, _span: Span, errors: &mut Vec<TypeError>) {
        self.check_type_expr(&decl.ty.node, decl.ty.span, errors);
        self.env
            .define_type_alias(decl.name.clone(), decl.ty.node.clone());
    }

    // -----------------------------------------------------------------------
    // Entity checking
    // -----------------------------------------------------------------------

    fn check_entity(&mut self, decl: &EntityDecl, span: Span, errors: &mut Vec<TypeError>) {
        let mut field_defs: Vec<FieldDef> = Vec::new();
        let mut invariant_scope: LocalScope = HashMap::new();

        for spanned_field in &decl.fields {
            let field = &spanned_field.node;
            let field_span = spanned_field.span;

            // Validate the field's type expression.
            self.check_type_expr(&field.ty.node, field_span, errors);

            field_defs.push(FieldDef {
                name: field.name.clone(),
                ty: field.ty.node.clone(),
            });
            invariant_scope.insert(field.name.clone(), field.ty.node.clone());
        }

        let def = EntityDef {
            name: decl.name.clone(),
            fields: field_defs,
        };
        self.env.define_entity(def);

        invariant_scope.insert("self".to_string(), TypeExpr::Named(decl.name.clone()));
        for invariant in &decl.invariants {
            self.check_constraint_expr(&invariant.node, invariant.span, &invariant_scope, errors);
        }

        let _ = span;
    }

    // -----------------------------------------------------------------------
    // State machine checking
    // -----------------------------------------------------------------------

    fn check_state(&mut self, decl: &StateDecl, _span: Span, errors: &mut Vec<TypeError>) {
        // Collect all state names that appear in transitions.
        let mut all_states: HashSet<String> = HashSet::new();
        let mut transition_defs: Vec<TransitionDef> = Vec::new();

        for transition in &decl.transitions {
            all_states.insert(transition.from.clone());
            all_states.insert(transition.to.clone());
            transition_defs.push(TransitionDef {
                from: transition.from.clone(),
                action: transition.action.clone(),
                to: transition.to.clone(),
            });

            // TODO: validate parameter types on transition actions
        }

        // Build adjacency for reachability analysis.
        // The first `from` state that appears is treated as the initial state.
        // A proper Nous grammar will annotate an explicit `initial` state;
        // until then we use the first `from` as a proxy.
        let initial_state = decl.transitions.first().map(|t| t.from.as_str());

        if let Some(init) = initial_state {
            let reachable = reachable_states(init, &transition_defs);

            for state in &all_states {
                if !reachable.contains(state.as_str()) {
                    // A state exists but cannot be reached from the initial state.
                    errors.push(TypeError::UnreachableState {
                        state: state.clone(),
                        machine: decl.name.clone(),
                        // TODO: carry per-state spans from the parser
                        span: Span::dummy(),
                    });
                }
            }
        }

        // TODO: detect orphan (sink) states that have no outgoing transitions
        //       and are not explicitly marked as terminal.
        // Hint: collect states with no outgoing edge and emit MissingTransition.

        let def = StateDef {
            name: decl.name.clone(),
            states: all_states,
            transitions: transition_defs,
        };
        self.env.define_state(def);
    }

    // -----------------------------------------------------------------------
    // Function checking
    // -----------------------------------------------------------------------

    fn check_fn(&mut self, decl: &FnDecl, _span: Span, errors: &mut Vec<TypeError>) {
        // Validate parameter types.
        let mut param_defs: Vec<ParamDef> = Vec::new();
        let mut scope: LocalScope = HashMap::new();
        for param in &decl.params {
            self.check_type_expr(&param.ty.node, param.ty.span, errors);
            param_defs.push(ParamDef {
                name: param.name.clone(),
                ty: param.ty.node.clone(),
            });
            scope.insert(param.name.clone(), param.ty.node.clone());
        }

        // Validate the return type.
        self.check_type_expr(&decl.return_type.node, decl.return_type.span, errors);
        self.check_contract_clauses(
            &decl.contract.requires,
            &decl.contract.ensures,
            &scope,
            Some(&decl.return_type.node),
            errors,
        );

        let body_is_empty = matches!(&decl.body.node, Expr::Void)
            || matches!(&decl.body.node, Expr::Block(stmts) if stmts.is_empty());
        if !(body_is_empty && !decl.contract.ensures.is_empty()) {
            self.check_expr_against(
                &decl.body.node,
                decl.body.span,
                &decl.return_type.node,
                &mut scope,
                errors,
            );
        }

        // TODO: check that each effect used in the body appears in contract.effects
        //       (emit UndeclaredEffect for violations)
        // TODO: statically verify require/ensures clauses where possible
        //       (emit ContractViolation for violations)

        let sig = FnSig {
            name: decl.name.clone(),
            params: param_defs,
            return_type: decl.return_type.node.clone(),
            effects: decl.contract.effects.clone(),
            declared_trust: decl.contract.trust.clone(),
        };
        self.env.define_fn(sig);
    }

    // -----------------------------------------------------------------------
    // Flow checking
    // -----------------------------------------------------------------------

    fn check_flow(&mut self, decl: &FlowDecl, span: Span, errors: &mut Vec<TypeError>) {
        // Validate parameter types.
        let mut param_defs: Vec<ParamDef> = Vec::new();
        let mut scope: LocalScope = HashMap::new();
        for param in &decl.params {
            self.check_type_expr(&param.ty.node, param.ty.span, errors);
            param_defs.push(ParamDef {
                name: param.name.clone(),
                ty: param.ty.node.clone(),
            });
            scope.insert(param.name.clone(), param.ty.node.clone());
        }

        // Validate the return type.
        self.check_type_expr(&decl.return_type.node, decl.return_type.span, errors);
        self.check_contract_clauses(
            &decl.contract.requires,
            &decl.contract.ensures,
            &scope,
            Some(&decl.return_type.node),
            errors,
        );

        let Some((ok_type, err_type)) = self.unwrap_result_signature(&decl.return_type.node) else {
            errors.push(TypeError::TypeMismatch {
                expected: "Result[T, E]".to_string(),
                got: self.type_to_string(&decl.return_type.node),
                span,
            });
            return;
        };

        for step in &decl.steps {
            let mut step_scope = scope.clone();
            let step_type =
                self.infer_expr_type(&step.body.node, step.body.span, &mut step_scope, errors);
            if let Some(step_type) = step_type {
                let (unwrapped, step_error) = self.unwrap_result_expr_type(&step_type);
                if let Some(step_error) = step_error {
                    self.expect_compatible(&err_type, &step_error, step.body.span, errors);
                }
                scope.insert(format!("{}_result", step.name), unwrapped);
            }

            if !matches!(&step.rollback.node, Expr::Void)
                && !matches!(&step.rollback.node, Expr::Ident(name) if name == "nothing")
            {
                self.infer_expr_type(
                    &step.rollback.node,
                    step.rollback.span,
                    &mut scope.clone(),
                    errors,
                );
            }
        }

        if let Some(last_step) = decl.steps.last() {
            if let Some(last_step_ty) = scope.get(&format!("{}_result", last_step.name)) {
                self.expect_compatible(&ok_type, last_step_ty, last_step.body.span, errors);
            }
        }

        // TODO: check that effects in steps match the contract

        let sig = FnSig {
            name: decl.name.clone(),
            params: param_defs,
            return_type: decl.return_type.node.clone(),
            effects: decl.contract.effects.clone(),
            declared_trust: decl.contract.trust.clone(),
        };
        self.env.define_fn(sig);
    }

    // -----------------------------------------------------------------------
    // Enum checking
    // -----------------------------------------------------------------------

    fn check_enum(&mut self, decl: &EnumDecl, _span: Span, errors: &mut Vec<TypeError>) {
        // Collect variant info for exhaustiveness checking at match sites.
        // The variant names are stored under the enum's own type name which
        // was already registered during the prescan.
        let mut _variant_names: Vec<String> = Vec::new();

        for variant in &decl.variants {
            _variant_names.push(variant.name.clone());

            // Validate each field type inside the variant.
            for field in &variant.fields {
                self.check_type_expr(&field.ty.node, field.ty.span, errors);
            }
        }

        // TODO: store variant lists in the environment so that match
        //       exhaustiveness can be checked (emit NonExhaustiveMatch).
    }

    // -----------------------------------------------------------------------
    // Effect propagation checking
    // -----------------------------------------------------------------------

    /// Check that a `fn` declaration does not call any function whose effects
    /// are not declared by the caller.
    fn check_fn_effects(&self, decl: &FnDecl, span: Span, errors: &mut Vec<TypeError>) {
        let caller_effects: HashSet<&str> =
            decl.contract.effects.iter().map(String::as_str).collect();
        let mut called: HashSet<String> = HashSet::new();
        collect_calls_in_expr(&decl.body.node, &mut called);
        self.check_effect_leakage(&decl.name, span, &caller_effects, &called, errors);
    }

    /// Check that a `flow` declaration does not call any function whose effects
    /// are not declared by the caller.
    fn check_flow_effects(&self, decl: &FlowDecl, span: Span, errors: &mut Vec<TypeError>) {
        let caller_effects: HashSet<&str> =
            decl.contract.effects.iter().map(String::as_str).collect();
        let mut called: HashSet<String> = HashSet::new();
        for step in &decl.steps {
            collect_calls_in_expr(&step.body.node, &mut called);
            collect_calls_in_expr(&step.rollback.node, &mut called);
        }
        self.check_effect_leakage(&decl.name, span, &caller_effects, &called, errors);
    }

    /// For each function in `called_fns` that the environment knows about,
    /// emit `UndeclaredEffect` for every effect not covered by `caller_effects`.
    fn check_effect_leakage(
        &self,
        fn_name: &str,
        span: Span,
        caller_effects: &HashSet<&str>,
        called_fns: &HashSet<String>,
        errors: &mut Vec<TypeError>,
    ) {
        for callee_name in called_fns {
            if let Some(sig) = self.env.lookup_fn(callee_name) {
                for effect in &sig.effects {
                    if !caller_effects.contains(effect.as_str()) {
                        errors.push(TypeError::UndeclaredEffect {
                            effect: effect.clone(),
                            fn_name: fn_name.to_owned(),
                            span,
                        });
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Contract / constraint typing
    // -----------------------------------------------------------------------

    fn check_contract_clauses(
        &self,
        requires: &[nous_ast::decl::RequireClause],
        ensures: &[nous_ast::decl::EnsureClause],
        base_scope: &LocalScope,
        result_type: Option<&TypeExpr>,
        errors: &mut Vec<TypeError>,
    ) {
        for require in requires {
            self.check_constraint_expr(
                &require.condition.node,
                require.condition.span,
                base_scope,
                errors,
            );
            if let Some(else_expr) = &require.else_expr {
                let mut else_scope = base_scope.clone();
                let _ =
                    self.infer_expr_type(&else_expr.node, else_expr.span, &mut else_scope, errors);
            }
        }

        let mut ensure_scope = base_scope.clone();
        if let Some(result_type) = result_type {
            ensure_scope.insert("result".to_string(), result_type.clone());
        }

        for ensure in ensures {
            self.check_constraint_expr(&ensure.condition.node, ensure.condition.span, &ensure_scope, errors);
        }
    }

    fn check_constraint_expr(
        &self,
        expr: &Expr,
        span: Span,
        scope: &LocalScope,
        errors: &mut Vec<TypeError>,
    ) {
        let mut scope = scope.clone();
        self.check_expr_against(
            expr,
            span,
            &TypeExpr::Named("Bool".to_string()),
            &mut scope,
            errors,
        );
    }

    // -----------------------------------------------------------------------
    // Expression typing
    // -----------------------------------------------------------------------

    fn check_expr_against(
        &self,
        expr: &Expr,
        span: Span,
        expected: &TypeExpr,
        scope: &mut LocalScope,
        errors: &mut Vec<TypeError>,
    ) {
        if let Some(got) = self.infer_expr_type(expr, span, scope, errors) {
            self.expect_compatible(expected, &got, span, errors);
        }
    }

    fn infer_expr_type(
        &self,
        expr: &Expr,
        span: Span,
        scope: &mut LocalScope,
        errors: &mut Vec<TypeError>,
    ) -> Option<TypeExpr> {
        match expr {
            Expr::IntLit(_) => Some(TypeExpr::Named("Int".to_string())),
            Expr::DecLit(_) => Some(TypeExpr::Named("Dec".to_string())),
            Expr::StringLit(_) => Some(TypeExpr::Named("Text".to_string())),
            Expr::BoolLit(_) => Some(TypeExpr::Named("Bool".to_string())),
            Expr::Void => Some(TypeExpr::Void),
            Expr::Ident(name) if name == "nothing" => Some(TypeExpr::Void),
            Expr::Ident(name) => scope.get(name).cloned(),
            Expr::SelfRef => scope.get("self").cloned(),

            Expr::FieldAccess { object, field } => {
                let object_ty = self.infer_expr_type(&object.node, object.span, scope, errors)?;
                match self.normalize_type(&object_ty) {
                    TypeExpr::Named(name) => self.env.get_entity(&name).and_then(|entity| {
                        entity
                            .fields
                            .iter()
                            .find(|candidate| candidate.name == *field)
                            .map(|field_def| field_def.ty.clone())
                    }),
                    _ => None,
                }
            }

            Expr::Primed(inner) | Expr::Pre(inner) | Expr::Transaction(inner) => {
                self.infer_expr_type(&inner.node, inner.span, scope, errors)
            }

            Expr::UnaryOp { op, operand } => {
                let operand_ty =
                    self.infer_expr_type(&operand.node, operand.span, scope, errors)?;
                match op {
                    nous_ast::expr::UnaryOp::Neg => {
                        if self.is_numeric_type(&operand_ty) {
                            Some(self.normalize_type(&operand_ty))
                        } else {
                            self.expect_compatible(
                                &TypeExpr::Named("Int".to_string()),
                                &operand_ty,
                                span,
                                errors,
                            );
                            None
                        }
                    }
                    nous_ast::expr::UnaryOp::Not => {
                        self.expect_compatible(
                            &TypeExpr::Named("Bool".to_string()),
                            &operand_ty,
                            span,
                            errors,
                        );
                        Some(TypeExpr::Named("Bool".to_string()))
                    }
                }
            }

            Expr::BinOp { op, left, right } => {
                let left_ty = self.infer_expr_type(&left.node, left.span, scope, errors)?;
                let right_ty = self.infer_expr_type(&right.node, right.span, scope, errors)?;

                match op {
                    nous_ast::expr::BinOp::Add
                    | nous_ast::expr::BinOp::Sub
                    | nous_ast::expr::BinOp::Mul
                    | nous_ast::expr::BinOp::Div
                    | nous_ast::expr::BinOp::Mod => {
                        if self.is_numeric_type(&left_ty) && self.is_numeric_type(&right_ty) {
                            if matches!(
                                self.normalize_type(&left_ty),
                                TypeExpr::Named(ref name) if name == "Dec"
                            ) || matches!(
                                self.normalize_type(&right_ty),
                                TypeExpr::Named(ref name) if name == "Dec"
                            ) {
                                Some(TypeExpr::Named("Dec".to_string()))
                            } else {
                                Some(TypeExpr::Named("Int".to_string()))
                            }
                        } else {
                            self.expect_compatible(
                                &TypeExpr::Named("Int".to_string()),
                                &left_ty,
                                left.span,
                                errors,
                            );
                            self.expect_compatible(
                                &TypeExpr::Named("Int".to_string()),
                                &right_ty,
                                right.span,
                                errors,
                            );
                            None
                        }
                    }
                    nous_ast::expr::BinOp::Eq | nous_ast::expr::BinOp::Neq => {
                        self.expect_compatible(&left_ty, &right_ty, span, errors);
                        Some(TypeExpr::Named("Bool".to_string()))
                    }
                    nous_ast::expr::BinOp::Lt
                    | nous_ast::expr::BinOp::Lte
                    | nous_ast::expr::BinOp::Gt
                    | nous_ast::expr::BinOp::Gte => {
                        self.expect_compatible(
                            &TypeExpr::Named("Int".to_string()),
                            &left_ty,
                            left.span,
                            errors,
                        );
                        self.expect_compatible(
                            &TypeExpr::Named("Int".to_string()),
                            &right_ty,
                            right.span,
                            errors,
                        );
                        Some(TypeExpr::Named("Bool".to_string()))
                    }
                    nous_ast::expr::BinOp::And
                    | nous_ast::expr::BinOp::Or
                    | nous_ast::expr::BinOp::Implies => {
                        self.expect_compatible(
                            &TypeExpr::Named("Bool".to_string()),
                            &left_ty,
                            left.span,
                            errors,
                        );
                        self.expect_compatible(
                            &TypeExpr::Named("Bool".to_string()),
                            &right_ty,
                            right.span,
                            errors,
                        );
                        Some(TypeExpr::Named("Bool".to_string()))
                    }
                }
            }

            Expr::Call { func, args } => {
                let Expr::Ident(name) = &func.node else {
                    return None;
                };

                // Map each arg to its inferred type. If inference fails,
                // use UNKNOWN_TYPE so the arg still counts — otherwise we
                // miscount arity on calls whose args contain unknowns.
                let arg_types: Vec<(TypeExpr, Span)> = args
                    .iter()
                    .map(|arg| {
                        let ty = self.infer_expr_type(&arg.node, arg.span, scope, errors)
                            .unwrap_or_else(|| TypeExpr::Named(UNKNOWN_TYPE.to_string()));
                        (ty, arg.span)
                    })
                    .collect();

                self.infer_named_call(name, &arg_types, span, errors)
            }

            Expr::Pipe { value, func, args } => {
                let Expr::Ident(name) = &func.node else {
                    return None;
                };

                let mut arg_types = Vec::with_capacity(args.len() + 1);
                if let Some(value_ty) = self.infer_expr_type(&value.node, value.span, scope, errors)
                {
                    arg_types.push((value_ty, value.span));
                }
                for arg in args {
                    if let Some(arg_ty) = self.infer_expr_type(&arg.node, arg.span, scope, errors) {
                        arg_types.push((arg_ty, arg.span));
                    }
                }

                self.infer_named_call(name, &arg_types, span, errors)
            }

            Expr::Try(inner) => {
                let inner_ty = self.infer_expr_type(&inner.node, inner.span, scope, errors)?;
                let Some((ok_ty, _)) = self.unwrap_result_signature(&inner_ty) else {
                    errors.push(TypeError::TypeMismatch {
                        expected: "Result[T, E]".to_string(),
                        got: self.type_to_string(&inner_ty),
                        span,
                    });
                    return None;
                };
                Some(ok_ty)
            }

            Expr::Let { pattern, ty, value } => {
                let value_ty = self.infer_expr_type(&value.node, value.span, scope, errors)?;
                let binding_ty = if let Some(annotation) = ty {
                    self.check_type_expr(&annotation.node, annotation.span, errors);
                    self.expect_compatible(&annotation.node, &value_ty, value.span, errors);
                    annotation.node.clone()
                } else {
                    value_ty
                };

                match &pattern.node {
                    Pattern::Ident(name) => {
                        scope.insert(name.clone(), binding_ty);
                    }
                    Pattern::Wildcard => {}
                    _ => {}
                }

                Some(TypeExpr::Void)
            }

            Expr::Block(stmts) => {
                if stmts.is_empty() {
                    return Some(TypeExpr::Void);
                }

                let mut block_scope = scope.clone();
                let mut last_ty = TypeExpr::Void;
                for stmt in stmts {
                    if let Some(stmt_ty) =
                        self.infer_expr_type(&stmt.node, stmt.span, &mut block_scope, errors)
                    {
                        last_ty = stmt_ty;
                    }
                }
                Some(last_ty)
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_expr_against(
                    &condition.node,
                    condition.span,
                    &TypeExpr::Named("Bool".to_string()),
                    scope,
                    errors,
                );

                let then_ty = self.infer_expr_type(
                    &then_branch.node,
                    then_branch.span,
                    &mut scope.clone(),
                    errors,
                )?;
                if let Some(else_branch) = else_branch {
                    let else_ty = self.infer_expr_type(
                        &else_branch.node,
                        else_branch.span,
                        &mut scope.clone(),
                        errors,
                    )?;
                    self.merge_types(&then_ty, &else_ty).or_else(|| {
                        errors.push(TypeError::TypeMismatch {
                            expected: self.type_to_string(&then_ty),
                            got: self.type_to_string(&else_ty),
                            span,
                        });
                        None
                    })
                } else {
                    Some(TypeExpr::Void)
                }
            }

            Expr::Match { scrutinee, arms } => {
                let scrutinee_ty =
                    self.infer_expr_type(&scrutinee.node, scrutinee.span, scope, errors)?;
                let mut merged: Option<TypeExpr> = None;

                for arm in arms {
                    let mut arm_scope = scope.clone();
                    self.bind_pattern(
                        &arm.pattern.node,
                        &scrutinee_ty,
                        arm.pattern.span,
                        &mut arm_scope,
                        errors,
                    );
                    let Some(body_ty) =
                        self.infer_expr_type(&arm.body.node, arm.body.span, &mut arm_scope, errors)
                    else {
                        continue;
                    };

                    merged = match merged {
                        None => Some(body_ty),
                        Some(existing) => self.merge_types(&existing, &body_ty).or_else(|| {
                            errors.push(TypeError::TypeMismatch {
                                expected: self.type_to_string(&existing),
                                got: self.type_to_string(&body_ty),
                                span: arm.body.span,
                            });
                            Some(existing)
                        }),
                    };
                }

                merged.or(Some(TypeExpr::Void))
            }

            Expr::Record { name, fields } => {
                let Some(entity) = self.env.get_entity(name) else {
                    return None;
                };

                for (field_name, field_expr) in fields {
                    if let Some(expected_field) =
                        entity.fields.iter().find(|field| field.name == *field_name)
                    {
                        self.check_expr_against(
                            &field_expr.node,
                            field_expr.span,
                            &expected_field.ty,
                            scope,
                            errors,
                        );
                    }
                }

                Some(TypeExpr::Named(name.clone()))
            }

            Expr::RecordUpdate { base, updates } => {
                let base_ty = self.infer_expr_type(&base.node, base.span, scope, errors)?;
                let TypeExpr::Named(entity_name) = self.normalize_type(&base_ty) else {
                    return Some(base_ty);
                };

                if let Some(entity) = self.env.get_entity(&entity_name) {
                    for (field_name, field_expr) in updates {
                        if let Some(expected_field) =
                            entity.fields.iter().find(|field| field.name == *field_name)
                        {
                            self.check_expr_against(
                                &field_expr.node,
                                field_expr.span,
                                &expected_field.ty,
                                scope,
                                errors,
                            );
                        }
                    }
                }

                Some(TypeExpr::Named(entity_name))
            }

            Expr::Tuple(elements) => Some(TypeExpr::Tuple(
                elements
                    .iter()
                    .filter_map(|elem| {
                        self.infer_expr_type(&elem.node, elem.span, scope, errors)
                            .map(Spanned::dummy)
                    })
                    .collect(),
            )),

            Expr::List(elements) => {
                let mut elem_ty: Option<TypeExpr> = None;
                for elem in elements {
                    let Some(current_ty) =
                        self.infer_expr_type(&elem.node, elem.span, scope, errors)
                    else {
                        continue;
                    };
                    elem_ty = match elem_ty {
                        None => Some(current_ty),
                        Some(existing) => {
                            self.merge_types(&existing, &current_ty).or(Some(existing))
                        }
                    };
                }

                Some(TypeExpr::Generic {
                    name: "List".to_string(),
                    args: vec![Spanned::dummy(
                        elem_ty.unwrap_or_else(|| TypeExpr::Named(UNKNOWN_TYPE.to_string())),
                    )],
                })
            }

            Expr::Lambda { .. } => None,
            Expr::Return(inner) => self.infer_expr_type(&inner.node, inner.span, scope, errors),
            Expr::Ok(inner) => Some(TypeExpr::Generic {
                name: "Result".to_string(),
                args: vec![
                    Spanned::dummy(self.infer_expr_type(&inner.node, inner.span, scope, errors)?),
                    Spanned::dummy(TypeExpr::Named(UNKNOWN_TYPE.to_string())),
                ],
            }),
            Expr::Err(inner) => Some(TypeExpr::Generic {
                name: "Result".to_string(),
                args: vec![
                    Spanned::dummy(TypeExpr::Named(UNKNOWN_TYPE.to_string())),
                    Spanned::dummy(self.infer_expr_type(&inner.node, inner.span, scope, errors)?),
                ],
            }),
            Expr::Require {
                condition,
                else_expr,
            } => {
                self.check_expr_against(
                    &condition.node,
                    condition.span,
                    &TypeExpr::Named("Bool".to_string()),
                    scope,
                    errors,
                );
                if let Some(else_expr) = else_expr {
                    let _ = self.infer_expr_type(&else_expr.node, else_expr.span, scope, errors);
                }
                Some(TypeExpr::Void)
            }
        }
    }

    fn infer_named_call(
        &self,
        name: &str,
        arg_types: &[(TypeExpr, Span)],
        span: Span,
        errors: &mut Vec<TypeError>,
    ) -> Option<TypeExpr> {
        if let Some(sig) = self.env.lookup_fn(name) {
            if sig.params.len() != arg_types.len() {
                errors.push(TypeError::TypeMismatch {
                    expected: format!("{} argument(s)", sig.params.len()),
                    got: format!("{} argument(s)", arg_types.len()),
                    span,
                });
            }

            for (param, (arg_ty, arg_span)) in sig.params.iter().zip(arg_types.iter()) {
                self.expect_compatible(&param.ty, arg_ty, *arg_span, errors);
            }

            return Some(sig.return_type.clone());
        }

        match name {
            "print" | "println" => Some(TypeExpr::Void),
            "to_text" | "text_concat" | "int_to_text" | "sha256" => {
                Some(TypeExpr::Named("Text".to_string()))
            }
            "text_to_int" | "text_len" | "now_unix" | "list_len" => {
                Some(TypeExpr::Named("Int".to_string()))
            }
            // Result-returning I/O builtins — return type depends on context
            "fs_read" | "fs_write" | "fs_delete" | "http_get" | "http_post"
            | "db_open" | "db_execute" | "db_query" | "json_stringify"
            | "json_parse" | "env_get" | "db_insert" | "db_find" | "db_find_one"
            | "db_update" | "db_delete" | "db_count" | "db_create_table" => {
                // Result[Unknown, Text] — leave inner unknown so it's compatible
                Some(TypeExpr::Named(UNKNOWN_TYPE.to_string()))
            }
            "fs_exists" => Some(TypeExpr::Named("Bool".to_string())),
            "http_serve_nous" | "http_serve_static" => Some(TypeExpr::Void),
            // Nous self-verification: returns a Record but we don't have a
            // named type for it, so treat as unknown.
            "nous_verify" => Some(TypeExpr::Named(UNKNOWN_TYPE.to_string())),
            // Unknown call — don't error cascade; treat as unknown so field
            // access and argument passing work. This is the sane default for
            // a language where builtins are plentiful.
            _ => Some(TypeExpr::Named(UNKNOWN_TYPE.to_string())),
        }
    }

    fn bind_pattern(
        &self,
        pattern: &Pattern,
        scrutinee_ty: &TypeExpr,
        span: Span,
        scope: &mut LocalScope,
        errors: &mut Vec<TypeError>,
    ) {
        match pattern {
            Pattern::Wildcard => {}
            Pattern::Ident(name) => {
                scope.insert(name.clone(), scrutinee_ty.clone());
            }
            Pattern::Literal(literal) => {
                self.check_expr_against(literal, span, scrutinee_ty, scope, errors);
            }
            Pattern::Tuple(patterns) => {
                let TypeExpr::Tuple(types) = self.normalize_type(scrutinee_ty) else {
                    return;
                };
                for (pattern, ty) in patterns.iter().zip(types.iter()) {
                    self.bind_pattern(&pattern.node, &ty.node, pattern.span, scope, errors);
                }
            }
            Pattern::Constructor { name, fields } => {
                if let Some((ok_ty, err_ty)) = self.unwrap_result_signature(scrutinee_ty) {
                    let inner_ty = match name.as_str() {
                        "Ok" => ok_ty,
                        "Err" => err_ty,
                        _ => return,
                    };

                    if let Some(field_pattern) = fields.first() {
                        self.bind_pattern(
                            &field_pattern.node,
                            &inner_ty,
                            field_pattern.span,
                            scope,
                            errors,
                        );
                    }
                }
            }
        }
    }

    fn unwrap_result_signature(&self, ty: &TypeExpr) -> Option<(TypeExpr, TypeExpr)> {
        match self.normalize_type(ty) {
            TypeExpr::Generic { name, args } if name == "Result" && args.len() == 2 => {
                Some((args[0].node.clone(), args[1].node.clone()))
            }
            _ => None,
        }
    }

    fn unwrap_result_expr_type(&self, ty: &TypeExpr) -> (TypeExpr, Option<TypeExpr>) {
        match self.unwrap_result_signature(ty) {
            Some((ok_ty, err_ty)) => (ok_ty, Some(err_ty)),
            None => (ty.clone(), None),
        }
    }

    fn expect_compatible(
        &self,
        expected: &TypeExpr,
        got: &TypeExpr,
        span: Span,
        errors: &mut Vec<TypeError>,
    ) {
        if !self.types_compatible(expected, got) {
            errors.push(TypeError::TypeMismatch {
                expected: self.type_to_string(expected),
                got: self.type_to_string(got),
                span,
            });
        }
    }

    fn types_compatible(&self, left: &TypeExpr, right: &TypeExpr) -> bool {
        self.merge_types(left, right).is_some()
    }

    fn merge_types(&self, left: &TypeExpr, right: &TypeExpr) -> Option<TypeExpr> {
        let left = self.normalize_type(left);
        let right = self.normalize_type(right);

        match (&left, &right) {
            (TypeExpr::Named(name), other) | (other, TypeExpr::Named(name))
                if name == UNKNOWN_TYPE =>
            {
                Some(other.clone())
            }
            (TypeExpr::Void, TypeExpr::Void) => Some(TypeExpr::Void),
            (TypeExpr::Named(left_name), TypeExpr::Named(right_name))
                if left_name == right_name =>
            {
                Some(left.clone())
            }
            (TypeExpr::Named(left_name), TypeExpr::Named(right_name))
                if self.is_numeric_name(left_name) && self.is_numeric_name(right_name) =>
            {
                Some(TypeExpr::Named(
                    if left_name == "Dec" || right_name == "Dec" {
                        "Dec"
                    } else if left_name == "Int" || right_name == "Int" {
                        "Int"
                    } else {
                        "Nat"
                    }
                    .to_string(),
                ))
            }
            (
                TypeExpr::Generic {
                    name: left_name,
                    args: left_args,
                },
                TypeExpr::Generic {
                    name: right_name,
                    args: right_args,
                },
            ) if left_name == right_name && left_args.len() == right_args.len() => {
                let mut merged_args = Vec::with_capacity(left_args.len());
                for (left_arg, right_arg) in left_args.iter().zip(right_args.iter()) {
                    let merged = self.merge_types(&left_arg.node, &right_arg.node)?;
                    merged_args.push(Spanned::dummy(merged));
                }
                Some(TypeExpr::Generic {
                    name: left_name.clone(),
                    args: merged_args,
                })
            }
            (TypeExpr::Tuple(left_elems), TypeExpr::Tuple(right_elems))
                if left_elems.len() == right_elems.len() =>
            {
                let mut merged = Vec::with_capacity(left_elems.len());
                for (left_elem, right_elem) in left_elems.iter().zip(right_elems.iter()) {
                    merged.push(Spanned::dummy(
                        self.merge_types(&left_elem.node, &right_elem.node)?,
                    ));
                }
                Some(TypeExpr::Tuple(merged))
            }
            (TypeExpr::Union(variants), other) | (other, TypeExpr::Union(variants)) => variants
                .iter()
                .find_map(|variant| self.merge_types(&variant.node, other)),
            _ => None,
        }
    }

    fn normalize_type(&self, ty: &TypeExpr) -> TypeExpr {
        self.normalize_type_inner(ty, &mut HashSet::new())
    }

    fn normalize_type_inner(&self, ty: &TypeExpr, seen_aliases: &mut HashSet<String>) -> TypeExpr {
        match ty {
            TypeExpr::Named(name) if name == UNKNOWN_TYPE => TypeExpr::Named(name.clone()),
            TypeExpr::Named(name) => {
                if seen_aliases.insert(name.clone()) {
                    if let Some(alias) = self.env.lookup_type_alias(name) {
                        let normalized = self.normalize_type_inner(alias, seen_aliases);
                        seen_aliases.remove(name);
                        normalized
                    } else {
                        TypeExpr::Named(name.clone())
                    }
                } else {
                    TypeExpr::Named(name.clone())
                }
            }
            TypeExpr::Generic { name, args } => TypeExpr::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| Spanned::dummy(self.normalize_type_inner(&arg.node, seen_aliases)))
                    .collect(),
            },
            TypeExpr::Union(variants) => TypeExpr::Union(
                variants
                    .iter()
                    .map(|variant| {
                        Spanned::dummy(self.normalize_type_inner(&variant.node, seen_aliases))
                    })
                    .collect(),
            ),
            TypeExpr::Refined { base, .. } => self.normalize_type_inner(&base.node, seen_aliases),
            TypeExpr::Tuple(elements) => TypeExpr::Tuple(
                elements
                    .iter()
                    .map(|elem| Spanned::dummy(self.normalize_type_inner(&elem.node, seen_aliases)))
                    .collect(),
            ),
            TypeExpr::Void => TypeExpr::Void,
        }
    }

    fn is_numeric_type(&self, ty: &TypeExpr) -> bool {
        matches!(
            self.normalize_type(ty),
            TypeExpr::Named(ref name) if name == "Int" || name == "Nat" || name == "Dec"
        )
    }

    fn is_numeric_name(&self, name: &str) -> bool {
        name == "Int" || name == "Nat" || name == "Dec"
    }

    fn type_to_string(&self, ty: &TypeExpr) -> String {
        match ty {
            TypeExpr::Named(name) => name.clone(),
            TypeExpr::Generic { name, args } => format!(
                "{name}[{}]",
                args.iter()
                    .map(|arg| self.type_to_string(&arg.node))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeExpr::Union(variants) => variants
                .iter()
                .map(|variant| self.type_to_string(&variant.node))
                .collect::<Vec<_>>()
                .join(" | "),
            TypeExpr::Refined { base, .. } => self.type_to_string(&base.node),
            TypeExpr::Tuple(elements) => format!(
                "({})",
                elements
                    .iter()
                    .map(|elem| self.type_to_string(&elem.node))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            TypeExpr::Void => "Void".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // Type expression validation helper
    // -----------------------------------------------------------------------

    /// Recursively validate a type expression, emitting `UnknownType` for
    /// any name that is not in scope.
    fn check_type_expr(&self, ty: &TypeExpr, span: Span, errors: &mut Vec<TypeError>) {
        match ty {
            TypeExpr::Named(name) => {
                if !self.env.lookup_type(name) {
                    errors.push(TypeError::UnknownType {
                        name: name.clone(),
                        span,
                    });
                }
            }

            TypeExpr::Generic { name, args } => {
                // Validate the outer generic name (e.g. "List", "Result").
                if !self.env.lookup_type(name) {
                    errors.push(TypeError::UnknownType {
                        name: name.clone(),
                        span,
                    });
                }
                // Recursively validate each type argument.
                for arg in args {
                    self.check_type_expr(&arg.node, arg.span, errors);
                }
            }

            TypeExpr::Union(variants) => {
                for variant in variants {
                    self.check_type_expr(&variant.node, variant.span, errors);
                }
            }

            TypeExpr::Refined { base, constraint } => {
                self.check_type_expr(&base.node, base.span, errors);
                let mut scope = LocalScope::new();
                scope.insert("self".to_string(), base.node.clone());
                self.check_constraint_expr(&constraint.node, constraint.span, &scope, errors);
            }

            TypeExpr::Tuple(elements) => {
                for elem in elements {
                    self.check_type_expr(&elem.node, elem.span, errors);
                }
            }

            TypeExpr::Void => {
                // Always valid; no further checking needed.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Effect tracking helpers
// ---------------------------------------------------------------------------

/// Walk an expression tree and collect the names of all directly-called
/// functions (i.e. `Call { func: Ident(name), .. }` patterns).
///
/// This is intentionally shallow: it only recognises named calls, not
/// higher-order function applications through variables or lambdas.
fn collect_calls_in_expr(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Call { func, args } => {
            if let Expr::Ident(name) = &func.node {
                out.insert(name.clone());
            }
            // Recurse into the callee and each argument.
            collect_calls_in_expr(&func.node, out);
            for arg in args {
                collect_calls_in_expr(&arg.node, out);
            }
        }
        Expr::Pipe { value, func, args } => {
            if let Expr::Ident(name) = &func.node {
                out.insert(name.clone());
            }
            collect_calls_in_expr(&value.node, out);
            collect_calls_in_expr(&func.node, out);
            for arg in args {
                collect_calls_in_expr(&arg.node, out);
            }
        }
        Expr::BinOp { left, right, .. } => {
            collect_calls_in_expr(&left.node, out);
            collect_calls_in_expr(&right.node, out);
        }
        Expr::UnaryOp { operand, .. } => {
            collect_calls_in_expr(&operand.node, out);
        }
        Expr::Let { value, .. } => {
            collect_calls_in_expr(&value.node, out);
        }
        Expr::Block(stmts) => {
            for s in stmts {
                collect_calls_in_expr(&s.node, out);
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(&condition.node, out);
            collect_calls_in_expr(&then_branch.node, out);
            if let Some(eb) = else_branch {
                collect_calls_in_expr(&eb.node, out);
            }
        }
        Expr::Match { scrutinee, arms } => {
            collect_calls_in_expr(&scrutinee.node, out);
            for arm in arms {
                collect_calls_in_expr(&arm.body.node, out);
            }
        }
        Expr::Record { fields, .. } => {
            for (_, v) in fields {
                collect_calls_in_expr(&v.node, out);
            }
        }
        Expr::RecordUpdate { base, updates } => {
            collect_calls_in_expr(&base.node, out);
            for (_, v) in updates {
                collect_calls_in_expr(&v.node, out);
            }
        }
        Expr::Tuple(elems) | Expr::List(elems) => {
            for e in elems {
                collect_calls_in_expr(&e.node, out);
            }
        }
        Expr::Lambda { body, .. } => {
            collect_calls_in_expr(&body.node, out);
        }
        Expr::Return(inner)
        | Expr::Ok(inner)
        | Expr::Err(inner)
        | Expr::Try(inner)
        | Expr::Primed(inner)
        | Expr::Pre(inner)
        | Expr::Transaction(inner) => {
            collect_calls_in_expr(&inner.node, out);
        }
        Expr::FieldAccess { object, .. } => {
            collect_calls_in_expr(&object.node, out);
        }
        Expr::Require {
            condition,
            else_expr,
        } => {
            collect_calls_in_expr(&condition.node, out);
            if let Some(e) = else_expr {
                collect_calls_in_expr(&e.node, out);
            }
        }
        // Leaves: no sub-expressions to recurse into.
        Expr::IntLit(_)
        | Expr::DecLit(_)
        | Expr::StringLit(_)
        | Expr::BoolLit(_)
        | Expr::Ident(_)
        | Expr::SelfRef
        | Expr::Void => {}
    }
}

// ---------------------------------------------------------------------------
// Reachability helpers
// ---------------------------------------------------------------------------

/// Compute the set of states reachable from `initial` by following transition
/// edges in `transitions`.  The initial state itself is always included.
fn reachable_states<'a>(initial: &'a str, transitions: &'a [TransitionDef]) -> HashSet<&'a str> {
    // Build an adjacency list keyed by `from` state.
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for t in transitions {
        adj.entry(t.from.as_str()).or_default().push(t.to.as_str());
    }

    // BFS from the initial state.
    let mut visited: HashSet<&str> = HashSet::new();
    let mut queue: std::collections::VecDeque<&str> = std::collections::VecDeque::new();

    visited.insert(initial);
    queue.push_back(initial);

    while let Some(current) = queue.pop_front() {
        if let Some(neighbours) = adj.get(current) {
            for &next in neighbours {
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }

    visited
}
