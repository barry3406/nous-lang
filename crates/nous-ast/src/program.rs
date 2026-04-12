use serde::{Deserialize, Serialize};

use crate::decl::Decl;
use crate::span::Spanned;

/// A complete Nous program (one source file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub declarations: Vec<Spanned<Decl>>,
}
