pub mod types;
pub mod values;
pub mod instructions;

pub use types::Type;
pub use values::{Value, ResultLocalVar};
pub use instructions::{Instr, Block, Function, GlobalVar, Module};