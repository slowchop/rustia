//! Private, fuel-invariant quickening plans.
//!
//! Public bytecode remains the semantic source of truth. These plans are rebuilt when a
//! program is loaded and are entered only when the VM can run a complete logical loop
//! iteration without crossing an observable policy checkpoint.

use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};
use crate::number::NumericOp;
use std::collections::BTreeSet;
use std::sync::Arc;

const MAX_KERNEL_BODY_INSTRUCTIONS: usize = 128;
const MAX_KERNEL_EXPRESSION_NODES: usize = 128;
const MAX_KERNEL_EXPANDED_EVALUATION_NODES: usize = 512;

#[derive(Clone, Debug)]
pub(crate) struct NumericLoopKernel {
    pub(crate) backedge_pc: usize,
    pub(crate) body_instruction_count: u64,
    pub(crate) index_register: u8,
    pub(crate) limit_register: u8,
    pub(crate) step_register: u8,
    pub(crate) visible_register: u8,
    pub(crate) register_writes: Box<[KernelRegisterWrite]>,
    pub(crate) cell_clears: Box<[u8]>,
    pub(crate) array_write: Option<KernelArrayWrite>,
}

impl NumericLoopKernel {
    #[inline]
    pub(crate) const fn continuing_iteration_instruction_count(&self) -> u64 {
        1 + self.body_instruction_count
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KernelRegisterWrite {
    pub(crate) register: u8,
    pub(crate) value: KernelValueExpr,
}

#[derive(Clone, Debug)]
pub(crate) struct KernelArrayWrite {
    pub(crate) table: KernelValueExpr,
    pub(crate) index: Arc<KernelNumberExpr>,
    pub(crate) value: KernelValueExpr,
}

#[derive(Clone, Debug)]
pub(crate) enum KernelValueExpr {
    Input(u8),
    Nil,
    Boolean(bool),
    Number(Arc<KernelNumberExpr>),
}

#[derive(Clone, Debug)]
pub(crate) enum KernelNumberExpr {
    Input(u8),
    Constant(f64),
    Binary {
        op: NumericOp,
        lhs: Arc<Self>,
        rhs: Arc<Self>,
    },
    NumericArrayGet {
        table: KernelValueExpr,
        index: Arc<Self>,
    },
}

/// Build guarded numeric-loop kernels from complete bytecode bodies. No source or debug names
/// participate in recognition, and malformed or unsupported bytecode simply stays interpreted.
pub(crate) fn recognize_numeric_loop_kernels(
    program: &Program,
    function: &Function,
) -> Vec<Arc<NumericLoopKernel>> {
    function
        .code
        .iter()
        .enumerate()
        .filter_map(|(pc, instruction)| {
            (instruction.op == Opcode::ForLoop)
                .then(|| recognize_numeric_loop_kernel(program, function, pc, *instruction))?
        })
        .map(Arc::new)
        .collect()
}

fn recognize_numeric_loop_kernel(
    program: &Program,
    function: &Function,
    backedge_pc: usize,
    backedge: Instruction,
) -> Option<NumericLoopKernel> {
    let body_start = usize::try_from(backedge.aux).ok()?;
    let body = function.code.get(body_start..backedge_pc)?;
    if body.is_empty() || body.len() > MAX_KERNEL_BODY_INSTRUCTIONS {
        return None;
    }

    let index_register = backedge.a;
    let limit_register = index_register.checked_add(1)?;
    let step_register = index_register.checked_add(2)?;
    let visible_register = index_register.checked_add(3)?;
    if usize::from(visible_register) >= function.max_registers {
        return None;
    }

    let mut values = (0..function.max_registers)
        .map(|register| u8::try_from(register).ok().map(KernelValueExpr::Input))
        .collect::<Option<Vec<_>>>()?;
    let mut dirty = vec![false; function.max_registers];
    let mut cell_clears = Vec::new();
    let mut array_write = None;
    let mut expression_nodes = 0usize;
    let mut fallible_results = Vec::new();
    let mut after_array_write = false;

    for instruction in body.iter().copied() {
        if after_array_write && instruction.op != Opcode::ClearLocalCell {
            // The first slice commits one array effect after every fallible expression guard.
            // Requiring only cleanup after it makes that commit atomic with the original body.
            return None;
        }
        match instruction.op {
            Opcode::Move => {
                let value = values.get(usize::from(instruction.b))?.clone();
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty[usize::from(instruction.a)] = true;
            }
            Opcode::LoadConst => {
                let value = kernel_constant(program.constants().get(usize::from(instruction.c))?)?;
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty[usize::from(instruction.a)] = true;
            }
            Opcode::LoadConstWide => {
                let constant = usize::try_from(instruction.aux).ok()?;
                let value = kernel_constant(program.constants().get(constant)?)?;
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty[usize::from(instruction.a)] = true;
            }
            Opcode::ClearLocalCell => {
                *values.get_mut(usize::from(instruction.a))? = KernelValueExpr::Nil;
                dirty[usize::from(instruction.a)] = true;
                if !cell_clears.contains(&instruction.a) {
                    cell_clears.push(instruction.a);
                }
            }
            Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::FloorDiv
            | Opcode::Mod
            | Opcode::Pow => {
                let lhs = kernel_number(values.get(usize::from(instruction.b))?)?;
                let rhs = kernel_number(values.get(usize::from(instruction.c))?)?;
                expression_nodes = expression_nodes.checked_add(1)?;
                if expression_nodes > MAX_KERNEL_EXPRESSION_NODES {
                    return None;
                }
                let expression = Arc::new(KernelNumberExpr::Binary {
                    op: numeric_op(instruction.op)?,
                    lhs,
                    rhs,
                });
                fallible_results.push(Arc::clone(&expression));
                let value = KernelValueExpr::Number(expression);
                *values.get_mut(usize::from(instruction.a))? = value;
                dirty[usize::from(instruction.a)] = true;
            }
            Opcode::GetIndex => {
                let table = values.get(usize::from(instruction.b))?.clone();
                let index = kernel_number(values.get(usize::from(instruction.c))?)?;
                expression_nodes = expression_nodes.checked_add(1)?;
                if expression_nodes > MAX_KERNEL_EXPRESSION_NODES {
                    return None;
                }
                let expression = Arc::new(KernelNumberExpr::NumericArrayGet { table, index });
                fallible_results.push(Arc::clone(&expression));
                *values.get_mut(usize::from(instruction.a))? = KernelValueExpr::Number(expression);
                dirty[usize::from(instruction.a)] = true;
            }
            Opcode::SetIndex => {
                if array_write.is_some() {
                    return None;
                }
                array_write = Some(KernelArrayWrite {
                    table: values.get(usize::from(instruction.a))?.clone(),
                    index: kernel_number(values.get(usize::from(instruction.b))?)?,
                    value: values.get(usize::from(instruction.c))?.clone(),
                });
                after_array_write = true;
            }
            _ => return None,
        }
    }

    let register_writes = values
        .into_iter()
        .enumerate()
        .filter_map(|(register, value)| {
            dirty[register].then(|| {
                Some(KernelRegisterWrite {
                    register: u8::try_from(register).ok()?,
                    value,
                })
            })?
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();

    // Arc sharing keeps recognition linear, but the recursive evaluator intentionally has no
    // memo table. Count the expanded tree with a decrementing budget so adversarial `x=x+x`
    // bytecode cannot turn a small DAG into exponential work at run time.
    let mut evaluation_budget = MAX_KERNEL_EXPANDED_EVALUATION_NODES;
    for write in register_writes.iter() {
        charge_value_evaluation(&write.value, &mut evaluation_budget)?;
    }
    if let Some(write) = &array_write {
        charge_value_evaluation(&write.table, &mut evaluation_budget)?;
        charge_number_evaluation(&write.index, &mut evaluation_budget)?;
        charge_value_evaluation(&write.value, &mut evaluation_budget)?;
    }

    // A numeric operation or indexed read is only safe to elide when its guarded result is
    // still evaluated by a committed register/table effect. Even an overwritten result can
    // raise an arithmetic error or invoke `__index`; those loops must stay on semantic bytecode.
    let mut reachable = BTreeSet::new();
    for write in register_writes.iter() {
        mark_reachable_value_expressions(&write.value, &mut reachable);
    }
    if let Some(write) = &array_write {
        mark_reachable_value_expressions(&write.table, &mut reachable);
        mark_reachable_number_expressions(&write.index, &mut reachable);
        mark_reachable_value_expressions(&write.value, &mut reachable);
    }
    if fallible_results
        .iter()
        .any(|expression| !reachable.contains(&(Arc::as_ptr(expression) as usize)))
    {
        return None;
    }

    Some(NumericLoopKernel {
        backedge_pc,
        body_instruction_count: u64::try_from(body.len()).ok()?,
        index_register,
        limit_register,
        step_register,
        visible_register,
        register_writes,
        cell_clears: cell_clears.into_boxed_slice(),
        array_write,
    })
}

fn mark_reachable_value_expressions(value: &KernelValueExpr, reachable: &mut BTreeSet<usize>) {
    if let KernelValueExpr::Number(expression) = value {
        mark_reachable_number_expressions(expression, reachable);
    }
}

fn mark_reachable_number_expressions(
    expression: &Arc<KernelNumberExpr>,
    reachable: &mut BTreeSet<usize>,
) {
    if !reachable.insert(Arc::as_ptr(expression) as usize) {
        return;
    }
    match expression.as_ref() {
        KernelNumberExpr::Input(_) | KernelNumberExpr::Constant(_) => {}
        KernelNumberExpr::Binary { lhs, rhs, .. } => {
            mark_reachable_number_expressions(lhs, reachable);
            mark_reachable_number_expressions(rhs, reachable);
        }
        KernelNumberExpr::NumericArrayGet { table, index } => {
            mark_reachable_value_expressions(table, reachable);
            mark_reachable_number_expressions(index, reachable);
        }
    }
}

fn charge_value_evaluation(value: &KernelValueExpr, budget: &mut usize) -> Option<()> {
    *budget = budget.checked_sub(1)?;
    if let KernelValueExpr::Number(expression) = value {
        charge_number_evaluation(expression, budget)?;
    }
    Some(())
}

fn charge_number_evaluation(expression: &KernelNumberExpr, budget: &mut usize) -> Option<()> {
    *budget = budget.checked_sub(1)?;
    match expression {
        KernelNumberExpr::Input(_) | KernelNumberExpr::Constant(_) => {}
        KernelNumberExpr::Binary { lhs, rhs, .. } => {
            charge_number_evaluation(lhs, budget)?;
            charge_number_evaluation(rhs, budget)?;
        }
        KernelNumberExpr::NumericArrayGet { table, index } => {
            charge_value_evaluation(table, budget)?;
            charge_number_evaluation(index, budget)?;
        }
    }
    Some(())
}

fn kernel_constant(constant: &Constant) -> Option<KernelValueExpr> {
    match constant {
        Constant::Nil => Some(KernelValueExpr::Nil),
        Constant::Boolean(value) => Some(KernelValueExpr::Boolean(*value)),
        Constant::Number(value) => Some(KernelValueExpr::Number(Arc::new(
            KernelNumberExpr::Constant(*value),
        ))),
        Constant::String(_) | Constant::Import(_) => None,
    }
}

fn kernel_number(value: &KernelValueExpr) -> Option<Arc<KernelNumberExpr>> {
    match value {
        KernelValueExpr::Input(register) => Some(Arc::new(KernelNumberExpr::Input(*register))),
        KernelValueExpr::Number(value) => Some(Arc::clone(value)),
        KernelValueExpr::Nil | KernelValueExpr::Boolean(_) => None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_general_numeric_array_fill_and_fold_loops() {
        let constants = vec![Constant::Number(3.0)];
        let fill = Function::new(
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
        let program = Program::new(fill.clone(), constants);
        let kernels = recognize_numeric_loop_kernels(&program, &fill);
        assert_eq!(kernels.len(), 1);
        assert_eq!(kernels[0].body_instruction_count, 10);
        assert!(kernels[0].array_write.is_some());
    }

    #[test]
    fn rejects_effects_followed_by_fallible_work() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::SetIndex, 4, 3, 5),
                Instruction::new(Opcode::Add, 6, 6, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            7,
        );
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_numeric_loop_kernels(&program, &function).is_empty());
    }

    #[test]
    fn rejects_exponentially_expanding_expression_dags() {
        let mut code = Vec::new();
        for _ in 0..16 {
            code.push(Instruction::new(Opcode::Add, 4, 4, 4));
        }
        code.push(Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0));
        let function = Function::new(code, 5);
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_numeric_loop_kernels(&program, &function).is_empty());
    }

    #[test]
    fn rejects_expanding_arithmetic_even_when_later_overwritten() {
        let mut code = Vec::new();
        for _ in 0..16 {
            code.push(Instruction::new(Opcode::Add, 4, 4, 4));
        }
        code.push(Instruction::new(Opcode::LoadConst, 4, 0, 0));
        code.push(Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0));
        let function = Function::new(code, 5);
        let program = Program::new(function.clone(), vec![Constant::Number(0.0)]);
        assert!(recognize_numeric_loop_kernels(&program, &function).is_empty());
    }

    #[test]
    fn rejects_overwritten_arithmetic_that_can_still_error() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::Add, 4, 4, 3),
                Instruction::new(Opcode::ClearLocalCell, 4, 0, 0),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            5,
        );
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_numeric_loop_kernels(&program, &function).is_empty());
    }

    #[test]
    fn rejects_overwritten_index_reads_that_can_invoke_guest_code() {
        let function = Function::new(
            vec![
                Instruction::new(Opcode::GetIndex, 5, 4, 3),
                Instruction::new(Opcode::ClearLocalCell, 5, 0, 0),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            ],
            6,
        );
        let program = Program::new(function.clone(), Vec::new());
        assert!(recognize_numeric_loop_kernels(&program, &function).is_empty());
    }
}
