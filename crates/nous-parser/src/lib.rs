pub mod builder;
pub mod error;
pub mod indent;
pub mod loader;

use nous_ast::Program;
use pest::Parser;
use pest_derive::Parser;

use crate::builder::build_program;
use crate::error::ParseError;
use crate::indent::preprocess_indentation;

pub use loader::load_program;

#[derive(Parser)]
#[grammar = "nous.pest"]
pub struct NousParser;

/// Parse a single Nous source string into an AST Program.
pub fn parse(source: &str) -> Result<Program, ParseError> {
    let preprocessed = preprocess_indentation(source)?;
    let pairs = NousParser::parse(Rule::program, &preprocessed)
        .map_err(|e| ParseError::Grammar(e.to_string()))?;
    build_program(pairs)
}
