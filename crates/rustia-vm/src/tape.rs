//! Loader-built whole-function executable tape for plain-mode execution.
//!
//! The tape is a private, load-time predecoding of a function's public
//! bytecode. Every source PC owns exactly one [`TapeOp`]: verified common
//! operations carry predecoded operands and direct branch targets, while every
//! other instruction is an [`TapeOp::Exit`] effect boundary that returns
//! control to the scalar semantic dispatcher. Grouping is expressed by
//! [`FunctionTape::run_from`]: the number of consecutive logical instructions
//! executable from a PC before the next effect boundary or block terminator,
//! which doubles as the block's remaining logical cost for deterministic-GC
//! admission. Public bytecode remains the semantic authority; a guard miss
//! side-exits to the exact canonical instruction before any mutation.

use std::sync::Arc;

use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};
use crate::number::NumericOp;
use crate::register_blocks::{recognize_register_block, RegisterBlockPlan};
use crate::value::Value;

/// `run_suffix` sentinel for PCs that must use scalar dispatch.
const NOT_ON_TAPE: u16 = 0;

/// Comparison forms whose both-number fast path is effect-free.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TapeComparison {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

/// One predecoded instruction. Register operands are verified against the
/// function's register file at build time; branch targets are the public
/// bytecode's absolute PCs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TapeOp {
    /// Instruction outside the verified tape set; scalar dispatch owns it.
    Exit,
    /// `Move destination, source`.
    Move { destination: u8, source: u8 },
    /// `ClearLocalCell register`.
    ClearLocalCell { register: u8 },
    /// Nil/boolean/number constant load with the value predecoded into
    /// [`FunctionTape::constants`].
    LoadPrimitive { destination: u8, constant: u16 },
    /// Binary arithmetic guarded on both operands being numbers.
    Numeric {
        operation: NumericOp,
        destination: u8,
        lhs: u8,
        rhs: u8,
    },
    /// `Neg` guarded on a number operand.
    NumericNegate { destination: u8, source: u8 },
    /// `Not`: pure truthiness inversion, no guard.
    Not { destination: u8, source: u8 },
    /// Comparison guarded on both operands being numbers.
    Compare {
        comparison: TapeComparison,
        destination: u8,
        lhs: u8,
        rhs: u8,
    },
    /// `GetIndex` guarded to a present dense-array slot read.
    DenseArrayGet {
        destination: u8,
        table: u8,
        index: u8,
    },
    /// `SetIndex` guarded to an existing dense-array slot write.
    DenseArraySet { table: u8, index: u8, value: u8 },
    /// Fused four- or six-instruction register-lifetime receipt starting at
    /// this PC. Interior PCs keep their individual operations, so branches
    /// into the receipt and resumption after a boundary stay canonical. A
    /// guard miss executes the receipt's canonical first `Move` instead.
    Receipt { index: u16 },
    /// Unconditional branch to an absolute PC. Terminates a block.
    Jump { target: u32 },
    /// Branch when the condition register is truthy. Terminates a block.
    JumpIf { condition: u8, target: u32 },
    /// Branch when the condition register is falsy. Terminates a block.
    JumpIfNot { condition: u8, target: u32 },
    /// Numeric `ForLoop` backedge guarded on a numeric control window.
    /// Terminates a block. Built only when no HotIR owns the backedge.
    ForLoop { base: u8, target: u32 },
}

/// Whole-function executable tape: one op and one run-suffix entry per source
/// PC, plus predecoded primitive constants and fused receipts.
pub(crate) struct FunctionTape {
    ops: Box<[TapeOp]>,
    run_suffix: Box<[u16]>,
    constants: Box<[Value]>,
    receipts: Box<[RegisterBlockPlan]>,
}

impl FunctionTape {
    /// Logical instructions executable from `pc` before the next effect
    /// boundary or block terminator; zero when `pc` requires scalar dispatch.
    #[inline(always)]
    pub(crate) fn run_from(&self, pc: usize) -> u16 {
        self.run_suffix.get(pc).copied().unwrap_or(NOT_ON_TAPE)
    }

    /// The predecoded op at `pc`. Callers must only use PCs inside a run.
    #[inline(always)]
    pub(crate) fn op(&self, pc: usize) -> TapeOp {
        self.ops.get(pc).copied().unwrap_or(TapeOp::Exit)
    }

    #[inline(always)]
    pub(crate) fn constant(&self, index: u16) -> Value {
        self.constants
            .get(usize::from(index))
            .copied()
            .unwrap_or_else(Value::nil)
    }

    #[inline(always)]
    pub(crate) fn receipt(&self, index: u16) -> Option<RegisterBlockPlan> {
        self.receipts.get(usize::from(index)).copied()
    }

    /// Heap memory retained by the tape, charged to the VM live/reserved
    /// logical size when the owning program is admitted.
    pub(crate) fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.ops.len().saturating_mul(std::mem::size_of::<TapeOp>()))
            .saturating_add(
                self.run_suffix
                    .len()
                    .saturating_mul(std::mem::size_of::<u16>()),
            )
            .saturating_add(
                self.constants
                    .len()
                    .saturating_mul(std::mem::size_of::<Value>()),
            )
            .saturating_add(
                self.receipts
                    .len()
                    .saturating_mul(std::mem::size_of::<RegisterBlockPlan>()),
            )
    }

    /// Build the tape for one function, or `None` when no instruction
    /// qualifies. `hot_ir_backedges` lists `ForLoop` PCs owned by HotIR; those
    /// stay effect boundaries so the loop tiers never compete at one PC.
    pub(crate) fn build(
        program: &Program,
        function: &Function,
        hot_ir_backedges: &[usize],
    ) -> Option<Arc<Self>> {
        let code = &function.code;
        if code.is_empty() {
            return None;
        }

        let mut ops = vec![TapeOp::Exit; code.len()];
        let mut constants: Vec<Value> = Vec::new();
        // One transient slot per program constant so repeated loads of the
        // same primitive share one predecoded value. Zero means unassigned.
        let mut constant_slots = vec![0u32; program.constants().len()];
        let mut any = false;
        for (pc, instruction) in code.iter().copied().enumerate() {
            let op = predecode(
                program,
                function,
                instruction,
                pc,
                hot_ir_backedges,
                &mut constants,
                &mut constant_slots,
            );
            if op != TapeOp::Exit {
                any = true;
            }
            ops[pc] = op;
        }
        if !any {
            return None;
        }

        // Fused receipts: recognized on the public bytecode; each receipt's
        // interior PCs keep the individual predecoded ops written above.
        let mut receipts: Vec<RegisterBlockPlan> = Vec::new();
        for pc in 0..code.len() {
            if !matches!(ops[pc], TapeOp::Move { .. }) {
                continue;
            }
            let Some(plan) = recognize_register_block(function, pc) else {
                continue;
            };
            // The receipt is only fused when every represented instruction is
            // itself on tape, so a mid-window cut can continue canonically.
            let span = plan.instruction_count() as usize;
            if pc + span > code.len() || ops[pc..pc + span].contains(&TapeOp::Exit) {
                continue;
            }
            let Ok(index) = u16::try_from(receipts.len()) else {
                break;
            };
            receipts.push(plan);
            ops[pc] = TapeOp::Receipt { index };
        }

        // Run suffixes: consecutive logical instructions to the next effect
        // boundary, counting block terminators as their final instruction.
        let mut run_suffix = vec![NOT_ON_TAPE; code.len()];
        let mut following: u32 = 0;
        for pc in (0..code.len()).rev() {
            following = match ops[pc] {
                TapeOp::Exit => 0,
                TapeOp::Jump { .. }
                | TapeOp::JumpIf { .. }
                | TapeOp::JumpIfNot { .. }
                | TapeOp::ForLoop { .. } => 1,
                _ => following.saturating_add(1),
            };
            run_suffix[pc] = u16::try_from(following).unwrap_or(u16::MAX);
        }

        Some(Arc::new(Self {
            ops: ops.into_boxed_slice(),
            run_suffix: run_suffix.into_boxed_slice(),
            constants: constants.into_boxed_slice(),
            receipts: receipts.into_boxed_slice(),
        }))
    }
}

/// True when every listed register indexes the function's register file.
fn registers_in_bounds(function: &Function, registers: &[u8]) -> bool {
    registers
        .iter()
        .all(|register| usize::from(*register) < function.max_registers)
}

/// Absolute branch target for a verified non-negative `aux`.
fn branch_target(aux: i32) -> Option<u32> {
    u32::try_from(aux).ok()
}

fn numeric_operation(opcode: Opcode) -> Option<NumericOp> {
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

fn comparison_form(opcode: Opcode) -> Option<TapeComparison> {
    match opcode {
        Opcode::Less => Some(TapeComparison::Less),
        Opcode::LessEqual => Some(TapeComparison::LessEqual),
        Opcode::Greater => Some(TapeComparison::Greater),
        Opcode::GreaterEqual => Some(TapeComparison::GreaterEqual),
        Opcode::Equal => Some(TapeComparison::Equal),
        Opcode::NotEqual => Some(TapeComparison::NotEqual),
        _ => None,
    }
}

/// Predecode one instruction, or `Exit` when any operand fails verification.
/// Malformed operands always decline to scalar dispatch so structured errors
/// keep their current kind and PC.
fn predecode(
    program: &Program,
    function: &Function,
    instruction: Instruction,
    pc: usize,
    hot_ir_backedges: &[usize],
    constants: &mut Vec<Value>,
    constant_slots: &mut [u32],
) -> TapeOp {
    match instruction.op {
        Opcode::Move => {
            if registers_in_bounds(function, &[instruction.a, instruction.b]) {
                TapeOp::Move {
                    destination: instruction.a,
                    source: instruction.b,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::ClearLocalCell => {
            if registers_in_bounds(function, &[instruction.a]) {
                TapeOp::ClearLocalCell {
                    register: instruction.a,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::LoadConst | Opcode::LoadConstWide => {
            let constant_index = match instruction.op {
                Opcode::LoadConst => usize::from(instruction.c),
                _ => match usize::try_from(instruction.aux) {
                    Ok(index) => index,
                    Err(_) => return TapeOp::Exit,
                },
            };
            let Some(value) = primitive_constant(program, constant_index) else {
                return TapeOp::Exit;
            };
            if !registers_in_bounds(function, &[instruction.a]) {
                return TapeOp::Exit;
            }
            let Some(slot) = constant_slots.get_mut(constant_index) else {
                return TapeOp::Exit;
            };
            let tape_index = if *slot == 0 {
                let Ok(next) = u16::try_from(constants.len()) else {
                    return TapeOp::Exit;
                };
                constants.push(value);
                *slot = u32::from(next) + 1;
                next
            } else {
                (*slot - 1) as u16
            };
            TapeOp::LoadPrimitive {
                destination: instruction.a,
                constant: tape_index,
            }
        }
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::FloorDiv
        | Opcode::Mod
        | Opcode::Pow => {
            let Some(operation) = numeric_operation(instruction.op) else {
                return TapeOp::Exit;
            };
            if registers_in_bounds(function, &[instruction.a, instruction.b, instruction.c]) {
                TapeOp::Numeric {
                    operation,
                    destination: instruction.a,
                    lhs: instruction.b,
                    rhs: instruction.c,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::Neg => {
            if registers_in_bounds(function, &[instruction.a, instruction.b]) {
                TapeOp::NumericNegate {
                    destination: instruction.a,
                    source: instruction.b,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::Not => {
            if registers_in_bounds(function, &[instruction.a, instruction.b]) {
                TapeOp::Not {
                    destination: instruction.a,
                    source: instruction.b,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::Less
        | Opcode::LessEqual
        | Opcode::Greater
        | Opcode::GreaterEqual
        | Opcode::Equal
        | Opcode::NotEqual => {
            let Some(comparison) = comparison_form(instruction.op) else {
                return TapeOp::Exit;
            };
            if registers_in_bounds(function, &[instruction.a, instruction.b, instruction.c]) {
                TapeOp::Compare {
                    comparison,
                    destination: instruction.a,
                    lhs: instruction.b,
                    rhs: instruction.c,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::GetIndex => {
            if registers_in_bounds(function, &[instruction.a, instruction.b, instruction.c]) {
                TapeOp::DenseArrayGet {
                    destination: instruction.a,
                    table: instruction.b,
                    index: instruction.c,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::SetIndex => {
            if registers_in_bounds(function, &[instruction.a, instruction.b, instruction.c]) {
                TapeOp::DenseArraySet {
                    table: instruction.a,
                    index: instruction.b,
                    value: instruction.c,
                }
            } else {
                TapeOp::Exit
            }
        }
        Opcode::Jump => match branch_target(instruction.aux) {
            Some(target) => TapeOp::Jump { target },
            None => TapeOp::Exit,
        },
        Opcode::JumpIf => match branch_target(instruction.aux) {
            Some(target) if registers_in_bounds(function, &[instruction.a]) => TapeOp::JumpIf {
                condition: instruction.a,
                target,
            },
            _ => TapeOp::Exit,
        },
        Opcode::JumpIfNot => match branch_target(instruction.aux) {
            Some(target) if registers_in_bounds(function, &[instruction.a]) => TapeOp::JumpIfNot {
                condition: instruction.a,
                target,
            },
            _ => TapeOp::Exit,
        },
        Opcode::ForLoop => {
            if hot_ir_backedges.contains(&pc) {
                return TapeOp::Exit;
            }
            let Some(target) = branch_target(instruction.aux) else {
                return TapeOp::Exit;
            };
            let window_end = instruction.a.checked_add(3);
            match window_end {
                Some(visible) if registers_in_bounds(function, &[visible]) => TapeOp::ForLoop {
                    base: instruction.a,
                    target,
                },
                _ => TapeOp::Exit,
            }
        }
        _ => TapeOp::Exit,
    }
}

/// Certify and decode a nil/boolean/number constant; strings and imports
/// stay on scalar dispatch (string loads may atomize).
fn primitive_constant(program: &Program, index: usize) -> Option<Value> {
    match program.constants().get(index)? {
        Constant::Nil => Some(Value::nil()),
        Constant::Boolean(value) => Some(Value::boolean(*value)),
        Constant::Number(value) => Some(Value::number(*value)),
        Constant::String(_) | Constant::Import(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(program: &Program) -> Option<Arc<FunctionTape>> {
        FunctionTape::build(program, &program.main, &[])
    }

    #[test]
    fn tape_op_stays_compact() {
        assert!(std::mem::size_of::<TapeOp>() <= 8);
    }

    #[test]
    fn predecodes_operands_and_direct_branch_targets() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::Move, 1, 0, 0),
                    Instruction::new(Opcode::Add, 2, 0, 1),
                    Instruction::new(Opcode::Less, 3, 0, 2),
                    Instruction::with_aux(Opcode::JumpIfNot, 3, 0, 0, 6),
                    Instruction::with_aux(Opcode::Jump, 0, 0, 0, 1),
                    Instruction::new(Opcode::Return, 2, 1, 0),
                ],
                4,
            ),
            vec![Constant::Number(2.0)],
        );
        let tape = build(&program).expect("tape metadata");
        assert_eq!(
            tape.op(0),
            TapeOp::LoadPrimitive {
                destination: 0,
                constant: 0
            }
        );
        assert_eq!(tape.constant(0).as_number(), Some(2.0));
        assert_eq!(
            tape.op(1),
            TapeOp::Move {
                destination: 1,
                source: 0
            }
        );
        assert_eq!(
            tape.op(2),
            TapeOp::Numeric {
                operation: NumericOp::Add,
                destination: 2,
                lhs: 0,
                rhs: 1
            }
        );
        assert_eq!(
            tape.op(3),
            TapeOp::Compare {
                comparison: TapeComparison::Less,
                destination: 3,
                lhs: 0,
                rhs: 2
            }
        );
        assert_eq!(
            tape.op(4),
            TapeOp::JumpIfNot {
                condition: 3,
                target: 6
            }
        );
        assert_eq!(tape.op(5), TapeOp::Jump { target: 1 });
        assert_eq!(tape.op(6), TapeOp::Exit);
    }

    #[test]
    fn run_suffixes_group_effect_terminated_blocks() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::Move, 1, 0, 0),
                    Instruction::with_aux(Opcode::Jump, 0, 0, 0, 3),
                    Instruction::new(Opcode::Add, 2, 0, 1),
                    Instruction::new(Opcode::Concat, 0, 0, 1),
                    Instruction::new(Opcode::Move, 3, 2, 0),
                    Instruction::new(Opcode::Return, 3, 1, 0),
                ],
                4,
            ),
            vec![Constant::Number(2.0)],
        );
        let tape = build(&program).expect("tape metadata");
        // Block one runs through its terminating branch.
        assert_eq!(tape.run_from(0), 3);
        assert_eq!(tape.run_from(1), 2);
        assert_eq!(tape.run_from(2), 1);
        // Block two ends at the Concat effect boundary.
        assert_eq!(tape.run_from(3), 1);
        assert_eq!(tape.run_from(4), 0);
        assert_eq!(tape.run_from(5), 1);
        assert_eq!(tape.run_from(6), 0);
        assert_eq!(tape.run_from(usize::MAX), 0);
    }

    #[test]
    fn shares_predecoded_primitive_constants_by_program_index() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 1),
                    Instruction::new(Opcode::LoadConst, 2, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                3,
            ),
            vec![Constant::Number(4.0), Constant::Boolean(true)],
        );
        let tape = build(&program).expect("tape metadata");
        let TapeOp::LoadPrimitive {
            constant: first, ..
        } = tape.op(0)
        else {
            panic!("expected predecoded constant load");
        };
        let TapeOp::LoadPrimitive {
            constant: second, ..
        } = tape.op(1)
        else {
            panic!("expected predecoded constant load");
        };
        let TapeOp::LoadPrimitive {
            constant: third, ..
        } = tape.op(2)
        else {
            panic!("expected predecoded constant load");
        };
        assert_eq!(first, third);
        assert_ne!(first, second);
        assert_eq!(tape.constant(second).as_bool(), Some(true));
    }

    #[test]
    fn non_primitive_and_malformed_loads_stay_scalar() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 1),
                    Instruction::new(Opcode::LoadConst, 0, 0, 7),
                    Instruction::with_aux(Opcode::LoadConstWide, 0, 0, 0, -1),
                    Instruction::new(Opcode::Move, 0, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                1,
            ),
            vec![
                Constant::Number(1.0),
                Constant::String(b"interned string constant".to_vec()),
            ],
        );
        let tape = build(&program).expect("tape metadata");
        assert_eq!(tape.op(0), TapeOp::Exit);
        assert_eq!(tape.op(1), TapeOp::Exit);
        assert_eq!(tape.op(2), TapeOp::Exit);
        assert_eq!(tape.run_from(0), 0);
        assert_eq!(tape.run_from(1), 0);
        assert_eq!(tape.run_from(2), 0);
        assert_eq!(tape.run_from(3), 1);
    }

    #[test]
    fn out_of_bounds_registers_and_negative_targets_stay_scalar() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::Move, 4, 0, 0),
                    Instruction::new(Opcode::Move, 0, 4, 0),
                    Instruction::new(Opcode::Add, 0, 4, 1),
                    Instruction::with_aux(Opcode::Jump, 0, 0, 0, -2),
                    Instruction::with_aux(Opcode::JumpIf, 4, 0, 0, 0),
                    Instruction::with_aux(Opcode::ForLoop, 3, 0, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                4,
            ),
            Vec::new(),
        );
        assert!(build(&program).is_none());
    }

    #[test]
    fn hot_ir_backedges_stay_effect_boundaries() {
        let code = vec![
            Instruction::new(Opcode::Move, 4, 0, 0),
            Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 0),
            Instruction::new(Opcode::Return, 0, 1, 0),
        ];
        let program = Program::new(Function::new(code, 5), Vec::new());
        let tape = FunctionTape::build(&program, &program.main, &[1]).expect("tape metadata");
        assert_eq!(tape.op(1), TapeOp::Exit);
        assert_eq!(tape.op(2), TapeOp::ForLoop { base: 0, target: 0 });
        assert_eq!(tape.run_from(2), 1);
    }

    #[test]
    fn fuses_receipts_and_keeps_interior_ops_canonical() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::Move, 2, 0, 0),
                    Instruction::new(Opcode::Move, 3, 1, 0),
                    Instruction::new(Opcode::Add, 2, 2, 3),
                    Instruction::new(Opcode::ClearLocalCell, 3, 0, 0),
                    Instruction::new(Opcode::Move, 0, 2, 0),
                    Instruction::new(Opcode::ClearLocalCell, 2, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                4,
            ),
            Vec::new(),
        );
        let tape = build(&program).expect("tape metadata");
        let TapeOp::Receipt { index } = tape.op(0) else {
            panic!("expected fused receipt at the block start");
        };
        let plan = tape.receipt(index).expect("receipt plan");
        assert_eq!(plan.instruction_count(), 6);
        assert_eq!(
            tape.op(1),
            TapeOp::Move {
                destination: 3,
                source: 1
            }
        );
        assert_eq!(
            tape.op(2),
            TapeOp::Numeric {
                operation: NumericOp::Add,
                destination: 2,
                lhs: 2,
                rhs: 3
            }
        );
        assert_eq!(tape.op(3), TapeOp::ClearLocalCell { register: 3 });
        assert_eq!(tape.run_from(0), 6);
        assert_eq!(tape.run_from(1), 5);
        assert_eq!(tape.receipt(u16::MAX), None);
    }

    #[test]
    fn functions_without_tape_candidates_allocate_no_metadata() {
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1),
            Vec::new(),
        );
        assert!(build(&program).is_none());
    }

    #[test]
    fn memory_bytes_counts_every_component() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                1,
            ),
            vec![Constant::Number(2.0)],
        );
        let tape = build(&program).expect("tape metadata");
        let expected = std::mem::size_of::<FunctionTape>()
            + 2 * std::mem::size_of::<TapeOp>()
            + 2 * std::mem::size_of::<u16>()
            + std::mem::size_of::<Value>();
        assert_eq!(tape.memory_bytes(), expected);
    }
}
