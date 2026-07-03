use crate::function::IntoHostFunction;
use crate::{Error, Profile, Result, Rustia, VmSettings};
use rustia_vm::{HostFunctionOp, OpCapability};

pub struct RustiaBuilder {
    profile: Profile,
    settings: VmSettings,
    extensions: Vec<RustiaExtension>,
}

impl RustiaBuilder {
    pub(crate) fn new() -> Self {
        Self {
            profile: Profile::ReproducibleStandard,
            settings: VmSettings::default(),
            extensions: Vec::new(),
        }
    }

    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        self
    }

    pub fn settings(mut self, settings: VmSettings) -> Self {
        self.settings = settings;
        self
    }

    pub fn fuel(mut self, fuel: u64) -> Self {
        self.settings.initial_fuel = Some(fuel);
        self
    }

    pub fn extension(mut self, extension: RustiaExtension) -> Self {
        self.extensions.push(extension);
        self
    }

    pub fn build(self) -> Result<Rustia> {
        let mut rustia = Rustia::with_settings(self.profile, self.settings);
        for extension in self.extensions {
            rustia.install_extension(extension)?;
        }
        Ok(rustia)
    }
}

#[derive(Clone, Debug)]
pub struct RustiaExtension {
    namespace: Option<String>,
    functions: Vec<HostFunctionOp>,
}

/// Version for the public host-function fuel-cost schedule.
///
/// Bump this when a rustia release changes the meaning of declared host fuel costs.
pub const RUSTIA_HOST_FUEL_ABI_VERSION: u32 = 1;

/// Default fuel cost assigned by capability-specific host-function helpers.
pub const RUSTIA_DEFAULT_HOST_FUNCTION_FUEL: u32 = 1;

/// Minimum accepted host-function fuel cost.
pub const RUSTIA_MIN_HOST_FUNCTION_FUEL: u32 = 1;

/// Public host-function fuel policy for extension metadata.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaHostFuelPolicy {
    /// ABI version for interpreting host-function fuel costs.
    pub abi_version: u32,
    /// Fuel cost used when the helper API does not take an explicit cost.
    pub default_function_fuel: u32,
    /// Smallest fuel cost accepted by VM registration.
    pub minimum_function_fuel: u32,
}

pub const RUSTIA_HOST_FUEL_POLICY: RustiaHostFuelPolicy = RustiaHostFuelPolicy {
    abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
    default_function_fuel: RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
    minimum_function_fuel: RUSTIA_MIN_HOST_FUNCTION_FUEL,
};

/// Public pre-install metadata for a registered host function.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RustiaHostFunctionInfo<'a> {
    pub name: &'a str,
    /// ABI version used to interpret `fuel_cost`.
    pub fuel_abi_version: u32,
    pub fuel_cost: u32,
    pub capability: OpCapability,
    pub parameter_count: u8,
    pub parameter_types: &'a [&'static str],
    pub return_types: &'a [&'static str],
    pub fallible: bool,
}

impl RustiaExtension {
    pub fn new(namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        Self {
            namespace: (!namespace.is_empty()).then_some(namespace),
            functions: Vec::new(),
        }
    }

    pub fn global() -> Self {
        Self {
            namespace: None,
            functions: Vec::new(),
        }
    }

    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub fn fuel_policy(&self) -> RustiaHostFuelPolicy {
        RUSTIA_HOST_FUEL_POLICY
    }

    pub fn functions(&self) -> impl Iterator<Item = RustiaHostFunctionInfo<'_>> + '_ {
        self.functions
            .iter()
            .map(|function| RustiaHostFunctionInfo {
                name: &function.name,
                fuel_abi_version: RUSTIA_HOST_FUEL_ABI_VERSION,
                fuel_cost: function.fuel_cost,
                capability: function.capability,
                parameter_count: function.parameter_count,
                parameter_types: &function.parameter_types,
                return_types: &function.return_types,
                fallible: function.fallible,
            })
    }

    pub fn fn_pure<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::Pure,
            function,
        )
    }

    pub fn fn_input<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsDeterministicInput,
            function,
        )
    }

    pub fn fn_slot_read<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsSlotState,
            function,
        )
    }

    pub fn fn_slot_write<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::WritesSlotEffect,
            function,
        )
    }

    pub fn fn_effect<Func, Args, Return>(self, name: impl Into<String>, function: Func) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::EmitsDeterministicEffect,
            function,
        )
    }

    pub fn fn_external_state<Func, Args, Return>(
        self,
        name: impl Into<String>,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::ReadsExternalMutableState,
            function,
        )
    }

    pub fn fn_nondeterministic<Func, Args, Return>(
        self,
        name: impl Into<String>,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.fn_with(
            name,
            RUSTIA_DEFAULT_HOST_FUNCTION_FUEL,
            OpCapability::NondeterministicEffect,
            function,
        )
    }

    pub fn fn_with<Func, Args, Return>(
        mut self,
        name: impl Into<String>,
        fuel_cost: u32,
        capability: OpCapability,
        function: Func,
    ) -> Self
    where
        Func: IntoHostFunction<Args, Return>,
    {
        self.functions
            .push(function.into_host_function(name.into(), fuel_cost, capability));
        self
    }

    pub(crate) fn install(self, rustia: &mut Rustia) -> Result<()> {
        let Some(namespace) = self.namespace else {
            for function in self.functions {
                rustia
                    .vm_mut()
                    .install_host_function(function)
                    .map_err(Error::Vm)?;
            }
            return Ok(());
        };

        let table = rustia.vm_mut().empty_table();
        for function in self.functions {
            let name = function.name.clone();
            let value = rustia
                .vm_mut()
                .register_host_function(function)
                .map_err(Error::Vm)?;
            let key = rustia.vm_mut().string(&name);
            rustia
                .vm_mut()
                .raw_table_set_value(table, key, value)
                .map_err(Error::Vm)?;
        }
        rustia
            .vm_mut()
            .set_global_value(namespace, table)
            .map_err(Error::Vm)
    }
}
