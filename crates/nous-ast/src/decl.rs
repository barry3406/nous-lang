use serde::{Deserialize, Serialize};

use crate::expr::{Expr, Param};
use crate::span::Spanned;
use crate::types::{Field, TypeDecl, TypeExpr};

/// Top-level declarations in a Nous program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Decl {
    Namespace(NamespaceDecl),
    Use(UseDecl),
    Type(TypeDecl),
    Entity(EntityDecl),
    Enum(EnumDecl),
    State(StateDecl),
    Effect(EffectDecl),
    Capability(CapabilityDecl),
    Fn(FnDecl),
    Flow(FlowDecl),
    Endpoint(EndpointDecl),
    Handler(HandlerDecl),
    Main(MainDecl),
}

/// `ns banking.transfer`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceDecl {
    pub path: Vec<String>,
}

/// `use banking.types.*`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub wildcard: bool,
}

/// Entity declaration with fields and invariants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDecl {
    pub name: String,
    pub fields: Vec<Spanned<Field>>,
    pub invariants: Vec<Spanned<Expr>>,
}

/// Enum declaration with variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Field>,
}

/// State machine declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDecl {
    pub name: String,
    pub transitions: Vec<Transition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub from: String,
    pub action: String,
    pub params: Vec<Param>,
    pub to: String,
    /// Source span of this transition (for error reporting).
    #[serde(default = "crate::span::Span::dummy")]
    pub span: crate::span::Span,
}

/// Effect declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectDecl {
    pub name: String,
}

/// Trust level — how confident are we that a claim is true?
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    /// Mathematically proven by Z3 SMT solver.
    Proved,
    /// Type-checked and runtime-enforced, but not formally proven.
    Checked,
    /// Based on observed external system behavior. May change.
    Observed,
    /// Explicitly assumed. No verification. Use at your own risk.
    Assumed,
}

impl Default for TrustLevel {
    fn default() -> Self { TrustLevel::Checked }
}

/// An obligation — a risk the compiler identified but the programmer hasn't resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Obligation {
    pub name: String,
    pub description: Option<String>,
}

/// Contract clauses shared by fn and flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub requires: Vec<RequireClause>,
    pub ensures: Vec<EnsureClause>,
    pub effects: Vec<String>,
    pub trust: TrustLevel,
    pub obligations: Vec<Obligation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequireClause {
    pub condition: Spanned<Expr>,
    pub else_expr: Option<Spanned<Expr>>,
    pub trust: TrustLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsureClause {
    pub condition: Spanned<Expr>,
    pub trust: TrustLevel,
}

/// Capability declaration — an external effect with semantic properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Spanned<TypeExpr>,
    pub idempotent_by: Option<String>,
    pub timeout: Option<i64>,
    pub compensate: Option<Spanned<Expr>>,
    pub retry: Option<i64>,
    pub confirm_by: Option<String>,
    pub trust: TrustLevel,
}

/// Function declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Spanned<TypeExpr>,
    pub contract: Contract,
    pub body: Spanned<Expr>,
}

/// Flow declaration with steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Spanned<TypeExpr>,
    pub contract: Contract,
    pub steps: Vec<FlowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub name: String,
    pub body: Spanned<Expr>,
    pub rollback: Spanned<Expr>,
}

/// Endpoint declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointDecl {
    pub method: HttpMethod,
    pub path: String,
    pub input_fields: Vec<Spanned<Field>>,
    pub output_mappings: Vec<OutputMapping>,
    pub handler: Spanned<Expr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputMapping {
    pub status: u16,
    pub ty: Spanned<TypeExpr>,
}

/// Handler declaration binding effects to implementations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDecl {
    pub name: String,
    pub bindings: Vec<HandlerBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerBinding {
    pub effect: String,
    pub implementation: Spanned<Expr>,
}

/// Main entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainDecl {
    pub handlers: Vec<String>,
    pub body: Spanned<Expr>,
}
