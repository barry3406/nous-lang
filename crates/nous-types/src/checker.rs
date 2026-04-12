use std::collections::{HashMap, HashSet};

use nous_ast::decl::{Decl, EnumDecl, EntityDecl, FnDecl, FlowDecl, StateDecl};
use nous_ast::types::TypeExpr;
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
                // TODO: check EndpointDecl — validate field types and handler type
                // TODO: check HandlerDecl — validate each binding's effect exists
                // TODO: check EffectDecl — register effects for UndeclaredEffect checks
                // TODO: check TypeDecl — validate the aliased type expression
                // Namespace, Use, Main: no type-level checks needed yet.
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
                // Functions / flows are registered during their full check so
                // that param types are validated first.
                _ => {}
            }
        }
    }

    // -----------------------------------------------------------------------
    // Entity checking
    // -----------------------------------------------------------------------

    fn check_entity(
        &mut self,
        decl: &EntityDecl,
        span: Span,
        errors: &mut Vec<TypeError>,
    ) {
        let mut field_defs: Vec<FieldDef> = Vec::new();

        for spanned_field in &decl.fields {
            let field = &spanned_field.node;
            let field_span = spanned_field.span;

            // Validate the field's type expression.
            self.check_type_expr(&field.ty.node, field_span, errors);

            field_defs.push(FieldDef {
                name: field.name.clone(),
                ty: field.ty.node.clone(),
            });
        }

        // TODO: check invariant expressions (requires expression type checking)
        let _ = span; // will be used when invariants are checked

        let def = EntityDef {
            name: decl.name.clone(),
            fields: field_defs,
        };
        self.env.define_entity(def);
    }

    // -----------------------------------------------------------------------
    // State machine checking
    // -----------------------------------------------------------------------

    fn check_state(
        &mut self,
        decl: &StateDecl,
        _span: Span,
        errors: &mut Vec<TypeError>,
    ) {
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

    fn check_fn(
        &mut self,
        decl: &FnDecl,
        _span: Span,
        errors: &mut Vec<TypeError>,
    ) {
        // Validate parameter types.
        let mut param_defs: Vec<ParamDef> = Vec::new();
        for param in &decl.params {
            self.check_type_expr(&param.ty.node, param.ty.span, errors);
            param_defs.push(ParamDef {
                name: param.name.clone(),
                ty: param.ty.node.clone(),
            });
        }

        // Validate the return type.
        self.check_type_expr(&decl.return_type.node, decl.return_type.span, errors);

        // TODO: type-check the body expression against return_type
        // TODO: check that each effect used in the body appears in contract.effects
        //       (emit UndeclaredEffect for violations)
        // TODO: statically verify require/ensures clauses where possible
        //       (emit ContractViolation for violations)

        let sig = FnSig {
            name: decl.name.clone(),
            params: param_defs,
            return_type: decl.return_type.node.clone(),
            effects: decl.contract.effects.clone(),
        };
        self.env.define_fn(sig);
    }

    // -----------------------------------------------------------------------
    // Flow checking
    // -----------------------------------------------------------------------

    fn check_flow(
        &mut self,
        decl: &FlowDecl,
        _span: Span,
        errors: &mut Vec<TypeError>,
    ) {
        // Validate parameter types.
        let mut param_defs: Vec<ParamDef> = Vec::new();
        for param in &decl.params {
            self.check_type_expr(&param.ty.node, param.ty.span, errors);
            param_defs.push(ParamDef {
                name: param.name.clone(),
                ty: param.ty.node.clone(),
            });
        }

        // Validate the return type.
        self.check_type_expr(&decl.return_type.node, decl.return_type.span, errors);

        // TODO: type-check each step's body and rollback expression
        // TODO: check that effects in steps match the contract

        let sig = FnSig {
            name: decl.name.clone(),
            params: param_defs,
            return_type: decl.return_type.node.clone(),
            effects: decl.contract.effects.clone(),
        };
        self.env.define_fn(sig);
    }

    // -----------------------------------------------------------------------
    // Enum checking
    // -----------------------------------------------------------------------

    fn check_enum(
        &mut self,
        decl: &EnumDecl,
        _span: Span,
        errors: &mut Vec<TypeError>,
    ) {
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
    // Type expression validation helper
    // -----------------------------------------------------------------------

    /// Recursively validate a type expression, emitting `UnknownType` for
    /// any name that is not in scope.
    fn check_type_expr(
        &self,
        ty: &TypeExpr,
        span: Span,
        errors: &mut Vec<TypeError>,
    ) {
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

            TypeExpr::Refined { base, .. } => {
                // TODO: also type-check the constraint expression
                self.check_type_expr(&base.node, base.span, errors);
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
