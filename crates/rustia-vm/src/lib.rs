//! Portable rustia VM core.
//!
//! This crate is the first P1 production slice from `design/30-architecture.md`: a sealed
//! 16-byte value type, deterministic numeric helpers, fixed-width bytecode, fuelled big-match
//! dispatch, profile-aware extension registration, and an owned, generation-checked tracing heap.

pub mod byte_string;
pub mod bytecode;
pub mod diagnostic;
pub mod error;
#[cfg(any(test, feature = "executor-diagnostics"))]
mod executor_census;
pub mod extension;
mod heap;
mod host_object;
mod hot_ir;
pub mod instruction_facts;
pub mod liveness;
pub mod number;
mod ownership;
pub mod profile;
mod register_blocks;
pub mod table_operations;
mod tape;
#[cfg(test)]
#[path = "../tests/support/ownership.rs"]
mod test_ownership;
mod value;
pub mod vm;

pub use bytecode::{
    CallDescriptor, CallDescriptorCall, Constant, Function, Instruction, Opcode, Program,
    ProgramLogicalUnits,
};
pub use error::{Error, ErrorKind, Result};
#[cfg(any(test, feature = "executor-diagnostics"))]
pub use executor_census::ExecutorCensus;
pub use extension::{
    ExtensionHandler, ExtensionOp, HostFunctionHandler, HostFunctionOp, OpCapability,
};
#[cfg(any(test, feature = "table-write-diagnostics"))]
pub use heap::NumericArraySlotWriteCounts;
pub use host_object::{
    HostObjectData, HostObjectHooks, HostObjectMut, HostObjectOwner, HostObjectPayload,
    HostObjectRef, HostObjectTypeDefinition,
};
pub use profile::{GuestSurface, Profile};
pub use value::{Value, ValueKind};
pub use vm::{
    ClockProvider, EntropyProvider, HardenedLeafProgram, LoadStringCompiler, ModuleResolver,
    ResolvedModule, RetainedValue, StdoutSink, Vm, VmExecution, VmHostCall, VmHostDispatcher,
    VmMemoryStats, VmServices, VmSettings, DECISION_RESET_ABI_VERSION,
    HARDENED_LEAF_GUEST_ABI_VERSION, INSTALL_TRANSACTION_ABI_VERSION, MAX_RECURSION_LIMIT,
    MODULE_RESOLUTION_ABI_VERSION,
};
