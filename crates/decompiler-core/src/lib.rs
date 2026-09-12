#![forbid(unsafe_code)]
pub mod analysis;
pub mod assembly;
pub mod ast;
pub mod cfg;
pub mod cil;
pub mod decompile;
pub mod emit;
pub mod error;
pub mod inspection;
pub mod ir;
pub mod metadata;
mod opcodes;
pub mod pe;
pub mod reader;
pub mod session;
pub mod signature;
pub mod transform;
