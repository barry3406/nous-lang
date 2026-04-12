use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ParseError {
    #[error("Grammar error: {0}")]
    Grammar(String),

    #[error("Indentation error at line {line}: {message}")]
    Indentation { line: usize, message: String },

    #[error("Unexpected token at line {line}, col {col}: expected {expected}, got {got}")]
    UnexpectedToken {
        line: usize,
        col: usize,
        expected: String,
        got: String,
    },

    #[error("Unexpected end of input: {message}")]
    UnexpectedEof { message: String },
}
