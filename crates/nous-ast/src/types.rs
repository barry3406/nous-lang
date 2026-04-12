use serde::{Deserialize, Serialize};

use crate::expr::Expr;
use crate::span::Spanned;

/// A type expression in Nous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeExpr {
    /// Named type: `Account`, `Int`, `Bool`
    Named(String),

    /// Generic type: `Result[T, E]`, `List[Int]`
    Generic {
        name: String,
        args: Vec<Spanned<TypeExpr>>,
    },

    /// Union type: `Active | Frozen | Closed`
    Union(Vec<Spanned<TypeExpr>>),

    /// Refinement type: `Int where self >= 0`
    Refined {
        base: Box<Spanned<TypeExpr>>,
        constraint: Box<Spanned<Expr>>,
    },

    /// Tuple type: `(Account, Account)`
    Tuple(Vec<Spanned<TypeExpr>>),

    /// The Void type
    Void,
}

/// A field in an entity or enum variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub ty: Spanned<TypeExpr>,
}

/// A type alias declaration: `type Money = Dec(2) where self >= 0`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeDecl {
    pub name: String,
    pub ty: Spanned<TypeExpr>,
}
