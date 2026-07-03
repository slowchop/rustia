use crate::error::{Error, ErrorKind, Result};
use crate::profile::Profile;
use crate::value::Value;
use crate::vm::Vm;
use std::fmt;
use std::sync::Arc;

pub type ExtensionHandler = fn(&[Value]) -> Result<Value>;
pub type HostFunctionHandler =
    Arc<dyn Fn(&mut Vm, &[Value]) -> Result<Vec<Value>> + Send + Sync + 'static>;

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
            Profile::ReproducibleStandard | Profile::LuauCompat => true,
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
    pub handler: HostFunctionHandler,
}

impl HostFunctionOp {
    pub fn new(
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        parameter_count: u8,
        handler: HostFunctionHandler,
    ) -> Self {
        Self {
            name: name.into(),
            fuel_cost,
            capability,
            parameter_count,
            handler,
        }
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
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExtensionRegistry {
    ops: Vec<ExtensionOp>,
}

impl ExtensionRegistry {
    pub fn register(&mut self, profile: Profile, op: ExtensionOp) -> Result<u16> {
        validate_registered_op(profile, op.fuel_cost, op.capability)?;
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
}

pub(crate) fn validate_registered_op(
    profile: Profile,
    fuel_cost: u32,
    capability: OpCapability,
) -> Result<()> {
    if fuel_cost == 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "extension op fuel cost must be non-zero",
        }));
    }
    if !ExtensionOp::is_allowed_in(capability, profile) {
        return Err(Error::new(ErrorKind::CapabilityRemoved {
            feature: "non-deterministic extension op",
            profile,
        }));
    }
    Ok(())
}
