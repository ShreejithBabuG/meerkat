pub mod ast;
pub mod parser;
pub mod interpreter;
pub mod semantic_analysis;
pub mod manager;
pub use manager::Manager;
pub type TestId = (usize, usize);
