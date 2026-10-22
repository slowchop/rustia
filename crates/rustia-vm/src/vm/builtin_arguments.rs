//! Luau 0.709 luaL_check* diagnostics. Presence and type checks stay separate:
//! luaL_checkany accepts nil, and each builtin chooses its validation order.
use super::*;

impl Vm {
    pub(super) fn builtin_required_arg(&self, args: &[Value], index: usize) -> Result<Value> {
        args.get(index).copied().ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: format!("missing argument #{}", index + 1),
            })
        })
    }

    pub(super) fn builtin_argument_error(
        &self,
        value: Option<Value>,
        index: usize,
        function: &str,
        expected: &str,
    ) -> Error {
        let message = match value {
            None => format!(
                "missing argument #{} to '{function}' ({expected} expected)",
                index + 1
            ),
            Some(value) => format!(
                "invalid argument #{} to '{function}' ({expected} expected, got {})",
                index + 1,
                self.luau_type_name(value)
            ),
        };
        Error::new(ErrorKind::RuntimeError { message })
    }

    pub(super) fn builtin_table_arg(&self, args: &[Value], function: &str) -> Result<u32> {
        let value = args.first().copied();
        value
            .and_then(Value::as_table)
            .ok_or_else(|| self.builtin_argument_error(value, 0, function, "table"))
    }

    pub(super) fn builtin_string_arg(
        &mut self,
        value: Option<Value>,
        index: usize,
        function: &str,
    ) -> Result<Vec<u8>> {
        match value {
            Some(value) if matches!(value.kind(), ValueKind::String | ValueKind::Number) => {
                self.value_to_string(value)
            }
            _ => Err(self.builtin_argument_error(value, index, function, "string")),
        }
    }

    pub(super) fn builtin_number_arg(
        &mut self,
        value: Option<Value>,
        index: usize,
        function: &str,
    ) -> Result<f64> {
        if let Some(value) = value {
            if let Some(number) = self.coerce_to_number(value)? {
                return Ok(number);
            }
        }
        Err(self.builtin_argument_error(value, index, function, "number"))
    }
}
