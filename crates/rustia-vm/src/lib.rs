#![forbid(unsafe_code)]

//! Portable rustia VM core.
//!
//! This crate is the first P1 production slice from `design/30-architecture.md`: a sealed
//! 16-byte value type, deterministic numeric helpers, fixed-width bytecode, fuelled big-match
//! dispatch, profile-aware extension registration, and a `gc-arena` VM cell scaffold.

pub mod bytecode;
pub mod error;
pub mod extension;
mod heap;
pub mod number;
pub mod profile;
mod value;
pub mod vm;

pub use bytecode::{Constant, Function, Instruction, Opcode, Program};
pub use error::{Error, ErrorKind, Result};
pub use extension::{
    ExtensionHandler, ExtensionOp, HostFunctionHandler, HostFunctionOp, OpCapability,
};
pub use profile::Profile;
pub use value::{Value, ValueKind};
pub use vm::{
    LoadStringCompiler, Vm, VmGlobalGetInlineCacheSnapshot, VmInlineCacheSnapshot,
    VmOpcodeProfileEntry, VmOpcodeProfileSnapshot, VmRawTableGetInlineCacheSnapshot, VmSettings,
    VmTableGetInlineCacheSnapshot, VmTableIntegerGetInlineCacheSnapshot,
    VmTableIntegerSetInlineCacheSnapshot, VmTableSetInlineCacheSnapshot, MAX_RECURSION_LIMIT,
};
