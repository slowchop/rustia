//! Effect-certified regions for the common interpreter fast path.
//!
//! A region contains only instructions whose successful fast path cannot
//! allocate, invoke guest code, or trigger collection. Guarded operations may
//! decline before their current instruction mutates observable state; the VM
//! then resumes that instruction through the semantic interpreter. This keeps
//! public bytecode authoritative while amortizing policy checks and dispatch
//! across consecutive hot instructions.

use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};

const OPCODE_TAG_SPACE: usize = u8::MAX as usize + 1;
const NO_REGION: u16 = 0;

/// The strongest observable effect admitted by an instruction's successful
/// common fast path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegionEffect {
    /// Loads an immutable nil/boolean/number constant without atomization.
    PrimitiveConstant,
    /// Reads or writes registers/cell bookkeeping only.
    RegisterOnly,
    /// Computes an unboxed numeric result after type guards.
    GuardedNumeric,
    /// Reads a present dense-array slot after table/index guards.
    GuardedArrayRead,
    /// Writes an existing dense-array slot after all guards succeed.
    GuardedArrayWrite,
}

/// A preverified consecutive region. The code remains in `Function::code`; the
/// side metadata is deliberately only a start and length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommonRegion {
    pub(crate) start_pc: usize,
    instruction_count: u16,
}

impl CommonRegion {
    #[inline]
    pub(crate) const fn instruction_count(self) -> u64 {
        self.instruction_count as u64
    }

    #[inline]
    pub(crate) const fn end_pc(self) -> usize {
        self.start_pc + self.instruction_count as usize
    }
}

/// Single source of truth for opcodes whose guarded fast implementation is
/// eligible for region execution.
pub(crate) const COMMON_REGION_CANDIDATE: [bool; OPCODE_TAG_SPACE] = {
    let mut candidates = [false; OPCODE_TAG_SPACE];
    candidates[Opcode::Add as usize] = true;
    candidates[Opcode::Sub as usize] = true;
    candidates[Opcode::Mul as usize] = true;
    candidates[Opcode::Div as usize] = true;
    candidates[Opcode::FloorDiv as usize] = true;
    candidates[Opcode::Mod as usize] = true;
    candidates[Opcode::Pow as usize] = true;
    candidates[Opcode::GetIndex as usize] = true;
    candidates[Opcode::SetIndex as usize] = true;
    candidates[Opcode::Move as usize] = true;
    candidates[Opcode::ClearLocalCell as usize] = true;
    candidates
};

/// Cheap opcode-only prefilter before consulting a loaded program's region
/// map. Constant loads are candidates only when their descriptor resolves to
/// a primitive; `region_effect` performs that exact certification.
pub(crate) const REGION_OPCODE_CANDIDATE: [bool; OPCODE_TAG_SPACE] = {
    let mut candidates = COMMON_REGION_CANDIDATE;
    candidates[Opcode::LoadConst as usize] = true;
    candidates[Opcode::LoadConstWide as usize] = true;
    candidates
};

const fn common_fast_path_effect(opcode: Opcode) -> Option<RegionEffect> {
    match opcode {
        Opcode::Move | Opcode::ClearLocalCell => Some(RegionEffect::RegisterOnly),
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::FloorDiv
        | Opcode::Mod
        | Opcode::Pow => Some(RegionEffect::GuardedNumeric),
        Opcode::GetIndex => Some(RegionEffect::GuardedArrayRead),
        Opcode::SetIndex => Some(RegionEffect::GuardedArrayWrite),
        _ => None,
    }
}

pub(crate) fn region_effect(program: &Program, instruction: Instruction) -> Option<RegionEffect> {
    let constant_index = match instruction.op {
        Opcode::LoadConst => Some(usize::from(instruction.c)),
        Opcode::LoadConstWide if instruction.aux >= 0 => Some(instruction.aux as usize),
        Opcode::LoadConstWide => None,
        _ => return common_fast_path_effect(instruction.op),
    }?;
    matches!(
        program.constants().get(constant_index),
        Some(Constant::Nil | Constant::Boolean(_) | Constant::Number(_))
    )
    .then_some(RegionEffect::PrimitiveConstant)
}

/// Build a two-byte-per-PC suffix-length map only when the function has a
/// region of at least two consecutive eligible instructions. Every interior
/// PC with two or more operations remaining is an entry, so a temporary fuel,
/// GC, or allocation-debt boundary can resume region execution later in the
/// same run. Long runs are split at the representable length; public bytecode
/// and jump targets are unchanged.
pub(crate) fn build_common_region_map(
    program: &Program,
    function: &Function,
) -> Option<Box<[u16]>> {
    let mut by_pc = vec![NO_REGION; function.code.len()];
    let mut found = false;
    let mut pc = 0;
    while pc < function.code.len() {
        if region_effect(program, function.code[pc]).is_none() {
            pc += 1;
            continue;
        }

        let start = pc;
        let limit = start
            .saturating_add(u16::MAX as usize)
            .min(function.code.len());
        while pc < limit && region_effect(program, function.code[pc]).is_some() {
            pc += 1;
        }
        let count = pc - start;
        if count >= 2 {
            for offset in 0..count - 1 {
                by_pc[start + offset] = (count - offset) as u16;
            }
            found = true;
        }
    }
    found.then(|| by_pc.into_boxed_slice())
}

#[inline]
pub(crate) fn common_region_at(by_pc: Option<&[u16]>, pc: usize) -> Option<CommonRegion> {
    let instruction_count = *by_pc?.get(pc)?;
    (instruction_count != NO_REGION).then_some(CommonRegion {
        start_pc: pc,
        instruction_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;

    #[test]
    fn effect_certificates_and_candidate_table_agree() {
        for (tag, candidate) in COMMON_REGION_CANDIDATE.iter().copied().enumerate() {
            let certified = match tag {
                value if value == Opcode::Add as usize => {
                    common_fast_path_effect(Opcode::Add).is_some()
                }
                value if value == Opcode::Sub as usize => {
                    common_fast_path_effect(Opcode::Sub).is_some()
                }
                value if value == Opcode::Mul as usize => {
                    common_fast_path_effect(Opcode::Mul).is_some()
                }
                value if value == Opcode::Div as usize => {
                    common_fast_path_effect(Opcode::Div).is_some()
                }
                value if value == Opcode::FloorDiv as usize => {
                    common_fast_path_effect(Opcode::FloorDiv).is_some()
                }
                value if value == Opcode::Mod as usize => {
                    common_fast_path_effect(Opcode::Mod).is_some()
                }
                value if value == Opcode::Pow as usize => {
                    common_fast_path_effect(Opcode::Pow).is_some()
                }
                value if value == Opcode::GetIndex as usize => {
                    common_fast_path_effect(Opcode::GetIndex).is_some()
                }
                value if value == Opcode::SetIndex as usize => {
                    common_fast_path_effect(Opcode::SetIndex).is_some()
                }
                value if value == Opcode::Move as usize => {
                    common_fast_path_effect(Opcode::Move).is_some()
                }
                value if value == Opcode::ClearLocalCell as usize => {
                    common_fast_path_effect(Opcode::ClearLocalCell).is_some()
                }
                _ => false,
            };
            assert_eq!(candidate, certified, "opcode tag {tag}");
        }
    }

    #[test]
    fn region_map_keeps_only_consecutive_runs() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::Move, 0, 1, 0),
                    Instruction::new(Opcode::ClearLocalCell, 1, 0, 0),
                    Instruction::new(Opcode::Jump, 0, 0, 0),
                    Instruction::new(Opcode::Add, 0, 1, 2),
                    Instruction::new(Opcode::LoadConst, 2, 0, 0),
                    Instruction::new(Opcode::GetIndex, 0, 1, 2),
                    Instruction::new(Opcode::SetIndex, 0, 1, 2),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                3,
            ),
            vec![Constant::Number(1.0)],
        );
        let map = build_common_region_map(&program, &program.main).unwrap();
        assert_eq!(common_region_at(Some(&map), 0).unwrap().end_pc(), 2);
        assert_eq!(common_region_at(Some(&map), 3).unwrap().end_pc(), 7);
        assert_eq!(common_region_at(Some(&map), 4).unwrap().end_pc(), 7);
        assert_eq!(common_region_at(Some(&map), 1), None);
        assert_eq!(common_region_at(Some(&map), 2), None);
        assert_eq!(common_region_at(Some(&map), 7), None);
    }

    #[test]
    fn isolated_candidate_does_not_allocate_region_metadata() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::Move, 0, 1, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                2,
            ),
            Vec::new(),
        );
        assert!(build_common_region_map(&program, &program.main).is_none());
    }

    #[test]
    fn only_primitive_constant_loads_join_regions() {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::Move, 0, 1, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 0),
                    Instruction::new(Opcode::LoadConst, 2, 0, 1),
                    Instruction::new(Opcode::Move, 3, 2, 0),
                ],
                4,
            ),
            vec![
                Constant::Number(1.0),
                Constant::String(b"long string value".to_vec()),
            ],
        );
        let map = build_common_region_map(&program, &program.main).unwrap();
        assert_eq!(common_region_at(Some(&map), 0).unwrap().end_pc(), 2);
        assert_eq!(common_region_at(Some(&map), 2), None);
    }
}
