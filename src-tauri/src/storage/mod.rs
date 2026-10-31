//! Storage scanning and path safety. The scanner is read-only; deleting lives in the action engine.

pub mod paths;
pub mod safety;
pub mod scanner;
pub mod state;
