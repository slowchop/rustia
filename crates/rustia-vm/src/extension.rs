use crate::error::{Error, ErrorKind, Result};
use crate::profile::Profile;
use crate::value::Value;
use crate::vm::Vm;
use std::fmt;
use std::mem::size_of;
use std::sync::Arc;

pub type ExtensionHandler = fn(&[Value]) -> Result<Value>;
pub type HostFunctionHandler =
    Arc<dyn Fn(&mut Vm, &[Value]) -> Result<Vec<Value>> + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) enum HostFunctionTarget {
    Inline(HostFunctionHandler),
    Dispatcher(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpCapability {
    Pure,
    ReadsDeterministicInput,
    ReadsSlotState,
    EmitsDeterministicEffect,
    WritesSlotEffect,
    ReadsExternalMutableState,
    NondeterministicEffect,
}

#[derive(Clone)]
pub struct ExtensionOp {
    pub name: String,
    pub fuel_cost: u32,
    pub capability: OpCapability,
    pub handler: ExtensionHandler,
}

impl ExtensionOp {
    pub fn new(
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        handler: ExtensionHandler,
    ) -> Self {
        Self {
            name: name.into(),
            fuel_cost,
            capability,
            handler,
        }
    }

    #[inline]
    pub const fn is_allowed_in(self_capability: OpCapability, profile: Profile) -> bool {
        match profile {
            Profile::DeterministicSim => matches!(
                self_capability,
                OpCapability::Pure
                    | OpCapability::ReadsDeterministicInput
                    | OpCapability::ReadsSlotState
                    | OpCapability::EmitsDeterministicEffect
                    | OpCapability::WritesSlotEffect
            ),
            Profile::ReproducibleStandard | Profile::LuauCompat | Profile::Puc51Compat => true,
        }
    }
}

impl fmt::Debug for ExtensionOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtensionOp")
            .field("name", &self.name)
            .field("fuel_cost", &self.fuel_cost)
            .field("capability", &self.capability)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct HostFunctionOp {
    pub name: String,
    pub fuel_cost: u32,
    pub capability: OpCapability,
    pub parameter_count: u8,
    pub parameter_types: Vec<&'static str>,
    pub return_types: Vec<&'static str>,
    pub fallible: bool,
    pub(crate) target: HostFunctionTarget,
}

impl HostFunctionOp {
    pub fn new(
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        handler: HostFunctionHandler,
    ) -> Self {
        Self {
            name: name.into(),
            fuel_cost,
            capability,
            parameter_count: 0,
            parameter_types: Vec::new(),
            return_types: Vec::new(),
            fallible: false,
            target: HostFunctionTarget::Inline(handler),
        }
    }

    /// Creates a host operation whose implementation is supplied by the
    /// short-lived dispatcher passed to a VM execution.
    ///
    /// This is a low-level embedding seam. Most embedders should use the typed
    /// invocation API in the `rustia` crate instead.
    #[doc(hidden)]
    pub fn dispatched(
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        dispatcher_key: u32,
    ) -> Self {
        Self {
            name: name.into(),
            fuel_cost,
            capability,
            parameter_count: 0,
            parameter_types: Vec::new(),
            return_types: Vec::new(),
            fallible: false,
            target: HostFunctionTarget::Dispatcher(dispatcher_key),
        }
    }

    pub fn with_signature(
        mut self,
        parameter_count: u8,
        parameter_types: Vec<&'static str>,
        return_types: Vec<&'static str>,
        fallible: bool,
    ) -> Self {
        debug_assert_eq!(usize::from(parameter_count), parameter_types.len());
        self.parameter_count = parameter_count;
        self.parameter_types = parameter_types;
        self.return_types = return_types;
        self.fallible = fallible;
        self
    }
}

impl fmt::Debug for HostFunctionOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostFunctionOp")
            .field("name", &self.name)
            .field("fuel_cost", &self.fuel_cost)
            .field("capability", &self.capability)
            .field("parameter_count", &self.parameter_count)
            .field("parameter_types", &self.parameter_types)
            .field("return_types", &self.return_types)
            .field("fallible", &self.fallible)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExtensionRegistry {
    ops: Vec<ExtensionOp>,
}

impl ExtensionRegistry {
    pub fn register(&mut self, profile: Profile, op: ExtensionOp) -> Result<u16> {
        validate_registered_op(profile, &op.name, op.fuel_cost, op.capability)?;
        let id = self.ops.len();
        if id > u16::MAX as usize {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "extension op table exhausted",
            }));
        }
        self.ops.push(op);
        Ok(id as u16)
    }

    #[inline]
    pub fn get(&self, id: u16) -> Option<&ExtensionOp> {
        self.ops.get(id as usize)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub(crate) fn memory_usage(&self) -> (usize, usize) {
        let mut live = self.ops.len().saturating_mul(size_of::<ExtensionOp>());
        let mut reserved = self.ops.capacity().saturating_mul(size_of::<ExtensionOp>());
        for op in &self.ops {
            live = live.saturating_add(op.name.len());
            reserved = reserved.saturating_add(op.name.capacity());
        }
        (live, reserved)
    }
}

pub(crate) fn validate_registered_op(
    profile: Profile,
    operation: &str,
    fuel_cost: u32,
    capability: OpCapability,
) -> Result<()> {
    if fuel_cost == 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "host operation fuel cost must be non-zero",
        }));
    }
    if !ExtensionOp::is_allowed_in(capability, profile) {
        let feature = match capability {
            OpCapability::Pure => "pure host function",
            OpCapability::ReadsDeterministicInput => "host function reading invocation input",
            OpCapability::ReadsSlotState => "host function reading persistent slots",
            OpCapability::EmitsDeterministicEffect => {
                "host function recording a deterministic effect"
            }
            OpCapability::WritesSlotEffect => "host function writing persistent slots",
            OpCapability::ReadsExternalMutableState => {
                "host function reading external mutable state"
            }
            OpCapability::NondeterministicEffect => "host function with a nondeterministic effect",
        };
        return Err(Error::new(ErrorKind::CapabilityRemoved {
            feature,
            operation: Some(operation.to_owned()),
            profile,
        }));
    }
    Ok(())
}
