use serde::{Deserialize, Serialize};

use crate::span::Spanned;
use crate::types::TypeExpr;

/// Expressions in Nous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    /// Integer literal: `42`
    IntLit(i64),

    /// Decimal literal: `3.14`
    DecLit(String),

    /// String literal: `"hello"`
    StringLit(String),

    /// Boolean literal: `true`, `false`
    BoolLit(bool),

    /// Identifier: `x`, `account`, `self`
    Ident(String),

    /// Field access: `account.balance`
    FieldAccess {
        object: Box<Spanned<Expr>>,
        field: String,
    },

    /// Primed field (postcondition): `from.balance'`
    Primed(Box<Spanned<Expr>>),

    /// Binary operation: `a + b`, `x >= y`, `a == b`
    BinOp {
        op: BinOp,
        left: Box<Spanned<Expr>>,
        right: Box<Spanned<Expr>>,
    },

    /// Unary operation: `not x`, `-y`
    UnaryOp {
        op: UnaryOp,
        operand: Box<Spanned<Expr>>,
    },

    /// Function call: `validate_transfer(from, to, amount)`
    Call {
        func: Box<Spanned<Expr>>,
        args: Vec<Spanned<Expr>>,
    },

    /// Method-style call: `list |> map(f)`
    Pipe {
        value: Box<Spanned<Expr>>,
        func: Box<Spanned<Expr>>,
        args: Vec<Spanned<Expr>>,
    },

    /// Error propagation: `expr?`
    Try(Box<Spanned<Expr>>),

    /// Let binding: `let x = expr`
    Let {
        pattern: Box<Spanned<Pattern>>,
        ty: Option<Spanned<TypeExpr>>,
        value: Box<Spanned<Expr>>,
    },

    /// Block of expressions (last is the value)
    Block(Vec<Spanned<Expr>>),

    /// If expression: `if cond then a else b`
    If {
        condition: Box<Spanned<Expr>>,
        then_branch: Box<Spanned<Expr>>,
        else_branch: Option<Box<Spanned<Expr>>>,
    },

    /// Match expression
    Match {
        scrutinee: Box<Spanned<Expr>>,
        arms: Vec<MatchArm>,
    },

    /// Record construction: `Account(id: "abc", balance: 100)`
    Record {
        name: String,
        fields: Vec<(String, Spanned<Expr>)>,
    },

    /// Record update: `{ account with balance: new_balance }`
    RecordUpdate {
        base: Box<Spanned<Expr>>,
        updates: Vec<(String, Spanned<Expr>)>,
    },

    /// Tuple: `(a, b, c)`
    Tuple(Vec<Spanned<Expr>>),

    /// List literal: `[1, 2, 3]`
    List(Vec<Spanned<Expr>>),

    /// Lambda: `x -> x + 1`
    Lambda {
        params: Vec<Param>,
        body: Box<Spanned<Expr>>,
    },

    /// Return: `return expr`
    Return(Box<Spanned<Expr>>),

    /// Ok/Err constructors
    Ok(Box<Spanned<Expr>>),
    Err(Box<Spanned<Expr>>),

    /// `void` literal
    Void,

    /// `self` keyword in refinement constraints
    SelfRef,

    /// Transaction block
    Transaction(Box<Spanned<Expr>>),

    /// Require with else: `require cond else ErrorVariant`
    Require {
        condition: Box<Spanned<Expr>>,
        else_expr: Option<Box<Spanned<Expr>>>,
    },

    /// `pre.field` for accessing pre-state in ensures
    Pre(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BinOp {
    Add,    // +
    Sub,    // -
    Mul,    // *
    Div,    // /
    Mod,    // %
    Eq,     // ==
    Neq,    // /=
    Lt,     // <
    Lte,    // <=
    Gt,     // >
    Gte,    // >=
    And,    // and
    Or,     // or
    Implies, // implies
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,    // -
    Not,    // not
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pattern {
    /// Wildcard: `_`
    Wildcard,
    /// Variable binding: `x`
    Ident(String),
    /// Constructor pattern: `Ok(value)`, `InsufficientFunds(a, b)`
    Constructor {
        name: String,
        fields: Vec<Spanned<Pattern>>,
    },
    /// Tuple pattern: `(a, b)`
    Tuple(Vec<Spanned<Pattern>>),
    /// Literal pattern
    Literal(Expr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Spanned<Expr>,
}

/// A function parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: Spanned<TypeExpr>,
}
