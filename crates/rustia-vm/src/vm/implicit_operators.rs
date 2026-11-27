//! Scalar implicit operators share the same explicit callback boundary as field access.
use super::*;
impl Vm {
    // Native table.sort still uses the synchronous operation wrapper.
    pub(super) fn less_than(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<bool> {
        let step = self.less_than_step(program, lhs, rhs, depth, fuel)?;
        self.finish_implicit_step(step, fuel, first_result_or_nil)
            .map(Value::truthy)
    }
    pub(super) fn order_values_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        comparison: OrderComparison,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        match comparison {
            OrderComparison::Less => self.less_than_step(program, lhs, rhs, depth, fuel),
            OrderComparison::LessEqual => self.less_equal_step(program, lhs, rhs, depth, fuel),
            OrderComparison::Greater => self.less_than_step(program, rhs, lhs, depth, fuel),
            OrderComparison::GreaterEqual => self.less_equal_step(program, rhs, lhs, depth, fuel),
        }
    }
    pub(super) fn numeric_binop_value_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        op: NumericOp,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if lhs.kind() == ValueKind::Vector || rhs.kind() == ValueKind::Vector {
            return self
                .vector_binop_value(lhs, rhs, op)
                .map(ImplicitStep::Ready);
        }
        if let (Some(lhs_number), Some(rhs_number)) =
            (self.coerce_to_number(lhs)?, self.coerce_to_number(rhs)?)
        {
            return Ok(ImplicitStep::Ready(Value::number(binary(
                op, lhs_number, rhs_number,
            ))));
        }
        let Some(method_name) = arithmetic_metamethod_name(op) else {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        };
        if let Some(method) = self.binary_metamethod(lhs, rhs, method_name)? {
            self.implicit_call(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)
                .map(|step| step.map(first_result_or_nil))
        } else {
            Err(self.with_diagnostic_reason(
                arithmetic_error(op, lhs, rhs),
                crate::diagnostic::DiagnosticReason::Arithmetic {
                    operation: arithmetic_error_op_name(op),
                    left_type: value_type_name(lhs),
                    right_type: value_type_name(rhs),
                },
            ))
        }
    }
    pub(super) fn numeric_unop_value_step(
        &mut self,
        program: &Program,
        value: Value,
        op: NumericOp,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if value.kind() == ValueKind::Vector && op == NumericOp::Neg {
            let vector = self.vector_data(value)?;
            return self
                .alloc_vector_value(-vector.x(), -vector.y(), -vector.z())
                .map(ImplicitStep::Ready);
        }
        if let Some(number) = self.coerce_to_number(value)? {
            return Ok(ImplicitStep::Ready(Value::number(binary(op, number, 0.0))));
        }
        let Some(method_name) = arithmetic_metamethod_name(op) else {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        };
        if let Some(method) = self.metamethod_for_value(value, method_name)? {
            self.implicit_call(program, method, &[value], depth.saturating_add(1), fuel)
                .map(|step| step.map(first_result_or_nil))
        } else {
            Err(unary_arithmetic_error(op, value))
        }
    }
    pub(super) fn less_than_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if let Some(result) = self.raw_order(lhs, rhs, OrderComparison::Less)? {
            return Ok(ImplicitStep::Ready(Value::boolean(result)));
        }
        let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__lt")? else {
            return Err(order_error(lhs, rhs, "<"));
        };
        self.implicit_call_result(
            program,
            method,
            &[lhs, rhs],
            depth.saturating_add(1),
            fuel,
            ImplicitResult::Truthy { invert: false },
        )
        .map(|step| step.map(first_result_or_nil))
    }
    pub(super) fn less_equal_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if let Some(result) = self.raw_order(lhs, rhs, OrderComparison::LessEqual)? {
            return Ok(ImplicitStep::Ready(Value::boolean(result)));
        }
        if let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__le")? {
            return self
                .implicit_call_result(
                    program,
                    method,
                    &[lhs, rhs],
                    depth.saturating_add(1),
                    fuel,
                    ImplicitResult::Truthy { invert: false },
                )
                .map(|step| step.map(first_result_or_nil));
        }
        let Some(method) = self.compatible_binary_metamethod(rhs, lhs, "__lt")? else {
            return Err(order_error(lhs, rhs, "<="));
        };
        self.implicit_call_result(
            program,
            method,
            &[rhs, lhs],
            depth.saturating_add(1),
            fuel,
            ImplicitResult::Truthy { invert: true },
        )
        .map(|step| step.map(first_result_or_nil))
    }
    pub(super) fn concat_operands_step(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if let (Ok(mut lhs_text), Ok(rhs_text)) =
            (self.value_to_string(lhs), self.value_to_string(rhs))
        {
            if let Some(atom) = self.settings.memory_limit_bytes.and_then(|_| {
                self.heap
                    .find_concatenated_string_atom(&lhs_text, &rhs_text)
            }) {
                return Ok(ImplicitStep::Ready(Value::string_atom(atom)));
            }
            let result_len = lhs_text.len().saturating_add(rhs_text.len());
            self.ensure_planned_memory_growth(
                |vm| vm.heap.planned_owned_string_live_bytes(result_len),
                &[lhs, rhs],
            )?;
            lhs_text
                .try_reserve_exact(rhs_text.len())
                .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
            lhs_text.extend_from_slice(&rhs_text);
            return Ok(ImplicitStep::Ready(
                self.heap.intern_string_value_bytes(&lhs_text),
            ));
        }
        if let Some(method) = self.binary_metamethod(lhs, rhs, "__concat")? {
            self.implicit_call(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)
                .map(|step| step.map(first_result_or_nil))
        } else {
            Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "attempt to concatenate {} with {}",
                    value_type_name(lhs),
                    value_type_name(rhs)
                ),
            }))
        }
    }
    pub(super) fn len_value_step(
        &mut self,
        program: &Program,
        value: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        if value.kind() == ValueKind::String {
            return Ok(ImplicitStep::Ready(Value::number(
                self.raw_len_value(value)? as f64,
            )));
        }
        match value.kind() {
            ValueKind::Table | ValueKind::Userdata => {
                if let Some(method) = self.metamethod_for_value(value, "__len")? {
                    self.implicit_call_result(
                        program,
                        method,
                        &[value],
                        depth.saturating_add(1),
                        fuel,
                        ImplicitResult::Length,
                    )
                    .map(|step| step.map(first_result_or_nil))
                } else {
                    Ok(ImplicitStep::Ready(Value::number(
                        self.raw_len_value(value)? as f64,
                    )))
                }
            }
            _ => Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "attempt to get length of a {} value",
                    self.luau_type_name(value)
                ),
            })),
        }
    }
}
