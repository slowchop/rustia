//! Static per-instruction operand and effect facts for lowered bytecode.
//!
//! This module is the single source of truth for what one production
//! instruction may read, must write, and how it can leave the current basic
//! block. [`crate::liveness`] consumes these facts to build the C2.3
//! control-flow/liveness analysis (`design/58-vm-core2.md` §C2.3), and the
//! planned whole-function executable tape consumes the same facts through
//! [`crate::liveness::BlockReceipt`].
//!
//! # Soundness direction
//!
//! The facts are deliberately asymmetric so downstream liveness is always
//! conservative:
//!
//! - **Reads over-approximate.** Every register the semantic interpreter can
//!   read on any path is included. Exotic shapes that are not worth modeling
//!   precisely set [`InstructionFacts::reads_all_registers`] instead.
//! - **Definite writes under-approximate.** `InstructionFacts::writes`
//!   contains only registers written on *every* non-throwing path before the
//!   instruction completes or branches. Writes that depend on a runtime
//!   condition (loop continuation, result-count adjustment) appear only in
//!   [`InstructionFacts::conditional_writes`], which liveness never uses as a
//!   kill.
//! - **Effect flags over-approximate.** `may_enter_guest`, `may_throw`, and
//!   `may_allocate` are static worst cases: `Add` may enter an `__add`
//!   metamethod even though the common numeric path does not.
//!
//! A wrong fact here is an analysis bug, never a VM behavior change: nothing
//! in the execution path consults this module. The dispatch-agreement tests in
//! `vm.rs` and the liveness differential tests pin the model to the
//! interpreter.

use crate::bytecode::{Function, Instruction, Opcode, Program, UpvalueDescriptor};

const REGISTER_SPACE: usize = 256;
const REGISTER_WORDS: usize = REGISTER_SPACE / 64;

/// Dense set of VM register indices (`0..=255`).
///
/// The register file is at most 256 slots by construction (`u8` operands), so
/// four words cover every function. Deterministic by construction: iteration
/// is ascending register order.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct RegisterSet {
    words: [u64; REGISTER_WORDS],
}

impl RegisterSet {
    pub const EMPTY: Self = Self {
        words: [0; REGISTER_WORDS],
    };

    #[inline]
    pub fn insert(&mut self, register: u8) {
        self.words[usize::from(register) / 64] |= 1u64 << (usize::from(register) % 64);
    }

    #[inline]
    pub fn remove(&mut self, register: u8) {
        self.words[usize::from(register) / 64] &= !(1u64 << (usize::from(register) % 64));
    }

    #[inline]
    pub fn contains(self, register: u8) -> bool {
        self.words[usize::from(register) / 64] & (1u64 << (usize::from(register) % 64)) != 0
    }

    /// Inserts `start..start + len`, saturating at register 255.
    ///
    /// Windows that would run past the register space are clamped instead of
    /// rejected: the VM reports a structured error for them at runtime, and an
    /// over-approximated read set stays sound.
    pub fn insert_window(&mut self, start: u8, len: usize) {
        let end = usize::from(start).saturating_add(len).min(REGISTER_SPACE);
        for register in usize::from(start)..end {
            self.insert(register as u8);
        }
    }

    /// The set `0..len`, saturating at register 255.
    pub fn all_below(len: usize) -> Self {
        let mut set = Self::EMPTY;
        set.insert_window(0, len.min(REGISTER_SPACE));
        set
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.words == [0; REGISTER_WORDS]
    }

    pub fn len(self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Unions `other` into `self`; returns whether `self` changed.
    pub fn union_with(&mut self, other: Self) -> bool {
        let mut changed = false;
        for (word, other_word) in self.words.iter_mut().zip(other.words) {
            let merged = *word | other_word;
            changed |= merged != *word;
            *word = merged;
        }
        changed
    }

    pub fn difference(self, other: Self) -> Self {
        let mut words = [0; REGISTER_WORDS];
        for (index, word) in words.iter_mut().enumerate() {
            *word = self.words[index] & !other.words[index];
        }
        Self { words }
    }

    pub fn intersection(self, other: Self) -> Self {
        let mut words = [0; REGISTER_WORDS];
        for (index, word) in words.iter_mut().enumerate() {
            *word = self.words[index] & other.words[index];
        }
        Self { words }
    }

    /// Ascending register order.
    pub fn iter(self) -> impl Iterator<Item = u8> {
        (0..REGISTER_SPACE as u16).filter_map(move |register| {
            let register = register as u8;
            self.contains(register).then_some(register)
        })
    }
}

impl std::fmt::Debug for RegisterSet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self.iter()).finish()
    }
}

impl FromIterator<u8> for RegisterSet {
    fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
        let mut set = Self::EMPTY;
        for register in iter {
            set.insert(register);
        }
        set
    }
}

/// How one instruction can leave the current straight-line block.
///
/// Targets are raw absolute `aux` values; [`crate::liveness`] validates them
/// against the function body. A conditional `Branch` also falls through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstructionFlow {
    /// Continues at `pc + 1` only.
    FallThrough,
    /// Continues at `target` only.
    Jump { target: i32 },
    /// Continues at `pc + 1` or at `target`.
    Branch { target: i32 },
    /// Leaves the frame: `Return`/`ReturnVarargs`/`ReturnCall*`/`Halt`.
    ///
    /// The stackless self-tail-call path can restart such a frame at `pc 0`,
    /// but it first rebuilds every register from the outgoing arguments, so no
    /// register value flows across that edge and liveness may ignore it.
    Terminate,
}

/// Static operand and effect facts for one instruction.
///
/// See the module documentation for the over-/under-approximation contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionFacts {
    /// Registers whose *values* the instruction may read.
    pub reads: RegisterSet,
    /// Conservative fallback: the instruction may read any register below the
    /// function's register count. Used for exotic call shapes instead of
    /// modeling their multi-field `aux` packings.
    pub reads_all_registers: bool,
    /// Registers written on every completing path (liveness kills).
    pub writes: RegisterSet,
    /// Registers that may be written on some path (never a kill).
    pub conditional_writes: RegisterSet,
    /// Local cells the instruction reads through (`GetLocalCell`).
    pub cell_reads: RegisterSet,
    /// Local cells the instruction may attach or write through.
    pub cell_attaches: RegisterSet,
    /// Local cells the instruction unconditionally detaches
    /// (`ClearLocalCell` only; conditional loop-variable detaches are
    /// deliberately ignored so may-attach stays an over-approximation).
    pub cell_detaches: RegisterSet,
    /// Reads the frame's vararg pack.
    pub reads_varargs: bool,
    pub flow: InstructionFlow,
    /// May execute guest code before completing: metamethods, iterator or
    /// function calls, `__tostring`, host extensions. Implies a safepoint and
    /// a possible yield inside a coroutine.
    pub may_enter_guest: bool,
    /// May raise a structured guest-visible error at this PC.
    pub may_throw: bool,
    /// May allocate VM-owned memory (collection boundary candidate).
    pub may_allocate: bool,
}

impl InstructionFacts {
    fn new() -> Self {
        Self {
            reads: RegisterSet::EMPTY,
            reads_all_registers: false,
            writes: RegisterSet::EMPTY,
            conditional_writes: RegisterSet::EMPTY,
            cell_reads: RegisterSet::EMPTY,
            cell_attaches: RegisterSet::EMPTY,
            cell_detaches: RegisterSet::EMPTY,
            reads_varargs: false,
            flow: InstructionFlow::FallThrough,
            may_enter_guest: false,
            may_throw: false,
            may_allocate: false,
        }
    }

    fn reads(mut self, register: u8) -> Self {
        self.reads.insert(register);
        self
    }

    fn reads_window(mut self, start: u8, len: usize) -> Self {
        self.reads.insert_window(start, len);
        self
    }

    fn reads_all(mut self) -> Self {
        self.reads_all_registers = true;
        self
    }

    fn writes(mut self, register: u8) -> Self {
        self.writes.insert(register);
        self
    }

    fn writes_window(mut self, start: u8, len: usize) -> Self {
        self.writes.insert_window(start, len);
        self
    }

    fn conditionally_writes_window(mut self, start: u8, len: usize) -> Self {
        self.conditional_writes.insert_window(start, len);
        self
    }

    fn reads_cell(mut self, register: u8) -> Self {
        self.cell_reads.insert(register);
        self
    }

    fn attaches_cell(mut self, register: u8) -> Self {
        self.cell_attaches.insert(register);
        self
    }

    fn detaches_cell(mut self, register: u8) -> Self {
        self.cell_detaches.insert(register);
        self
    }

    fn reads_varargs(mut self) -> Self {
        self.reads_varargs = true;
        self
    }

    fn jump(mut self, target: i32) -> Self {
        self.flow = InstructionFlow::Jump { target };
        self
    }

    fn branch(mut self, target: i32) -> Self {
        self.flow = InstructionFlow::Branch { target };
        self
    }

    fn terminates(mut self) -> Self {
        self.flow = InstructionFlow::Terminate;
        self
    }

    fn guest(mut self) -> Self {
        self.may_enter_guest = true;
        self.may_throw = true;
        self.may_allocate = true;
        self
    }

    fn throws(mut self) -> Self {
        self.may_throw = true;
        self
    }

    fn allocates(mut self) -> Self {
        self.may_allocate = true;
        self.may_throw = true;
        self
    }

    /// True when the successful common path is pure register bookkeeping, the
    /// same certification the region analysis uses for `RegisterOnly` effects.
    pub fn is_register_only(&self) -> bool {
        !self.may_enter_guest && !self.may_allocate && !self.reads_varargs
    }
}

/// Clamps a small `aux` arity operand into `0..=cap` for window sizing.
///
/// Invalid arities fail with a structured error at runtime; clamping keeps the
/// read over-approximation intact without trusting hand-built programs.
fn clamped_arity(aux: i32, cap: usize) -> usize {
    if aux <= 0 {
        0
    } else {
        (aux as usize).min(cap)
    }
}

/// The extra registers after `c` read by an `aux`-arity builtin whose first
/// `fixed` arguments live in dedicated operand lanes.
fn trailing_window(aux: i32, fixed: usize, cap: usize) -> usize {
    clamped_arity(aux, cap.saturating_add(fixed)).saturating_sub(fixed)
}

/// Static facts for `instruction` inside `function` of `program`.
///
/// `program` resolves `LoadFunction` prototypes so closure creation reports
/// the registers it captures. The match is exhaustive over [`Opcode`]: adding
/// an opcode without deciding its facts is a compile error.
pub fn instruction_facts(
    program: &Program,
    function: &Function,
    instruction: Instruction,
) -> InstructionFacts {
    let instruction =
        if instruction.aux < 0 && crate::bytecode::supports_call_descriptor(instruction.op) {
            match program.constants().get((!instruction.aux) as usize) {
                Some(crate::bytecode::Constant::CallDescriptor(descriptor)) => Instruction {
                    aux: descriptor.aux,
                    ..instruction
                },
                _ => instruction,
            }
        } else {
            instruction
        };
    let base = InstructionFacts::new();
    let Instruction { op, a, b, c, aux } = instruction;
    match op {
        Opcode::Halt => base.terminates(),
        // Constant loads may atomize a long string; primitive certification is
        // region_effect's job, so a conservative allocation flag is fine here.
        Opcode::LoadConst | Opcode::LoadConstWide => base.writes(a).allocates(),
        Opcode::Move => base.reads(b).writes(a).throws(),
        Opcode::ClearLocalCell => base.writes(a).detaches_cell(a).throws(),
        Opcode::GetLocalCell => base
            .reads(b)
            .reads_cell(b)
            .attaches_cell(b)
            .writes(a)
            .allocates(),
        Opcode::SetLocalCell => base.reads(a).writes(b).attaches_cell(b).allocates(),
        Opcode::GetUpvalue => base.writes(a).throws(),
        Opcode::SetUpvalue => base.reads(a).throws(),
        Opcode::LoadCurrentFunction => base.writes(a).throws(),
        Opcode::LoadFunction => {
            let mut facts = base.writes(a).allocates();
            // Closure creation reads every captured parent local and
            // materializes cells for the mutable ones. Invalid prototype
            // indices fail with a structured error at runtime and capture
            // nothing.
            if let Ok(index) = usize::try_from(aux) {
                if let Some(prototype) = program.functions.get(index) {
                    for descriptor in &prototype.upvalues {
                        match *descriptor {
                            UpvalueDescriptor::ParentLocal(register) => {
                                facts.reads.insert(register);
                                facts.cell_reads.insert(register);
                                facts.cell_attaches.insert(register);
                            }
                            UpvalueDescriptor::ParentLocalValue(register) => {
                                facts.reads.insert(register);
                            }
                            UpvalueDescriptor::ParentUpvalue(_)
                            | UpvalueDescriptor::ParentUpvalueValue(_) => {}
                        }
                    }
                }
            }
            facts
        }
        // Binary arithmetic and comparison may enter metamethods.
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::Mod
        | Opcode::FloorDiv
        | Opcode::Pow
        | Opcode::Equal
        | Opcode::NotEqual
        | Opcode::Less
        | Opcode::LessEqual
        | Opcode::Greater
        | Opcode::GreaterEqual
        | Opcode::Concat => base.reads(b).reads(c).writes(a).guest(),
        Opcode::Neg | Opcode::Len | Opcode::ToString => base.reads(b).writes(a).guest(),
        Opcode::Not => base.reads(b).writes(a).throws(),
        Opcode::Abs | Opcode::ToNumber => base.reads(b).writes(a).throws(),
        Opcode::CoerceForNumber => base.reads(b).writes(a).throws(),
        Opcode::Jump => base.jump(aux),
        Opcode::JumpIf | Opcode::JumpIfNot => base.reads(a).branch(aux).throws(),
        Opcode::Assert => {
            let mut facts = base.reads(b).writes(a).throws();
            if let Ok(message) = u8::try_from(aux) {
                facts.reads.insert(message);
            }
            facts
        }
        Opcode::Return => base.reads_window(a, usize::from(b)).terminates().throws(),
        Opcode::ReturnVarargs => base
            .reads_window(a, usize::from(b))
            .reads_varargs()
            .terminates()
            .throws(),
        Opcode::ReturnCall => {
            // prefix window (a, b), callee c, argument window from packed aux.
            let args_start = ((aux >> 8) & 0xFF) as u8;
            let argc = (aux & 0xFF) as usize;
            base.reads_window(a, usize::from(b))
                .reads(c)
                .reads_window(args_start, argc)
                .terminates()
                .guest()
        }
        // Remaining tail-call shapes pack multiple windows into aux; they end
        // the frame, so reading everything loses no useful precision.
        Opcode::ReturnCallVarargs
        | Opcode::ReturnCallExpand
        | Opcode::ReturnCallExpand2
        | Opcode::ReturnCallExpandVarargs
        | Opcode::ReturnCallExpand2Varargs => base.reads_all().reads_varargs().terminates().guest(),
        Opcode::Call | Opcode::FastCall => base
            .reads(b)
            .reads_window(c, clamped_arity(aux, REGISTER_SPACE))
            .writes(a)
            .guest(),
        Opcode::CallMulti => base
            .reads(a)
            .reads_window(a.saturating_add(1), usize::from(b))
            .writes_window(a, usize::from(c))
            .guest(),
        Opcode::CallVarargs => base
            .reads(a)
            .reads_window(a.saturating_add(1), usize::from(b))
            .reads_varargs()
            .writes_window(a, usize::from(c))
            .guest(),
        // Expand-family calls splice nested multi-result windows whose layout
        // lives in packed aux fields; their result windows are runtime
        // adjusted, so nothing is a definite write.
        Opcode::CallExpand
        | Opcode::CallExpand2
        | Opcode::CallExpand3
        | Opcode::CallExpandVarargs => base.reads_all().reads_varargs().guest(),
        Opcode::AppendCall | Opcode::AppendCallVarargs | Opcode::AppendCallExpand => {
            base.reads_all().reads_varargs().guest()
        }
        Opcode::CallExtension => base.reads_window(b, usize::from(c)).writes(a).guest(),
        Opcode::LoadVarargs => base
            .reads_varargs()
            .writes_window(a, usize::from(b))
            .throws(),
        Opcode::AppendVarargs => base.reads(a).reads_varargs().allocates(),
        Opcode::ForPrep => base
            .reads_window(a, 3)
            .conditionally_writes_window(a.saturating_add(3), 1)
            .branch(aux)
            .throws(),
        Opcode::ForLoop => base
            .reads_window(a, 3)
            .writes(a)
            .conditionally_writes_window(a.saturating_add(3), 1)
            .branch(aux)
            .throws(),
        Opcode::IpairsPrep | Opcode::PairsPrep => base
            .reads(a)
            .conditionally_writes_window(a.saturating_add(1), 3)
            .branch(aux)
            .throws(),
        Opcode::IpairsLoop | Opcode::PairsLoop => base
            .reads(a)
            .reads_window(a.saturating_add(1), 1)
            .conditionally_writes_window(a.saturating_add(1), 3)
            .branch(aux)
            .throws(),
        Opcode::GenericCallPrep | Opcode::GenericCallLoop => base
            .reads_window(a, 3)
            .conditionally_writes_window(a, 3usize.saturating_add(usize::from(c)))
            .branch(aux)
            .guest(),
        Opcode::GeneralizedIterPrep | Opcode::GeneralizedIterLoop => base
            .reads_window(a, 4)
            .conditionally_writes_window(a, 4usize.saturating_add(usize::from(c)))
            .branch(aux)
            .guest(),
        Opcode::NewArray => base.reads_window(b, usize::from(c)).writes(a).allocates(),
        Opcode::GetIndex => base.reads(b).reads(c).writes(a).guest(),
        Opcode::SetIndex => base.reads(a).reads(b).reads(c).guest(),
        Opcode::GetTableKs => {
            let facts = base.reads(b).writes(a).guest();
            if c != 0 {
                facts.writes(c)
            } else {
                facts
            }
        }
        Opcode::SetTableKs => {
            let facts = base.reads(a).reads(b).guest();
            if c != 0 {
                facts.reads(c)
            } else {
                facts
            }
        }
        Opcode::NameCall => base.reads(b).writes(a).guest(),
        Opcode::CheckMethod => base.reads(a).reads(b).reads(c).throws(),
        Opcode::GetImport => base.writes(a).guest(),
        Opcode::GetGlobal => base.writes(a).guest(),
        Opcode::SetGlobal => base.reads(a).guest(),
        Opcode::RawGet => base.reads(b).reads(c).writes(a).throws(),
        Opcode::RawSet => base.reads(a).reads(b).reads(c).allocates(),
        Opcode::RawLen => base.reads(b).writes(a).throws(),
        Opcode::NextKey => base.reads(b).reads(c).writes(a).throws(),
        Opcode::TypeOf => base.reads(b).writes(a).allocates(),
        Opcode::TableConcat => base
            .reads(b)
            .reads_window(c, 1 + trailing_window(aux, 2, 2))
            .writes(a)
            .guest(),
        Opcode::ConcatArgs => base.reads_window(b, usize::from(c)).writes(a).guest(),
        Opcode::TableGetN | Opcode::TableMaxN => base.reads(b).writes(a).throws(),
        Opcode::TableFind => base
            .reads(b)
            .reads_window(c, 1 + trailing_window(aux, 2, 1))
            .writes(a)
            .guest(),
        Opcode::TableCreate => {
            // aux == 0 is the PUC constructor pre-sizing hint (b and c are
            // both sizes); aux 1..=2 is the builtin (b size, optional c fill).
            let facts = base.reads(b).writes(a).allocates();
            if aux == 0 || aux == 2 {
                facts.reads(c)
            } else {
                facts
            }
        }
        Opcode::TableInsert => base
            .reads(b)
            .reads_window(c, 1 + trailing_window(aux, 2, 1))
            .writes(a)
            .allocates(),
        Opcode::TableRemove => {
            let facts = base.reads(b).writes(a).allocates();
            if aux == 2 {
                facts.reads(c)
            } else {
                facts
            }
        }
        Opcode::StringSub => base
            .reads(b)
            .reads_window(c, 1 + trailing_window(aux, 2, 1))
            .writes(a)
            .allocates(),
        Opcode::StringLen => base.reads(b).writes(a).throws(),
        Opcode::StringByte => base
            .reads(b)
            .reads_window(c, trailing_window(aux, 1, 2))
            .writes(a)
            .throws(),
        Opcode::StringUpper | Opcode::StringLower | Opcode::StringReverse => {
            base.reads(b).writes(a).allocates()
        }
        Opcode::StringRep => base.reads(b).reads(c).writes(a).allocates(),
        Opcode::StringFind => base
            .reads(b)
            .reads_window(c, 1 + trailing_window(aux, 2, 2))
            .writes(a)
            .throws(),
        Opcode::StringSplit => {
            let facts = base.reads(b).writes(a).allocates();
            if aux == 2 {
                facts.reads(c)
            } else {
                facts
            }
        }
        // One-argument math ops validate aux == 1 and read only b.
        Opcode::MathFloor
        | Opcode::MathCeil
        | Opcode::MathRound
        | Opcode::MathSign
        | Opcode::MathSqrt
        | Opcode::MathDeg
        | Opcode::MathRad
        | Opcode::MathIsNan
        | Opcode::MathIsInf
        | Opcode::MathIsFinite
        | Opcode::MathSin
        | Opcode::MathCos
        | Opcode::MathTan
        | Opcode::MathAsin
        | Opcode::MathAcos
        | Opcode::MathAtan
        | Opcode::MathSinh
        | Opcode::MathCosh
        | Opcode::MathTanh
        | Opcode::MathExp
        | Opcode::MathLog10 => base.reads(b).writes(a).throws(),
        // Fixed-arity multi-argument math ops read a window anchored at b.
        Opcode::MathFmod | Opcode::MathPow | Opcode::MathAtan2 | Opcode::MathLdexp => {
            base.reads_window(b, 2).writes(a).throws()
        }
        Opcode::MathLog => base
            .reads_window(b, clamped_arity(aux, 2).max(1))
            .writes(a)
            .throws(),
        Opcode::MathClamp | Opcode::MathLerp => base.reads_window(b, 3).writes(a).throws(),
        Opcode::MathMap => base.reads_window(b, 5).writes(a).throws(),
        Opcode::MathMin | Opcode::MathMax => base
            .reads_window(b, clamped_arity(aux, REGISTER_SPACE))
            .writes(a)
            .throws(),
    }
    .validated(function)
}

impl InstructionFacts {
    /// Expands the `reads_all_registers` fallback against the function's
    /// register count so downstream set algebra never needs the flag.
    fn validated(mut self, function: &Function) -> Self {
        if self.reads_all_registers {
            self.reads
                .union_with(RegisterSet::all_below(function.max_registers));
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};

    fn program_with(code: Vec<Instruction>, max_registers: usize) -> Program {
        Program::new(
            Function::new(code, max_registers),
            vec![Constant::Number(1.0)],
        )
    }

    fn facts_for(instruction: Instruction) -> InstructionFacts {
        let program = program_with(vec![instruction], 16);
        instruction_facts(&program, &program.main, instruction)
    }

    #[test]
    fn register_set_algebra_is_exact() {
        let mut set = RegisterSet::EMPTY;
        assert!(set.is_empty());
        set.insert(0);
        set.insert(63);
        set.insert(64);
        set.insert(255);
        assert_eq!(set.len(), 4);
        assert!(set.contains(63) && set.contains(64));
        assert!(!set.contains(1));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![0, 63, 64, 255]);

        set.remove(63);
        assert!(!set.contains(63));

        let mut window = RegisterSet::EMPTY;
        window.insert_window(254, 5);
        assert_eq!(window.iter().collect::<Vec<_>>(), vec![254, 255]);

        let all = RegisterSet::all_below(3);
        assert_eq!(all.iter().collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(
            all.difference([1u8].into_iter().collect())
                .iter()
                .collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            all.intersection([1u8, 7].into_iter().collect())
                .iter()
                .collect::<Vec<_>>(),
            vec![1]
        );

        let mut union = RegisterSet::EMPTY;
        assert!(union.union_with(all));
        assert!(!union.union_with(all));
    }

    #[test]
    fn move_and_clear_facts_are_register_only() {
        let move_facts = facts_for(Instruction::new(Opcode::Move, 2, 5, 0));
        assert_eq!(move_facts.reads.iter().collect::<Vec<_>>(), vec![5]);
        assert_eq!(move_facts.writes.iter().collect::<Vec<_>>(), vec![2]);
        assert!(move_facts.is_register_only());
        assert_eq!(move_facts.flow, InstructionFlow::FallThrough);

        let clear_facts = facts_for(Instruction::new(Opcode::ClearLocalCell, 3, 0, 0));
        assert!(clear_facts.reads.is_empty());
        assert_eq!(clear_facts.writes.iter().collect::<Vec<_>>(), vec![3]);
        assert_eq!(
            clear_facts.cell_detaches.iter().collect::<Vec<_>>(),
            vec![3]
        );
        assert!(clear_facts.is_register_only());
    }

    #[test]
    fn arithmetic_reads_operands_and_may_enter_metamethods() {
        let facts = facts_for(Instruction::new(Opcode::Add, 1, 2, 3));
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(facts.writes.iter().collect::<Vec<_>>(), vec![1]);
        assert!(facts.may_enter_guest && facts.may_throw);
    }

    #[test]
    fn call_reads_callee_and_argument_window() {
        let facts = facts_for(Instruction::with_aux(Opcode::Call, 0, 4, 6, 2));
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![4, 6, 7]);
        assert_eq!(facts.writes.iter().collect::<Vec<_>>(), vec![0]);
        assert!(facts.may_enter_guest);
    }

    #[test]
    fn call_multi_defines_its_result_window() {
        let facts = facts_for(Instruction::new(Opcode::CallMulti, 3, 2, 4));
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![3, 4, 5]);
        assert_eq!(facts.writes.iter().collect::<Vec<_>>(), vec![3, 4, 5, 6]);
    }

    #[test]
    fn control_flow_targets_come_from_aux() {
        assert_eq!(
            facts_for(Instruction::with_aux(Opcode::Jump, 0, 0, 0, 7)).flow,
            InstructionFlow::Jump { target: 7 }
        );
        assert_eq!(
            facts_for(Instruction::with_aux(Opcode::JumpIfNot, 1, 0, 0, 9)).flow,
            InstructionFlow::Branch { target: 9 }
        );
        assert_eq!(
            facts_for(Instruction::new(Opcode::Return, 0, 2, 0)).flow,
            InstructionFlow::Terminate
        );
    }

    #[test]
    fn for_loop_updates_index_unconditionally_and_visible_conditionally() {
        let facts = facts_for(Instruction::with_aux(Opcode::ForLoop, 4, 0, 0, 2));
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![4, 5, 6]);
        assert_eq!(facts.writes.iter().collect::<Vec<_>>(), vec![4]);
        assert_eq!(facts.conditional_writes.iter().collect::<Vec<_>>(), vec![7]);
        assert_eq!(facts.flow, InstructionFlow::Branch { target: 2 });
    }

    #[test]
    fn local_cell_facts_track_attach_read_and_detach() {
        let get = facts_for(Instruction::new(Opcode::GetLocalCell, 0, 3, 0));
        assert_eq!(get.cell_reads.iter().collect::<Vec<_>>(), vec![3]);
        assert_eq!(get.cell_attaches.iter().collect::<Vec<_>>(), vec![3]);

        let set = facts_for(Instruction::new(Opcode::SetLocalCell, 1, 3, 0));
        assert_eq!(set.reads.iter().collect::<Vec<_>>(), vec![1]);
        assert_eq!(set.writes.iter().collect::<Vec<_>>(), vec![3]);
        assert_eq!(set.cell_attaches.iter().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn load_function_reads_captured_parent_locals() {
        let prototype = Function::with_params_and_upvalues(
            vec![Instruction::new(Opcode::Return, 0, 0, 0)],
            1,
            0,
            vec![
                UpvalueDescriptor::ParentLocal(2),
                UpvalueDescriptor::ParentLocalValue(5),
                UpvalueDescriptor::ParentUpvalue(0),
            ],
            Vec::new(),
        );
        let program = Program::with_functions(
            Function::new(
                vec![Instruction::with_aux(Opcode::LoadFunction, 0, 0, 0, 0)],
                8,
            ),
            Vec::new(),
            vec![prototype],
        );
        let facts = instruction_facts(&program, &program.main, program.main.code[0]);
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(facts.cell_attaches.iter().collect::<Vec<_>>(), vec![2]);

        // A missing prototype captures nothing and stays conservative.
        let missing = instruction_facts(
            &program,
            &program.main,
            Instruction::with_aux(Opcode::LoadFunction, 0, 0, 0, 9),
        );
        assert!(missing.reads.is_empty());
    }

    #[test]
    fn exotic_call_shapes_fall_back_to_reading_all_registers() {
        let program = program_with(
            vec![Instruction::with_aux(
                Opcode::ReturnCallExpand,
                0,
                1,
                2,
                0x0101,
            )],
            4,
        );
        let facts = instruction_facts(&program, &program.main, program.main.code[0]);
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(facts.reads_all_registers);
        assert_eq!(facts.flow, InstructionFlow::Terminate);
    }

    #[test]
    fn aux_arity_windows_clamp_untrusted_operands() {
        // string.sub with the 3-argument arity reads c and c + 1.
        let facts = facts_for(Instruction::with_aux(Opcode::StringSub, 0, 1, 2, 3));
        assert_eq!(facts.reads.iter().collect::<Vec<_>>(), vec![1, 2, 3]);
        // A hostile arity cannot expand the window beyond the modeled cap.
        let hostile = facts_for(Instruction::with_aux(Opcode::StringSub, 0, 1, 2, i32::MAX));
        assert_eq!(hostile.reads.iter().collect::<Vec<_>>(), vec![1, 2, 3]);
        // math.min reads its full aux-sized window.
        let min = facts_for(Instruction::with_aux(Opcode::MathMin, 0, 1, 0, 3));
        assert_eq!(min.reads.iter().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn every_region_candidate_is_register_only_or_guarded() {
        // The region effect table and these facts must agree on which opcodes
        // have a register-only successful path.
        for opcode in [Opcode::Move, Opcode::ClearLocalCell] {
            let facts = facts_for(Instruction::new(opcode, 1, 2, 0));
            assert!(facts.is_register_only(), "{opcode:?}");
        }
    }
}
