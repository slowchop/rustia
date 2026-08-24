//! Private, preverified interpreter superinstruction metadata.
//!
//! Public bytecode remains the semantic authority. This module recognizes only
//! instruction runs whose effects can be executed in bulk after the VM has
//! admitted the complete logical span. Any malformed or unsupported shape is
//! left unmarked so ordinary instruction dispatch retains its exact error and
//! policy behavior.

use std::ops::Range;

use crate::bytecode::{Function, Opcode};

const NO_CLEAR_SPAN: u16 = 0;
const CLEAR_SPAN_LOW_REGISTER_MASK: u16 = u8::MAX as u16;
const CLEAR_SPAN_COUNT_SHIFT: u32 = u8::BITS;

/// A consecutive, monotonic run of distinct `ClearLocalCell` instructions.
///
/// The compiler emits scope cleanup from the lowest register up and temporary
/// register-window cleanup from the highest register down. With collection,
/// coverage, and other policy boundaries excluded by the caller, clearing
/// distinct registers commutes and can be implemented as one contiguous slice
/// operation. `register_count` is also the span's logical bytecode instruction
/// count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ClearSpan {
    pub(crate) start_pc: usize,
    low_register: u8,
    register_count: u16,
}

impl ClearSpan {
    /// Number of logical `ClearLocalCell` instructions represented by the
    /// span. This is suitable for instruction accounting and policy admission.
    #[inline]
    pub(crate) const fn instruction_count(self) -> u64 {
        self.register_count as u64
    }

    /// First PC after the represented bytecode instructions.
    #[inline]
    pub(crate) const fn end_pc(self) -> usize {
        self.start_pc + self.register_count as usize
    }

    /// Contiguous register window cleared by the span, in ascending slice
    /// order regardless of source-instruction order.
    #[inline]
    pub(crate) fn register_range(self) -> Range<usize> {
        let start = usize::from(self.low_register);
        start..start + usize::from(self.register_count)
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn low_register(self) -> u8 {
        self.low_register
    }

    #[inline]
    #[cfg(test)]
    pub(crate) const fn high_register(self) -> u8 {
        (self.low_register as u16 + self.register_count - 1) as u8
    }
}

/// Build compact, two-byte-per-PC ClearSpan metadata only when at least one
/// run contains two or more instructions.
///
/// Every interior PC with at least two clears remaining receives a suffix
/// entry. This lets execution resume into the same run after a temporary fuel,
/// collection, allocation-debt, or coverage boundary forced earlier PCs down
/// the scalar path.
///
/// A recognized run must:
///
/// - contain only `ClearLocalCell`;
/// - name registers within `Function::max_registers`;
/// - ascend or descend by exactly one register at each PC without changing
///   direction; and
/// - contain at least two instructions.
///
/// Gaps, duplicates, direction changes, and out-of-bounds operands terminate a
/// run and remain on ordinary bytecode dispatch. A complete 0-to-255 or
/// 255-to-0 run is representable: the compact entry stores `count - 1` in one
/// byte.
pub(crate) fn build_clear_span_map(function: &Function) -> Option<Box<[u16]>> {
    let mut by_pc = vec![NO_CLEAR_SPAN; function.code.len()];
    let mut found = false;
    let mut pc = 0;

    while pc < function.code.len() {
        let Some(first_register) = clear_register(function, pc) else {
            pc += 1;
            continue;
        };

        let start_pc = pc;
        let mut last_register = first_register;
        let mut direction = None;
        pc += 1;

        while let Some(register) = clear_register(function, pc) {
            let next_direction = if last_register.checked_add(1) == Some(register) {
                ClearDirection::Ascending
            } else if register.checked_add(1) == Some(last_register) {
                ClearDirection::Descending
            } else {
                break;
            };
            if direction.is_some_and(|direction| direction != next_direction) {
                break;
            }
            direction = Some(next_direction);
            last_register = register;
            pc += 1;
        }

        let register_count = pc - start_pc;
        if register_count < 2 {
            continue;
        }

        // A u8 register file permits at most the complete 255..=0 run.
        debug_assert!(register_count <= usize::from(u8::MAX) + 1);
        for offset in 0..register_count - 1 {
            let suffix_count = register_count - offset;
            let suffix_first = function.code[start_pc + offset].a;
            let suffix_last = function.code[pc - 1].a;
            let low_register = suffix_first.min(suffix_last);
            by_pc[start_pc + offset] = encode_clear_span(low_register, suffix_count);
        }
        found = true;
    }

    found.then(|| by_pc.into_boxed_slice())
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ClearDirection {
    Ascending,
    Descending,
}

/// Resolve the preverified ClearSpan beginning at `pc`.
#[inline]
pub(crate) fn clear_span_at(by_pc: Option<&[u16]>, pc: usize) -> Option<ClearSpan> {
    let encoded = *by_pc?.get(pc)?;
    if encoded == NO_CLEAR_SPAN {
        return None;
    }
    let low_register = (encoded & CLEAR_SPAN_LOW_REGISTER_MASK) as u8;
    let register_count = (encoded >> CLEAR_SPAN_COUNT_SHIFT) + 1;
    Some(ClearSpan {
        start_pc: pc,
        low_register,
        register_count,
    })
}

#[inline]
fn clear_register(function: &Function, pc: usize) -> Option<u8> {
    let instruction = function.code.get(pc)?;
    (instruction.op == Opcode::ClearLocalCell
        && usize::from(instruction.a) < function.max_registers)
        .then_some(instruction.a)
}

#[inline]
fn encode_clear_span(low_register: u8, register_count: usize) -> u16 {
    debug_assert!((2..=usize::from(u8::MAX) + 1).contains(&register_count));
    let count_minus_one = (register_count - 1) as u16;
    (count_minus_one << CLEAR_SPAN_COUNT_SHIFT) | u16::from(low_register)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;

    fn clear(register: u8) -> Instruction {
        Instruction::new(Opcode::ClearLocalCell, register, 0, 0)
    }

    fn function(code: Vec<Instruction>, max_registers: usize) -> Function {
        Function::new(code, max_registers)
    }

    #[test]
    fn maps_complete_descending_runs_and_interior_suffixes() {
        let function = function(
            vec![
                Instruction::new(Opcode::Move, 0, 1, 0),
                clear(5),
                clear(4),
                clear(3),
                clear(2),
                Instruction::new(Opcode::Return, 0, 0, 0),
            ],
            6,
        );
        let map = build_clear_span_map(&function).expect("clear-span metadata");

        assert_eq!(clear_span_at(Some(&map), 0), None);
        assert_eq!(
            clear_span_at(Some(&map), 1),
            Some(ClearSpan {
                start_pc: 1,
                low_register: 2,
                register_count: 4,
            })
        );
        assert_eq!(
            clear_span_at(Some(&map), 2),
            Some(ClearSpan {
                start_pc: 2,
                low_register: 2,
                register_count: 3,
            })
        );
        assert_eq!(
            clear_span_at(Some(&map), 3),
            Some(ClearSpan {
                start_pc: 3,
                low_register: 2,
                register_count: 2,
            })
        );
        assert_eq!(clear_span_at(Some(&map), 4), None);
        assert_eq!(clear_span_at(Some(&map), 5), None);
    }

    #[test]
    fn span_reports_logical_count_end_pc_and_contiguous_register_window() {
        let span = ClearSpan {
            start_pc: 9,
            low_register: 17,
            register_count: 4,
        };
        assert_eq!(span.instruction_count(), 4);
        assert_eq!(span.end_pc(), 13);
        assert_eq!(span.low_register(), 17);
        assert_eq!(span.high_register(), 20);
        assert_eq!(span.register_range(), 17..21);
    }

    #[test]
    fn does_not_allocate_metadata_for_isolated_clears() {
        let function = function(
            vec![
                clear(3),
                Instruction::new(Opcode::Move, 0, 1, 0),
                clear(2),
                Instruction::new(Opcode::Return, 0, 0, 0),
            ],
            4,
        );
        assert!(build_clear_span_map(&function).is_none());
    }

    #[test]
    fn maps_complete_ascending_runs_and_interior_suffixes() {
        let function = function(vec![clear(2), clear(3), clear(4), clear(5)], 6);
        let map = build_clear_span_map(&function).expect("ascending clear-span metadata");

        let complete = clear_span_at(Some(&map), 0).expect("complete ascending span");
        assert_eq!(complete.low_register(), 2);
        assert_eq!(complete.high_register(), 5);
        assert_eq!(complete.instruction_count(), 4);
        assert_eq!(complete.register_range(), 2..6);

        let suffix = clear_span_at(Some(&map), 1).expect("ascending suffix");
        assert_eq!(suffix.low_register(), 3);
        assert_eq!(suffix.high_register(), 5);
        assert_eq!(suffix.instruction_count(), 3);
        assert_eq!(suffix.register_range(), 3..6);

        assert_eq!(clear_span_at(Some(&map), 2).unwrap().register_range(), 4..6);
        assert_eq!(clear_span_at(Some(&map), 3), None);
    }

    #[test]
    fn rejects_gaps_duplicates_and_direction_changes() {
        for registers in [[5, 3], [5, 5]] {
            let function = function(registers.into_iter().map(clear).collect(), 6);
            assert!(
                build_clear_span_map(&function).is_none(),
                "unexpected span for {registers:?}"
            );
        }

        for registers in [[3, 4, 3], [4, 3, 4]] {
            let function = function(registers.into_iter().map(clear).collect(), 6);
            let map =
                build_clear_span_map(&function).expect("initial monotonic pair remains valid");
            assert_eq!(
                clear_span_at(Some(&map), 0)
                    .expect("initial pair")
                    .instruction_count(),
                2
            );
            assert_eq!(clear_span_at(Some(&map), 1), None);
        }
    }

    #[test]
    fn malformed_registers_stay_on_scalar_dispatch() {
        let malformed = function(vec![clear(3), clear(2), clear(1), clear(0)], 3);
        let map = build_clear_span_map(&malformed).expect("valid suffix remains recognizable");

        // Register 3 is out of bounds and must retain the scalar interpreter's
        // RegisterOutOfBounds error at PC 0.
        assert_eq!(clear_span_at(Some(&map), 0), None);
        assert_eq!(
            clear_span_at(Some(&map), 1),
            Some(ClearSpan {
                start_pc: 1,
                low_register: 0,
                register_count: 3,
            })
        );

        let wholly_invalid = function(vec![clear(1), clear(0)], 0);
        assert!(build_clear_span_map(&wholly_invalid).is_none());
    }

    #[test]
    fn separate_runs_keep_independent_low_registers() {
        let function = function(
            vec![
                clear(7),
                clear(6),
                Instruction::new(Opcode::Jump, 0, 0, 0),
                clear(3),
                clear(2),
                clear(1),
            ],
            8,
        );
        let map = build_clear_span_map(&function).expect("two clear spans");

        assert_eq!(clear_span_at(Some(&map), 0).unwrap().register_range(), 6..8);
        assert_eq!(clear_span_at(Some(&map), 1), None);
        assert_eq!(clear_span_at(Some(&map), 2), None);
        assert_eq!(clear_span_at(Some(&map), 3).unwrap().register_range(), 1..4);
        assert_eq!(clear_span_at(Some(&map), 4).unwrap().register_range(), 1..3);
        assert_eq!(clear_span_at(Some(&map), 5), None);
    }

    #[test]
    fn compact_encoding_supports_the_complete_u8_register_file() {
        let code = (0..=u8::MAX).rev().map(clear).collect();
        let descending = function(code, usize::from(u8::MAX) + 1);
        let map = build_clear_span_map(&descending).expect("complete register-file span");

        let complete = clear_span_at(Some(&map), 0).expect("complete span entry");
        assert_eq!(complete.low_register(), 0);
        assert_eq!(complete.high_register(), u8::MAX);
        assert_eq!(complete.instruction_count(), 256);
        assert_eq!(complete.register_range(), 0..256);
        assert_eq!(complete.end_pc(), 256);

        let final_pair = clear_span_at(Some(&map), 254).expect("final suffix pair");
        assert_eq!(final_pair.low_register(), 0);
        assert_eq!(final_pair.instruction_count(), 2);
        assert_eq!(final_pair.register_range(), 0..2);
        assert_eq!(clear_span_at(Some(&map), 255), None);

        let ascending = function((0..=u8::MAX).map(clear).collect(), usize::from(u8::MAX) + 1);
        let map = build_clear_span_map(&ascending).expect("complete ascending register-file span");
        let complete = clear_span_at(Some(&map), 0).expect("complete ascending span entry");
        assert_eq!(complete.low_register(), 0);
        assert_eq!(complete.high_register(), u8::MAX);
        assert_eq!(complete.instruction_count(), 256);
        assert_eq!(complete.register_range(), 0..256);
    }

    #[test]
    fn accessor_rejects_missing_truncated_and_out_of_bounds_maps() {
        assert_eq!(clear_span_at(None, 0), None);
        assert_eq!(clear_span_at(Some(&[]), 0), None);
        assert_eq!(clear_span_at(Some(&[NO_CLEAR_SPAN]), 0), None);
        assert_eq!(clear_span_at(Some(&[encode_clear_span(2, 3)]), 1), None);
    }
}
