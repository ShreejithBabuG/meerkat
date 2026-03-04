pub mod error;
pub mod runtime;

// Compatibility re-exports for old code
pub use runtime::ast;
pub use runtime::semantic_analysis as static_analysis;

pub use error::*;
pub use runtime::TestId;
