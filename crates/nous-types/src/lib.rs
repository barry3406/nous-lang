pub mod checker;
pub mod env;
pub mod error;

pub use checker::TypeChecker;
pub use env::TypeEnv;
pub use error::{ContractViolationKind, TypeError};
