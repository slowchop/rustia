use crate::function::{adapter, HostReturnValue};
use crate::{Error, NewObject, Object, Result, Value};
use std::any::{type_name, TypeId};

impl<T: 'static> adapter::Value for Object<T> {
    fn into_host_value(self, _: adapter::Token) -> Result<HostReturnValue> {
        Ok(HostReturnValue::Live(self.as_value()))
    }
}

#[cfg(not(feature = "transferable"))]
impl<T: 'static> adapter::Value for NewObject<T> {
    fn into_host_value(self, _: adapter::Token) -> Result<HostReturnValue> {
        Ok(HostReturnValue::NewObject {
            rust_type_id: TypeId::of::<T>(),
            rust_type_name: type_name::<T>(),
            value: Box::new(self.value),
        })
    }
}

#[cfg(feature = "transferable")]
impl<T: Send + Sync + 'static> adapter::Value for NewObject<T> {
    fn into_host_value(self, _: adapter::Token) -> Result<HostReturnValue> {
        Ok(HostReturnValue::NewObject {
            rust_type_id: TypeId::of::<T>(),
            rust_type_name: type_name::<T>(),
            value: Box::new(self.value),
        })
    }
}

impl<T: 'static> adapter::Argument for Object<T> {
    fn from_host_argument(
        _: adapter::Token,
        vm: &mut rustia_vm::Vm,
        value: rustia_vm::Value,
    ) -> Result<Self> {
        vm.checked_host_object(value, TypeId::of::<T>(), type_name::<T>())
            .map_err(Error::from_engine)?;
        Ok(Self::new(Value::retain(vm, value)?))
    }
}

impl<T: 'static> adapter::Return for Object<T> {
    fn into_host_return(self, token: adapter::Token) -> Result<Vec<HostReturnValue>> {
        Ok(vec![adapter::Value::into_host_value(self, token)?])
    }
    fn rustia_type_names(_: adapter::Token) -> Vec<&'static str> {
        vec![type_name::<Self>()]
    }
}

#[cfg(not(feature = "transferable"))]
impl<T: 'static> adapter::Return for NewObject<T> {
    fn into_host_return(self, token: adapter::Token) -> Result<Vec<HostReturnValue>> {
        Ok(vec![adapter::Value::into_host_value(self, token)?])
    }
    fn rustia_type_names(_: adapter::Token) -> Vec<&'static str> {
        vec![type_name::<Self>()]
    }
}

#[cfg(feature = "transferable")]
impl<T: Send + Sync + 'static> adapter::Return for NewObject<T> {
    fn into_host_return(self, token: adapter::Token) -> Result<Vec<HostReturnValue>> {
        Ok(vec![adapter::Value::into_host_value(self, token)?])
    }
    fn rustia_type_names(_: adapter::Token) -> Vec<&'static str> {
        vec![type_name::<Self>()]
    }
}
