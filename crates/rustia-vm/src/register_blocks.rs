//! Recognition of preverified register-lifetime instruction blocks.
//!
//! Public bytecode remains unchanged and remains the semantic authority. This
//! module recognizes exact compiler-emitted four-instruction shapes and an
//! optional two-instruction result-lifetime suffix. The executable tape fuses
//! a recognized plan into one operation at its start PC; guard misses execute
//! the first unchanged public instruction instead.

use crate::bytecode::{Function, Opcode};

const REGISTER_BLOCK_INSTRUCTION_COUNT: usize = 4;
const REGISTER_LIFETIME_BLOCK_INSTRUCTION_COUNT: usize = 6;

/// A preverified four- or six-instruction register block.
///
/// The represented bytecode is exactly:
///
/// ```text
/// Move           target, lhs
/// Move           temporary, rhs
/// operation      target, target, temporary
/// ClearLocalCell temporary
/// [Move           destination, target]
/// [ClearLocalCell target]
/// ```
///
/// `operation` is one of the arithmetic binary operations or `GetIndex`.
/// The two destination registers do not alias either source register. The two
/// source registers may be equal, which preserves common expressions such as
/// `value + value`. When present, the optional lifetime destination is distinct
/// from both temporary destinations but may alias either or both sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegisterBlockPlan {
    operation: Opcode,
    target_register: u8,
    lhs_register: u8,
    temporary_register: u8,
    rhs_register: u8,
    lifetime_destination_register: Option<u8>,
}

impl RegisterBlockPlan {
    /// Number of logical bytecode instructions represented by this plan.
    #[inline]
    pub(crate) const fn instruction_count(self) -> u64 {
        if self.lifetime_destination_register.is_some() {
            REGISTER_LIFETIME_BLOCK_INSTRUCTION_COUNT as u64
        } else {
            REGISTER_BLOCK_INSTRUCTION_COUNT as u64
        }
    }

    #[inline]
    pub(crate) const fn operation(self) -> Opcode {
        self.operation
    }

    #[inline]
    pub(crate) const fn target_register(self) -> u8 {
        self.target_register
    }

    #[inline]
    pub(crate) const fn lhs_register(self) -> u8 {
        self.lhs_register
    }

    #[inline]
    pub(crate) const fn temporary_register(self) -> u8 {
        self.temporary_register
    }

    #[inline]
    pub(crate) const fn rhs_register(self) -> u8 {
        self.rhs_register
    }

    /// Destination of the optional `Move destination, target` lifetime suffix.
    #[inline]
    pub(crate) const fn lifetime_destination_register(self) -> Option<u8> {
        self.lifetime_destination_register
    }
}

/// Recognize the preverified register block beginning at `pc`.
///
/// Every instruction lane, auxiliary word, register bound, and relevant
/// register distinction is verified. Malformed and near-miss shapes remain on
/// scalar dispatch.
#[inline]
pub(crate) fn recognize_register_block(
    function: &Function,
    pc: usize,
) -> Option<RegisterBlockPlan> {
    let lhs_move = function.code.get(pc)?;
    let rhs_move = function.code.get(pc + 1)?;
    let operation = function.code.get(pc + 2)?;
    let clear = function.code.get(pc + 3)?;

    if lhs_move.op != Opcode::Move || lhs_move.c != 0 || lhs_move.aux != 0 {
        return None;
    }
    if rhs_move.op != Opcode::Move || rhs_move.c != 0 || rhs_move.aux != 0 {
        return None;
    }
    if !is_supported_operation(operation.op) || operation.aux != 0 {
        return None;
    }
    if clear.op != Opcode::ClearLocalCell || clear.b != 0 || clear.c != 0 || clear.aux != 0 {
        return None;
    }

    let target_register = lhs_move.a;
    let lhs_register = lhs_move.b;
    let temporary_register = rhs_move.a;
    let rhs_register = rhs_move.b;

    if operation.a != target_register
        || operation.b != target_register
        || operation.c != temporary_register
        || clear.a != temporary_register
    {
        return None;
    }

    if [
        target_register,
        lhs_register,
        temporary_register,
        rhs_register,
    ]
    .into_iter()
    .any(|register| usize::from(register) >= function.max_registers)
    {
        return None;
    }

    // These are the distinctions guaranteed by the compiler's temporary
    // allocation. They also make both original source operands stable if a
    // future executor bypasses the two setup Moves. `lhs == rhs` remains safe.
    if target_register == temporary_register
        || target_register == lhs_register
        || target_register == rhs_register
        || temporary_register == lhs_register
        || temporary_register == rhs_register
    {
        return None;
    }

    let lifetime_destination_register = recognize_lifetime_suffix(
        function,
        pc + REGISTER_BLOCK_INSTRUCTION_COUNT,
        target_register,
        temporary_register,
    );

    Some(RegisterBlockPlan {
        operation: operation.op,
        target_register,
        lhs_register,
        temporary_register,
        rhs_register,
        lifetime_destination_register,
    })
}

#[inline]
fn recognize_lifetime_suffix(
    function: &Function,
    pc: usize,
    target_register: u8,
    temporary_register: u8,
) -> Option<u8> {
    let destination_move = function.code.get(pc)?;
    let target_clear = function.code.get(pc + 1)?;
    if destination_move.op != Opcode::Move
        || destination_move.b != target_register
        || destination_move.c != 0
        || destination_move.aux != 0
        || target_clear.op != Opcode::ClearLocalCell
        || target_clear.a != target_register
        || target_clear.b != 0
        || target_clear.c != 0
        || target_clear.aux != 0
    {
        return None;
    }

    let destination_register = destination_move.a;
    (usize::from(destination_register) < function.max_registers
        && destination_register != target_register
        && destination_register != temporary_register)
        .then_some(destination_register)
}

#[inline]
const fn is_supported_operation(operation: Opcode) -> bool {
    matches!(
        operation,
        Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::FloorDiv
            | Opcode::Mod
            | Opcode::Pow
            | Opcode::GetIndex
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;

    fn function(code: Vec<Instruction>, max_registers: usize) -> Function {
        Function::new(code, max_registers)
    }

    fn block(operation: Opcode, target: u8, lhs: u8, temporary: u8, rhs: u8) -> Vec<Instruction> {
        vec![
            Instruction::new(Opcode::Move, target, lhs, 0),
            Instruction::new(Opcode::Move, temporary, rhs, 0),
            Instruction::new(operation, target, target, temporary),
            Instruction::new(Opcode::ClearLocalCell, temporary, 0, 0),
        ]
    }

    fn lifetime_block(
        operation: Opcode,
        target: u8,
        lhs: u8,
        temporary: u8,
        rhs: u8,
        destination: u8,
    ) -> Vec<Instruction> {
        let mut code = block(operation, target, lhs, temporary, rhs);
        code.extend([
            Instruction::new(Opcode::Move, destination, target, 0),
            Instruction::new(Opcode::ClearLocalCell, target, 0, 0),
        ]);
        code
    }

    #[test]
    fn plan_payload_stays_compact() {
        assert!(std::mem::size_of::<RegisterBlockPlan>() <= 8);
    }

    #[test]
    fn recognizes_every_supported_operation_and_reports_all_fields() {
        for operation in [
            Opcode::Add,
            Opcode::Sub,
            Opcode::Mul,
            Opcode::Div,
            Opcode::FloorDiv,
            Opcode::Mod,
            Opcode::Pow,
            Opcode::GetIndex,
        ] {
            let base_function = function(block(operation, 4, 1, 5, 2), 6);
            let plan = recognize_register_block(&base_function, 0).expect("complete block");

            assert_eq!(plan.instruction_count(), 4);
            assert_eq!(plan.operation(), operation);
            assert_eq!(plan.target_register(), 4);
            assert_eq!(plan.lhs_register(), 1);
            assert_eq!(plan.temporary_register(), 5);
            assert_eq!(plan.rhs_register(), 2);
            assert_eq!(plan.lifetime_destination_register(), None);

            let function = function(lifetime_block(operation, 4, 1, 5, 2, 1), 6);
            let plan = recognize_register_block(&function, 0).expect("lifetime block");
            assert_eq!(plan.instruction_count(), 6);
            assert_eq!(plan.lifetime_destination_register(), Some(1));
        }
    }

    #[test]
    fn does_not_recognize_without_a_complete_supported_shape() {
        assert!(recognize_register_block(&function(Vec::new(), 0), 0).is_none());
        assert!(recognize_register_block(
            &function(vec![Instruction::new(Opcode::Return, 0, 0, 0)], 1),
            0
        )
        .is_none());

        let mut missing_clear = block(Opcode::Add, 4, 1, 5, 2);
        missing_clear.pop();
        assert!(recognize_register_block(&function(missing_clear, 6), 0).is_none());

        let unsupported = block(Opcode::Equal, 4, 1, 5, 2);
        assert!(recognize_register_block(&function(unsupported, 6), 0).is_none());
    }

    #[test]
    fn rejects_out_of_bounds_registers_in_every_role() {
        for (target, lhs, temporary, rhs) in
            [(4, 1, 2, 3), (0, 4, 2, 3), (0, 1, 4, 3), (0, 1, 2, 4)]
        {
            let function = function(block(Opcode::Mul, target, lhs, temporary, rhs), 4);
            assert!(
                recognize_register_block(&function, 0).is_none(),
                "unexpected plan for target={target}, lhs={lhs}, temporary={temporary}, rhs={rhs}"
            );
        }
    }

    #[test]
    fn rejects_destination_aliases_but_allows_a_shared_source() {
        for (target, lhs, temporary, rhs) in [
            (0, 1, 0, 3),
            (0, 0, 2, 3),
            (0, 1, 2, 0),
            (0, 2, 2, 3),
            (0, 1, 2, 2),
        ] {
            let function = function(block(Opcode::Add, target, lhs, temporary, rhs), 4);
            assert!(
                recognize_register_block(&function, 0).is_none(),
                "unexpected aliased plan for target={target}, lhs={lhs}, temporary={temporary}, rhs={rhs}"
            );
        }

        let base_function = function(block(Opcode::Add, 0, 1, 2, 1), 3);
        let plan = recognize_register_block(&base_function, 0).expect("shared source is safe");
        assert_eq!(plan.lhs_register(), plan.rhs_register());

        let function = function(lifetime_block(Opcode::Add, 1, 0, 2, 0, 0), 3);
        let plan =
            recognize_register_block(&function, 0).expect("shared source/destination is safe");
        assert_eq!(plan.lhs_register(), plan.rhs_register());
        assert_eq!(plan.lifetime_destination_register(), Some(0));
        assert_eq!(plan.instruction_count(), 6);
    }

    #[test]
    fn malformed_lifetime_suffixes_retain_the_four_instruction_plan() {
        let exact = lifetime_block(Opcode::Add, 4, 1, 5, 2, 1);
        let mut near_misses = Vec::new();

        let mut code = exact.clone();
        code[4].op = Opcode::LoadConst;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].b = 3;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].c = 1;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].aux = 1;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].a = 4;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].a = 5;
        near_misses.push(code);
        let mut code = exact.clone();
        code[4].a = 6;
        near_misses.push(code);

        let mut code = exact.clone();
        code[5].op = Opcode::SetLocalCell;
        near_misses.push(code);
        let mut code = exact.clone();
        code[5].a = 3;
        near_misses.push(code);
        let mut code = exact.clone();
        code[5].b = 1;
        near_misses.push(code);
        let mut code = exact.clone();
        code[5].c = 1;
        near_misses.push(code);
        let mut code = exact;
        code[5].aux = 1;
        near_misses.push(code);

        for (index, code) in near_misses.into_iter().enumerate() {
            let plan = recognize_register_block(&function(code, 6), 0)
                .unwrap_or_else(|| panic!("base plan missing for suffix near miss {index}"));
            assert_eq!(plan.instruction_count(), 4, "suffix near miss {index}");
            assert_eq!(plan.lifetime_destination_register(), None);
        }
    }

    #[test]
    fn rejects_near_misses_in_every_instruction_position() {
        let mut near_misses = Vec::new();

        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[0].op = Opcode::LoadConst;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[0].c = 1;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[0].aux = 1;
        near_misses.push(code);

        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[1].op = Opcode::LoadConst;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[1].c = 1;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[1].aux = 1;
        near_misses.push(code);

        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[2].a = 3;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[2].b = 3;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[2].c = 3;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[2].aux = 1;
        near_misses.push(code);

        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[3].op = Opcode::SetLocalCell;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[3].a = 4;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[3].b = 1;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[3].c = 1;
        near_misses.push(code);
        let mut code = block(Opcode::Add, 4, 1, 5, 2);
        code[3].aux = 1;
        near_misses.push(code);

        for (index, code) in near_misses.into_iter().enumerate() {
            assert!(
                recognize_register_block(&function(code, 6), 0).is_none(),
                "near miss {index} was recognized"
            );
        }
    }

    #[test]
    fn recognizes_plans_at_interior_program_counters_only_at_exact_starts() {
        let mut code = block(Opcode::Sub, 4, 0, 5, 1);
        code.push(Instruction::new(Opcode::Return, 0, 0, 0));
        code.extend(block(Opcode::GetIndex, 6, 2, 7, 3));

        let function = function(code, 8);
        let first = recognize_register_block(&function, 0).expect("first plan");
        assert_eq!(first.operation(), Opcode::Sub);
        assert_eq!(first.instruction_count(), 4);

        for pc in 1..5 {
            assert!(recognize_register_block(&function, pc).is_none());
        }

        let second = recognize_register_block(&function, 5).expect("second plan");
        assert_eq!(second.operation(), Opcode::GetIndex);
        assert_eq!(second.instruction_count(), 4);

        assert!(recognize_register_block(&function, usize::MAX).is_none());
    }
}
