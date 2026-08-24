//! Portable rustia VM core.
//!
//! This crate is the first P1 production slice from `design/30-architecture.md`: a sealed
//! 16-byte value type, deterministic numeric helpers, fixed-width bytecode, fuelled big-match
//! dispatch, profile-aware extension registration, and a `gc-arena` VM cell scaffold.

pub mod bytecode;
pub mod error;
pub mod extension;
mod heap;
mod hot_ir;
pub mod instruction_facts;
pub mod liveness;
pub mod number;
pub mod profile;
mod register_blocks;
mod tape;
mod value;
pub mod vm;

pub use bytecode::{Constant, Function, Instruction, Opcode, Program};
pub use error::{Error, ErrorKind, Result};
pub use extension::{
    ExtensionHandler, ExtensionOp, HostFunctionHandler, HostFunctionOp, OpCapability,
};
pub use profile::{GuestSurface, Profile};
pub use value::{Value, ValueKind};
pub use vm::{
    ClockProvider, EntropyProvider, HardenedLeafProgram, LoadStringCompiler, ModuleResolver,
    ResolvedModule, RetainedValue, StdoutSink, Vm, VmExecution, VmMemoryStats, VmServices,
    VmSettings, DECISION_RESET_ABI_VERSION, HARDENED_LEAF_GUEST_ABI_VERSION, MAX_RECURSION_LIMIT,
    MODULE_RESOLUTION_ABI_VERSION,
};
