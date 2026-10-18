//! Private, bounded HotIR plans for policy-invisible loop execution.
//!
//! HotIR is deliberately smaller than public bytecode. The interpreter remains the semantic
//! authority: recognition accepts only a complete loop body whose guards can be evaluated
//! before any guest-visible effect, and a failed guard resumes at the original `ForLoop`.
//!
//! Operations are stored in bytecode source order and produce typed SSA-like number temporaries.
//! Inputs and constants are leaves, so evaluating a plan is a single forward pass with bounded
//! scratch storage rather than a recursive expression walk.

use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};
use crate::number::NumericOp;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) const MAX_HOT_IR_BODY_INSTRUCTIONS: usize = 128;
pub(crate) const MAX_HOT_IR_OPERATIONS: usize = 128;
const MAX_HOT_IR_REGISTERS: usize = (u8::MAX as usize) + 1;

/// A complete, guarded numeric-for loop plan.
///
/// The VM may commit `register_writes`, `cell_clears`, and `array_write` only after every
/// operation and effect operand has passed its guards. `operations` are always in original
/// bytecode order, which preserves the interpreter's first-failure order.
#[derive(Clone, Debug)]
pub(crate) struct HotIr {
    pub(crate) body_start_pc: usize,
    pub(crate) backedge_pc: usize,
    pub(crate) body_instruction_count: u64,
    pub(crate) register_count: u16,
    pub(crate) index_register: u8,
    pub(crate) limit_register: u8,
    pub(crate) step_register: u8,
    pub(crate) visible_register: u8,
    pub(crate) operations: Box<[HotIrOp]>,
    pub(crate) register_writes: Box<[HotIrRegisterWrite]>,
    pub(crate) cell_clears: Box<[HotIrCellClear]>,
    pub(crate) array_write: Option<HotIrArrayWrite>,
}

impl HotIr {
    pub(crate) fn memory_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.operations.len().saturating_mul(size_of::<HotIrOp>()))
            .saturating_add(
                self.register_writes
                    .len()
                    .saturating_mul(size_of::<HotIrRegisterWrite>()),
            )
            .saturating_add(
                self.cell_clears
                    .len()
                    .saturating_mul(size_of::<HotIrCellClear>()),
            )
    }

    #[inline]
    pub(crate) const fn continuing_iteration_instruction_count(&self) -> u64 {
        1 + self.body_instruction_count
    }

    /// Verify structural bounds, typing, forward-only SSA references, and source order.
    ///
    /// Every operation is executed exactly once in this order, including operations whose
    /// result is later overwritten. This preserves fallible bytecode without requiring outputs
    /// to remain reachable from the final staged writes.
    pub(crate) fn verify(&self) -> Result<(), HotIrVerificationError> {
        let body_instruction_count = usize::try_from(self.body_instruction_count)
            .map_err(|_| HotIrVerificationError::InvalidBodyRange)?;
        if body_instruction_count == 0 {
            return Err(HotIrVerificationError::EmptyBody);
        }
        if body_instruction_count > MAX_HOT_IR_BODY_INSTRUCTIONS {
            return Err(HotIrVerificationError::BodyTooLarge);
        }
        if self
            .body_start_pc
            .checked_add(body_instruction_count)
            .filter(|end| *end == self.backedge_pc)
            .is_none()
        {
            return Err(HotIrVerificationError::InvalidBodyRange);
        }

        let register_count = usize::from(self.register_count);
        if register_count == 0 || register_count > MAX_HOT_IR_REGISTERS {
            return Err(HotIrVerificationError::InvalidRegisterCount);
        }
        if self.index_register.checked_add(1) != Some(self.limit_register)
            || self.index_register.checked_add(2) != Some(self.step_register)
            || self.index_register.checked_add(3) != Some(self.visible_register)
            || usize::from(self.visible_register) >= register_count
        {
            return Err(HotIrVerificationError::InvalidLoopRegisters);
        }

        if self.operations.len() > MAX_HOT_IR_OPERATIONS {
            return Err(HotIrVerificationError::TooManyOperations);
        }
        let mut previous_source_pc = None;
        for (operation_index, operation) in self.operations.iter().enumerate() {
            self.verify_source_pc(operation.source_pc)?;
            if previous_source_pc.is_some_and(|previous| previous >= operation.source_pc) {
                return Err(HotIrVerificationError::OperationsOutOfSourceOrder {
                    operation: operation_index,
                });
            }
            previous_source_pc = Some(operation.source_pc);
            operation.verify_operands(operation_index, register_count)?;
        }

        let mut previous_register = None;
        for (write_index, write) in self.register_writes.iter().enumerate() {
            if usize::from(write.register) >= register_count {
                return Err(HotIrVerificationError::RegisterOutOfBounds {
                    register: write.register,
                });
            }
            self.verify_source_pc(write.source_pc)?;
            if previous_register.is_some_and(|previous| previous >= write.register) {
                return Err(HotIrVerificationError::RegisterWritesOutOfOrder {
                    write: write_index,
                });
            }
            previous_register = Some(write.register);
            self.verify_value(write.value, self.operations.len(), register_count)?;
            self.verify_value_available_at(write.value, write.source_pc)?;
        }

        let mut cleared_registers = [false; MAX_HOT_IR_REGISTERS];
        let mut previous_clear_pc = None;
        for (clear_index, clear) in self.cell_clears.iter().enumerate() {
            let register = usize::from(clear.register);
            if register >= register_count {
                return Err(HotIrVerificationError::RegisterOutOfBounds {
                    register: clear.register,
                });
            }
            self.verify_source_pc(clear.source_pc)?;
            if previous_clear_pc.is_some_and(|previous| previous >= clear.source_pc) {
                return Err(HotIrVerificationError::CellClearsOutOfSourceOrder {
                    clear: clear_index,
                });
            }
            previous_clear_pc = Some(clear.source_pc);
            if std::mem::replace(&mut cleared_registers[register], true) {
                return Err(HotIrVerificationError::DuplicateCellClear {
                    register: clear.register,
                });
            }
            let write_index = self
                .register_writes
                .binary_search_by_key(&clear.register, |write| write.register)
                .map_err(|_| HotIrVerificationError::CellClearWithoutRegisterWrite {
                    register: clear.register,
                })?;
            let write = &self.register_writes[write_index];
            if clear.source_pc > write.source_pc {
                return Err(HotIrVerificationError::CellClearAfterRegisterWrite {
                    register: clear.register,
                });
            }
        }

        if let Some(write) = &self.array_write {
            self.verify_source_pc(write.source_pc)?;
            if previous_source_pc.is_some_and(|previous| previous >= write.source_pc) {
                return Err(HotIrVerificationError::ArrayWriteBeforeFallibleOperation);
            }
            self.verify_value(write.table, self.operations.len(), register_count)?;
            self.verify_number(write.index, self.operations.len(), register_count)?;
            self.verify_value(write.value, self.operations.len(), register_count)?;
        }

        Ok(())
    }

    fn verify_source_pc(&self, source_pc: usize) -> Result<(), HotIrVerificationError> {
        if (self.body_start_pc..self.backedge_pc).contains(&source_pc) {
            Ok(())
        } else {
            Err(HotIrVerificationError::SourceOutsideBody { source_pc })
        }
    }

    fn verify_value(
        &self,
        value: HotIrValue,
        temporary_limit: usize,
        register_count: usize,
    ) -> Result<(), HotIrVerificationError> {
        match value {
            HotIrValue::Input(register) => verify_input_register(register, register_count),
            HotIrValue::Nil | HotIrValue::Boolean(_) => Ok(()),
            HotIrValue::Number(number) => {
                self.verify_number(number, temporary_limit, register_count)
            }
        }
    }

    fn verify_number(
        &self,
        number: HotIrNumber,
        temporary_limit: usize,
        register_count: usize,
    ) -> Result<(), HotIrVerificationError> {
        match number {
            HotIrNumber::Input(register) => verify_input_register(register, register_count),
            HotIrNumber::Constant(_) => Ok(()),
            HotIrNumber::Temporary(temporary) if temporary.index() < temporary_limit => Ok(()),
            HotIrNumber::Temporary(temporary) => Err(HotIrVerificationError::InvalidTemporary {
                temporary: temporary.index(),
                limit: temporary_limit,
            }),
        }
    }

    fn verify_value_available_at(
        &self,
        value: HotIrValue,
        source_pc: usize,
    ) -> Result<(), HotIrVerificationError> {
        let HotIrValue::Number(HotIrNumber::Temporary(temporary)) = value else {
            return Ok(());
        };
        let definition_pc = self.operations[temporary.index()].source_pc;
        if definition_pc <= source_pc {
            Ok(())
        } else {
            Err(HotIrVerificationError::ValueUsedBeforeDefinition {
                temporary: temporary.index(),
                source_pc,
                definition_pc,
            })
        }
    }
}

/// A number-producing instruction. Its output is the operation's index in `HotIr::operations`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HotIrOp {
    pub(crate) source_pc: usize,
    pub(crate) kind: HotIrOpKind,
}

impl HotIrOp {
    fn verify_operands(
        &self,
        operation_index: usize,
        register_count: usize,
    ) -> Result<(), HotIrVerificationError> {
        match self.kind {
            HotIrOpKind::Binary { lhs, rhs, .. } => {
                verify_number_operand(lhs, operation_index, register_count)?;
                verify_number_operand(rhs, operation_index, register_count)
            }
            HotIrOpKind::NumericArrayGet { table, index } => {
                verify_value_operand(table, operation_index, register_count)?;
                verify_number_operand(index, operation_index, register_count)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HotIrOpKind {
    Binary {
        op: NumericOp,
        lhs: HotIrNumber,
        rhs: HotIrNumber,
    },
    NumericArrayGet {
        table: HotIrValue,
        index: HotIrNumber,
    },
}

/// A boxed-or-unboxed value operand. `Number` remains unboxed until an effect is staged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HotIrValue {
    Input(u8),
    Nil,
    Boolean(bool),
    Number(HotIrNumber),
}

/// A statically numeric operand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HotIrNumber {
    Input(u8),
    Constant(f64),
    Temporary(HotIrNumberId),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct HotIrNumberId(u16);

impl HotIrNumberId {
    #[inline]
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    #[inline]
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HotIrRegisterWrite {
    pub(crate) source_pc: usize,
    pub(crate) register: u8,
    pub(crate) value: HotIrValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HotIrCellClear {
    pub(crate) source_pc: usize,
    pub(crate) register: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HotIrArrayWrite {
    pub(crate) source_pc: usize,
    pub(crate) table: HotIrValue,
    pub(crate) index: HotIrNumber,
    pub(crate) value: HotIrValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HotIrVerificationError {
    EmptyBody,
    BodyTooLarge,
    InvalidBodyRange,
    InvalidRegisterCount,
    InvalidLoopRegisters,
    TooManyOperations,
    SourceOutsideBody {
        source_pc: usize,
    },
    OperationsOutOfSourceOrder {
        operation: usize,
    },
    InvalidTemporary {
        temporary: usize,
        limit: usize,
    },
    RegisterOutOfBounds {
        register: u8,
    },
    RegisterWritesOutOfOrder {
        write: usize,
    },
    CellClearsOutOfSourceOrder {
        clear: usize,
    },
    DuplicateCellClear {
        register: u8,
    },
    CellClearWithoutRegisterWrite {
        register: u8,
    },
    CellClearAfterRegisterWrite {
        register: u8,
    },
    ValueUsedBeforeDefinition {
        temporary: usize,
        source_pc: usize,
        definition_pc: usize,
    },
    ArrayWriteBeforeFallibleOperation,
}

/// Recognize all complete numeric-for bodies supported by the first HotIR slice.
///
/// Unsupported or malformed bytecode stays interpreted. Debug names and source lines never
/// participate in recognition.
pub(crate) fn recognize_hot_ir_loops(program: &Program, function: &Function) -> Vec<Arc<HotIr>> {
    function
        .code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| {
            (instruction.op == Opcode::ForLoop)
                .then(|| recognize_hot_ir_loop(program, function, pc, *instruction))?
        })
        .map(Arc::new)
        .collect()
}

fn recognize_hot_ir_loop(
    program: &Program,
    function: &Function,
    backedge_pc: usize,
    backedge: Instruction,
) -> Option<HotIr> {
    let body_start_pc = usize::try_from(backedge.aux).ok()?;
    let body = function.code.get(body_start_pc..backedge_pc)?;
    if body.is_empty() || body.len() > MAX_HOT_IR_BODY_INSTRUCTIONS {
        return None;
    }

    let index_register = backedge.a;
    let limit_register = index_register.checked_add(1)?;
    let step_register = index_register.checked_add(2)?;
    let visible_register = index_register.checked_add(3)?;
    if usize::from(visible_register) >= function.max_registers
        || function.max_registers > MAX_HOT_IR_REGISTERS
    {
        return None;
    }
    let register_count = u16::try_from(function.max_registers).ok()?;

    let mut values = (0..function.max_registers)
        .map(|register| u8::try_from(register).ok().map(HotIrValue::Input))
        .collect::<Option<Vec<_>>>()?;
    let mut dirty_source_pc = vec![None; function.max_registers];
    let mut cell_clears = Vec::new();
    let mut cleared_registers = [false; MAX_HOT_IR_REGISTERS];
    let mut array_write = None;
    let mut operations = Vec::new();
    let mut after_array_write = false;

    for (body_offset, instruction) in body.iter().copied().enumerate() {
        let source_pc = body_start_pc.checked_add(body_offset)?;
        if after_array_write && instruction.op != Opcode::ClearLocalCell {
            // One array effect is committed only after every fallible guard. Restricting the
            // suffix to cell cleanup makes the effect atomic with the original loop body.
            return None;
        }

        match instruction.op {
            Opcode::Move => {
                let value = *values.get(usize::from(instruction.b))?;
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
            }
            Opcode::LoadConst => {
                let value = hot_ir_constant(program.constants().get(usize::from(instruction.c))?)?;
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
            }
            Opcode::LoadConstWide => {
                let constant = usize::try_from(instruction.aux).ok()?;
                let value = hot_ir_constant(program.constants().get(constant)?)?;
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
            }
            Opcode::ClearLocalCell => {
                *values.get_mut(usize::from(instruction.a))? = HotIrValue::Nil;
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
                let register = usize::from(instruction.a);
                if !std::mem::replace(&mut cleared_registers[register], true) {
                    cell_clears.push(HotIrCellClear {
                        source_pc,
                        register: instruction.a,
                    });
                }
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::FloorDiv
            | Opcode::Mod
            | Opcode::Pow => {
                let lhs = hot_ir_number(*values.get(usize::from(instruction.b))?)?;
                let rhs = hot_ir_number(*values.get(usize::from(instruction.c))?)?;
                let output = push_operation(
                    &mut operations,
                    source_pc,
                    HotIrOpKind::Binary {
                        op: numeric_op(instruction.op)?,
                        lhs,
                        rhs,
                    },
                )?;
                *values.get_mut(usize::from(instruction.a))? =
                    HotIrValue::Number(HotIrNumber::Temporary(output));
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
            }
            Opcode::GetIndex => {
                let table = *values.get(usize::from(instruction.b))?;
                let index = hot_ir_number(*values.get(usize::from(instruction.c))?)?;
                let output = push_operation(
                    &mut operations,
                    source_pc,
                    HotIrOpKind::NumericArrayGet { table, index },
                )?;
                *values.get_mut(usize::from(instruction.a))? =
                    HotIrValue::Number(HotIrNumber::Temporary(output));
                dirty_source_pc[usize::from(instruction.a)] = Some(source_pc);
            }
            Opcode::SetIndex => {
                if array_write.is_some() {
                    return None;
                }
                array_write = Some(HotIrArrayWrite {
                    source_pc,
                    table: *values.get(usize::from(instruction.a))?,
                    index: hot_ir_number(*values.get(usize::from(instruction.b))?)?,
                    value: *values.get(usize::from(instruction.c))?,
                });
                after_array_write = true;
            }
            _ => return None,
        }
    }

    let register_writes = values
        .into_iter()
        .zip(dirty_source_pc)
        .enumerate()
        .filter_map(|(register, (value, source_pc))| {
            Some(HotIrRegisterWrite {
                source_pc: source_pc?,
                register: u8::try_from(register).ok()?,
                value,
            })
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();

    let hot_ir = HotIr {
        body_start_pc,
        backedge_pc,
        body_instruction_count: u64::try_from(body.len()).ok()?,
        register_count,
        index_register,
        limit_register,
        step_register,
        visible_register,
        operations: operations.into_boxed_slice(),
        register_writes,
        cell_clears: cell_clears.into_boxed_slice(),
        array_write,
    };
    hot_ir.verify().ok()?;
    Some(hot_ir)
}

fn push_operation(
    operations: &mut Vec<HotIrOp>,
    source_pc: usize,
    kind: HotIrOpKind,
) -> Option<HotIrNumberId> {
    if operations.len() >= MAX_HOT_IR_OPERATIONS {
        return None;
    }
    let output = HotIrNumberId::from_index(operations.len())?;
    operations.push(HotIrOp { source_pc, kind });
    Some(output)
}

fn hot_ir_constant(constant: &Constant) -> Option<HotIrValue> {
    match constant {
        Constant::Nil => Some(HotIrValue::Nil),
        Constant::Boolean(value) => Some(HotIrValue::Boolean(*value)),
        Constant::Number(value) => Some(HotIrValue::Number(HotIrNumber::Constant(*value))),
        Constant::String(_) | Constant::Import(_) | Constant::CallDescriptor(_) => None,
    }
}

fn hot_ir_number(value: HotIrValue) -> Option<HotIrNumber> {
    match value {
        HotIrValue::Input(register) => Some(HotIrNumber::Input(register)),
        HotIrValue::Number(number) => Some(number),
        HotIrValue::Nil | HotIrValue::Boolean(_) => None,
    }
}

const fn numeric_op(opcode: Opcode) -> Option<NumericOp> {
    match opcode {
        Opcode::Add => Some(NumericOp::Add),
        Opcode::Sub => Some(NumericOp::Sub),
        Opcode::Mul => Some(NumericOp::Mul),
        Opcode::Div => Some(NumericOp::Div),
        Opcode::FloorDiv => Some(NumericOp::FloorDiv),
        Opcode::Mod => Some(NumericOp::Mod),
        Opcode::Pow => Some(NumericOp::Pow),
        _ => None,
    }
}

fn verify_input_register(
    register: u8,
    register_count: usize,
) -> Result<(), HotIrVerificationError> {
    if usize::from(register) < register_count {
        Ok(())
    } else {
        Err(HotIrVerificationError::RegisterOutOfBounds { register })
    }
}

fn verify_value_operand(
    value: HotIrValue,
    temporary_limit: usize,
    register_count: usize,
) -> Result<(), HotIrVerificationError> {
    match value {
        HotIrValue::Input(register) => verify_input_register(register, register_count),
        HotIrValue::Nil | HotIrValue::Boolean(_) => Ok(()),
        HotIrValue::Number(number) => {
            verify_number_operand(number, temporary_limit, register_count)
        }
    }
}

fn verify_number_operand(
    number: HotIrNumber,
    temporary_limit: usize,
    register_count: usize,
) -> Result<(), HotIrVerificationError> {
    match number {
        HotIrNumber::Input(register) => verify_input_register(register, register_count),
        HotIrNumber::Constant(_) => Ok(()),
        HotIrNumber::Temporary(temporary) if temporary.index() < temporary_limit => Ok(()),
        HotIrNumber::Temporary(temporary) => Err(HotIrVerificationError::InvalidTemporary {
            temporary: temporary.index(),
            limit: temporary_limit,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_flat_numeric_array_fill_in_source_order() {
        let constants = vec![Constant::Number(3.0)];
        let function = Function::new(
            vec![
                Instruction::new(Opcode::Move, 4, 0, 0),
                Instruction::new(Opcode::Move, 5, 3, 0),
                Instruction::new(Opcode::Move, 6, 3, 0),
                Instruction::new(Opcode::LoadConst, 7, 0, 0),
                Instruction::new(Opcode::Mul, 6, 6, 7),
                Instruction::new(Opcode::SetIndex, 4, 5, 6),
                Instruction::new(Opcode::ClearLocalCell, 7, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 6, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 5, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 4, 0, 0),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            8,
        );
        let program = Program::new(function.clone(), constants);
        let plans = recognize_hot_ir_loops(&program, &function);
        let plan = plans.first().expect("HotIR plan");

        assert_eq!(plan.body_instruction_count, 10);
        assert_eq!(plan.operations.len(), 1);
        assert_eq!(plan.operations[0].source_pc, 4);
        assert!(matches!(
            plan.operations[0].kind,
            HotIrOpKind::Binary {
                op: NumericOp::Mul,
                ..
            }
        ));
        assert_eq!(plan.array_write.as_ref().unwrap().source_pc, 5);
        assert_eq!(
            plan.cell_clears
                .iter()
                .map(|clear| clear.source_pc)
                .collect::<Vec<_>>(),
            vec![6, 7, 8, 9]
        );
        assert_eq!(plan.verify(), Ok(()));
    }

    #[test]
    fn numeric_reads_and_arithmetic_form_forward_typed_ssa() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::GetIndex, 5, 4, 3),
                Instruction::new(Opcode::Add, 6, 5, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            7,
        );
        let program = Program::new(function.clone(), Vec::new());
        let plans = recognize_hot_ir_loops(&program, &function);
        let plan = plans.first().expect("HotIR plan");

        assert_eq!(plan.operations.len(), 2);
        assert_eq!(plan.operations[0].source_pc, 0);
        assert!(matches!(
            plan.operations[0].kind,
            HotIrOpKind::NumericArrayGet { .. }
        ));
        assert_eq!(plan.operations[1].source_pc, 1);
        assert!(matches!(
            plan.operations[1].kind,
            HotIrOpKind::Binary {
                lhs: HotIrNumber::Temporary(HotIrNumberId(0)),
                ..
            }
        ));
    }

    #[test]
    fn rejects_an_effect_followed_by_fallible_work() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::SetIndex, 4, 3, 5),
                Instruction::new(Opcode::Add, 6, 6, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            7,
        );
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_hot_ir_loops(&program, &function).is_empty());
    }

    #[test]
    fn exponential_expression_dag_stays_flat_and_bounded() {
        let mut code = Vec::with_capacity(MAX_HOT_IR_OPERATIONS + 1);
        for _ in 0..MAX_HOT_IR_OPERATIONS {
            code.push(Instruction::new(Opcode::Add, 4, 4, 4));
        }
        code.push(Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0));
        let function = Function::new(code, 5);
        let program = Program::new(function.clone(), Vec::new());
        let plans = recognize_hot_ir_loops(&program, &function);
        let plan = plans.first().expect("bounded HotIR plan");

        assert_eq!(plan.operations.len(), MAX_HOT_IR_OPERATIONS);
        assert!(matches!(
            plan.operations[0].kind,
            HotIrOpKind::Binary {
                lhs: HotIrNumber::Input(4),
                rhs: HotIrNumber::Input(4),
                ..
            }
        ));
        for (operation_index, operation) in plan.operations.iter().enumerate().skip(1) {
            let previous = HotIrNumberId::from_index(operation_index - 1).unwrap();
            assert!(matches!(
                operation.kind,
                HotIrOpKind::Binary {
                    lhs: HotIrNumber::Temporary(lhs),
                    rhs: HotIrNumber::Temporary(rhs),
                    ..
                } if lhs == previous && rhs == previous
            ));
        }

        let mut too_large =
            vec![Instruction::new(Opcode::Add, 4, 4, 4); MAX_HOT_IR_BODY_INSTRUCTIONS + 1];
        too_large.push(Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0));
        let function = Function::new(too_large, 5);
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_hot_ir_loops(&program, &function).is_empty());
    }

    #[test]
    fn retains_overwritten_fallible_operations_in_source_order() {
        for opcode in [Opcode::Add, Opcode::GetIndex] {
            let fallible = Instruction::new(opcode, 5, 4, 3);
            let function = Function::new(
                vec![
                    fallible,
                    Instruction::new(Opcode::ClearLocalCell, 5, 0, 0),
                    Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
                ],
                6,
            );
            let program = Program::new(function.clone(), Vec::new());
            let plans = recognize_hot_ir_loops(&program, &function);
            let plan = plans.first().expect("HotIR plan");

            assert_eq!(plan.operations.len(), 1, "opcode={opcode:?}");
            assert_eq!(plan.operations[0].source_pc, 0, "opcode={opcode:?}");
            assert!(
                matches!(
                    (opcode, plan.operations[0].kind),
                    (Opcode::Add, HotIrOpKind::Binary { .. })
                        | (Opcode::GetIndex, HotIrOpKind::NumericArrayGet { .. })
                ),
                "opcode={opcode:?}"
            );
            assert_eq!(
                plan.register_writes.as_ref(),
                &[HotIrRegisterWrite {
                    source_pc: 1,
                    register: 5,
                    value: HotIrValue::Nil,
                }],
                "opcode={opcode:?}"
            );
            assert_eq!(plan.verify(), Ok(()), "opcode={opcode:?}");
        }
    }

    #[test]
    fn verifier_rejects_forward_ssa_references() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::Add, 4, 4, 3),
                Instruction::new(Opcode::Add, 5, 4, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            6,
        );
        let program = Program::new(function.clone(), Vec::new());
        let mut plan = recognize_hot_ir_loops(&program, &function)
            .first()
            .expect("HotIR plan")
            .as_ref()
            .clone();
        let HotIrOpKind::Binary { lhs, .. } = &mut plan.operations[0].kind else {
            unreachable!();
        };
        *lhs = HotIrNumber::Temporary(HotIrNumberId(1));

        assert_eq!(
            plan.verify(),
            Err(HotIrVerificationError::InvalidTemporary {
                temporary: 1,
                limit: 0,
            })
        );
    }

    #[test]
    fn verifier_rejects_reordered_fallible_operations() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::Add, 4, 4, 3),
                Instruction::new(Opcode::Add, 5, 5, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            6,
        );
        let program = Program::new(function.clone(), Vec::new());
        let mut plan = recognize_hot_ir_loops(&program, &function)
            .first()
            .expect("HotIR plan")
            .as_ref()
            .clone();
        plan.operations.swap(0, 1);

        assert_eq!(
            plan.verify(),
            Err(HotIrVerificationError::OperationsOutOfSourceOrder { operation: 1 })
        );
    }

    #[test]
    fn verifier_rejects_output_before_its_definition() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::Move, 5, 3, 0),
                Instruction::new(Opcode::Add, 4, 4, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            6,
        );
        let program = Program::new(function.clone(), Vec::new());
        let mut plan = recognize_hot_ir_loops(&program, &function)
            .first()
            .expect("HotIR plan")
            .as_ref()
            .clone();
        let write = plan
            .register_writes
            .iter_mut()
            .find(|write| write.register == 4)
            .expect("arithmetic result write");
        write.source_pc = 0;

        assert_eq!(
            plan.verify(),
            Err(HotIrVerificationError::ValueUsedBeforeDefinition {
                temporary: 0,
                source_pc: 0,
                definition_pc: 1,
            })
        );
    }

    #[test]
    fn verifier_rejects_cell_clear_after_its_final_register_write() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::ClearLocalCell, 5, 0, 0),
                Instruction::new(Opcode::Move, 5, 4, 0),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            6,
        );
        let program = Program::new(function.clone(), Vec::new());
        let mut plan = recognize_hot_ir_loops(&program, &function)
            .first()
            .expect("HotIR plan")
            .as_ref()
            .clone();
        plan.register_writes[0].source_pc = 0;
        plan.cell_clears[0].source_pc = 1;

        assert_eq!(
            plan.verify(),
            Err(HotIrVerificationError::CellClearAfterRegisterWrite { register: 5 })
        );
    }
}
