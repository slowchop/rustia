//! Default-disabled C2.3 compiler O2 rewrite passes.
//!
//! Every pass here is behind an [`ExperimentalO2Options`] flag that defaults
//! to **off**: the default compiler emits byte-identical bytecode whether this
//! module exists or not, which is pinned by tests. The flags exist so each
//! optimization family can be validated independently and enabled one at a
//! time *after* C2.2's whole-function executable tape makes fuel and
//! deterministic-GC scheduling optimization-independent through logical
//! receipts ([`design/58-vm-core2.md`] §C2.3). Until then, enabling a family
//! is a declared compiler/fuel-schedule revision, never a default.
//!
//! # Semantic contract (all families)
//!
//! - Source evaluation order, metamethod timing, and the operation at which
//!   every error occurs are preserved: rewrites only retarget pure destination
//!   operands, delete writes to registers the liveness analysis proves dead,
//!   and fold branches whose condition is a compile-time constant. Error
//!   *source locations* (lines) and messages are identical because the source
//!   map is remapped with the code; raw PCs necessarily shift whenever an
//!   instruction is deleted, exactly as they do across any compiler revision.
//! - Required captured-local close/detach operations are never removed: only
//!   [`ClearKind::PureCleanup`] clears with dead values are candidates.
//! - Multiple-result adjustment and vararg semantics are untouched: no rewrite
//!   crosses or alters a call, expand, or vararg window.
//! - Every deleted instruction folds its logical unit into a surviving
//!   instruction of the same basic block, so any executed path charges exactly
//!   the logical instructions the unoptimized path charged
//!   ([`LogicalReceipts`]). This is the schedule the tape consumes.

use rustia_vm::bytecode::{Function, Instruction, InstructionSource, Opcode, Program};
use rustia_vm::instruction_facts::{instruction_facts, InstructionFlow};
use rustia_vm::liveness::{ClearKind, FunctionLiveness, LogicalReceipts};
use rustia_vm::Constant;

use crate::{CompileError, Result};

/// Internal, default-disabled C2.3 optimization families.
///
/// This is deliberately not a stable public knob: the fields may change as
/// C2.3 lands, and no default construction path enables any of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExperimentalO2Options {
    /// Fold `LoadConst t; JumpIf/JumpIfNot t` pairs whose condition is a
    /// compile-time constant into an unconditional jump or plain
    /// fall-through.
    pub fold_constant_branches: bool,
    /// Lower `x op= rhs` on uncaptured locals directly into `x`'s register,
    /// deleting the read-copy, write-back, and result-lifetime tail while
    /// keeping the observable commit after complete right-hand-side
    /// evaluation.
    pub commit_late_compound_locals: bool,
    /// Retarget single-destination producers over their `Move d, t;
    /// ClearLocalCell t` result-lifetime tails and delete uncaptured cleanup
    /// clears whose value the analysis proves dead.
    pub remove_dead_register_tails: bool,
    /// Shrink `max_registers` to the analyzed operand high-water mark.
    pub tighten_local_slots: bool,
}

impl ExperimentalO2Options {
    /// Every family enabled; test/validation convenience.
    pub const ALL: Self = Self {
        fold_constant_branches: true,
        commit_late_compound_locals: true,
        remove_dead_register_tails: true,
        tighten_local_slots: true,
    };

    pub fn any_enabled(self) -> bool {
        self != Self::default()
    }
}

/// Logical receipts for a whole optimized program, main first.
///
/// This plus the optimized [`Program`] is the compiler half of the tape
/// contract: the tape prices each block by summing these units, which restores
/// the unoptimized fuel and deterministic-GC schedule over optimized code.
#[derive(Clone, Debug)]
pub struct ProgramReceipts {
    pub main: LogicalReceipts,
    pub functions: Vec<LogicalReceipts>,
}

impl ProgramReceipts {
    /// Identity receipts for an unoptimized program.
    pub fn identity(program: &Program) -> Self {
        Self {
            main: LogicalReceipts::identity(program.main.code.len()),
            functions: program
                .functions
                .iter()
                .map(|function| LogicalReceipts::identity(function.code.len()))
                .collect(),
        }
    }
}

/// Applies the enabled families to every function of `program`.
///
/// Pure bytecode-to-bytecode: upvalue descriptors, captured-register sets,
/// parameter metadata, and the constant pool are never changed, so
/// instruction facts resolved against the *original* program stay valid for
/// every function while the rewrite proceeds.
pub(crate) fn optimize_program(
    program: &Program,
    options: ExperimentalO2Options,
) -> Result<(Program, ProgramReceipts)> {
    let (main, main_units) = optimize_function(program, &program.main, options)?;
    let mut functions = Vec::with_capacity(program.functions.len());
    let mut function_receipts = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        let (optimized, units) = optimize_function(program, function, options)?;
        let receipts = LogicalReceipts::from_units(units, optimized.code.len())
            .map_err(|error| CompileError::new(format!("O2 receipt invariant: {error}")))?;
        functions.push(optimized);
        function_receipts.push(receipts);
    }
    let main_receipts = LogicalReceipts::from_units(main_units, main.code.len())
        .map_err(|error| CompileError::new(format!("O2 receipt invariant: {error}")))?;

    let constants = program.constants().to_vec();
    let mut optimized = Program::with_functions(main, constants, functions);
    optimized.source_name = program.source_name.clone();
    Ok((
        optimized,
        ProgramReceipts {
            main: main_receipts,
            functions: function_receipts,
        },
    ))
}

fn optimize_function(
    program: &Program,
    function: &Function,
    options: ExperimentalO2Options,
) -> Result<(Function, Vec<u32>)> {
    let mut current = function.clone();
    let mut units: Vec<u32> = vec![1; current.code.len()];

    if options.fold_constant_branches {
        let plan = plan_constant_branch_folds(program, &current)?;
        apply_rewrite(program, &mut current, &mut units, plan)?;
    }
    if options.commit_late_compound_locals {
        let plan = plan_commit_late_compound_locals(program, &current)?;
        apply_rewrite(program, &mut current, &mut units, plan)?;
    }
    if options.remove_dead_register_tails {
        let plan = plan_dead_register_tails(program, &current)?;
        apply_rewrite(program, &mut current, &mut units, plan)?;
    }
    if options.tighten_local_slots {
        tighten_local_slots(program, &mut current)?;
    }

    Ok((current, units))
}

/// One pass's pending edits, parallel to the current code vector.
struct RewritePlan {
    replacements: Vec<Option<Instruction>>,
    deleted: Vec<bool>,
}

impl RewritePlan {
    fn new(len: usize) -> Self {
        Self {
            replacements: vec![None; len],
            deleted: vec![false; len],
        }
    }

    fn is_empty(&self) -> bool {
        self.replacements.iter().all(Option::is_none) && !self.deleted.iter().any(|flag| *flag)
    }

    fn touched(&self, pc: usize) -> bool {
        self.deleted[pc] || self.replacements[pc].is_some()
    }
}

/// Applies one plan: rewrites instructions in place, deletes marked
/// instructions, remaps absolute jump targets and the source map, and folds
/// each deleted instruction's logical units into a surviving instruction of
/// the same basic block.
fn apply_rewrite(
    program: &Program,
    function: &mut Function,
    units: &mut Vec<u32>,
    plan: RewritePlan,
) -> Result<()> {
    if plan.is_empty() {
        return Ok(());
    }
    let liveness = analyze(program, function)?;
    let code_len = function.code.len();

    // Fold deleted units into a same-block survivor: nearest earlier, else
    // nearest later. Planners guarantee at least one survivor per block that
    // loses an instruction.
    let block_range = |pc: usize| -> (usize, usize) {
        let block = liveness
            .cfg()
            .block_index_of_pc(pc)
            .expect("pc is inside the function");
        let block = &liveness.cfg().blocks()[block];
        (block.start_pc, block.end_pc)
    };
    for pc in 0..code_len {
        if !plan.deleted[pc] {
            continue;
        }
        let (start, end) = block_range(pc);
        let earlier = (start..pc).rev().find(|&other| !plan.deleted[other]);
        let later = (pc + 1..end).find(|&other| !plan.deleted[other]);
        let Some(survivor) = earlier.or(later) else {
            return Err(CompileError::new(
                "O2 deleted a complete basic block; receipts cannot stay block-local",
            ));
        };
        units[survivor] += units[pc];
        units[pc] = 0;
    }

    // Old-PC to new-PC maps: exact for survivors, first-surviving-at-or-after
    // for jump targets (a deleted instruction's effect is proven absent, so
    // entering at its successor is equivalent).
    let mut new_pc_of = vec![usize::MAX; code_len + 1];
    let mut next_surviving = vec![usize::MAX; code_len + 1];
    let mut emitted = 0usize;
    for (slot, deleted) in new_pc_of.iter_mut().zip(plan.deleted.iter()) {
        if !deleted {
            *slot = emitted;
            emitted += 1;
        }
    }
    let surviving_total = emitted;
    let mut next = surviving_total;
    next_surviving[code_len] = surviving_total;
    for pc in (0..code_len).rev() {
        if !plan.deleted[pc] {
            next = new_pc_of[pc];
        }
        next_surviving[pc] = next;
    }

    // Emit the rewritten body.
    let mut new_code = Vec::with_capacity(surviving_total);
    let mut new_units = Vec::with_capacity(surviving_total);
    for (pc, deleted) in plan.deleted.iter().enumerate() {
        if *deleted {
            continue;
        }
        let mut instruction = plan.replacements[pc].unwrap_or(function.code[pc]);
        let facts = instruction_facts(program, function, instruction);
        match facts.flow {
            InstructionFlow::Jump { target } | InstructionFlow::Branch { target } => {
                let target = usize::try_from(target)
                    .map_err(|_| CompileError::new("O2 found a negative jump target"))?;
                let remapped = *next_surviving
                    .get(target)
                    .ok_or_else(|| CompileError::new("O2 found a jump past the function"))?;
                instruction.aux = i32::try_from(remapped)
                    .map_err(|_| CompileError::new("O2 jump target overflow"))?;
            }
            InstructionFlow::FallThrough | InstructionFlow::Terminate => {}
        }
        new_code.push(instruction);
        new_units.push(units[pc]);
    }

    // Remap the source map onto surviving PCs; keep the last entry per PC,
    // matching the emitter's own dedup rule.
    let mut new_source_map: Vec<InstructionSource> = Vec::with_capacity(function.source_map.len());
    for entry in &function.source_map {
        let Some(&mapped) = next_surviving.get(entry.pc.min(code_len)) else {
            continue;
        };
        if mapped >= surviving_total {
            continue;
        }
        if let Some(last) = new_source_map.last_mut() {
            if last.pc == mapped {
                last.line = entry.line;
                continue;
            }
        }
        new_source_map.push(InstructionSource {
            pc: mapped,
            line: entry.line,
        });
    }

    function.code = new_code;
    function.source_map = new_source_map;
    *units = new_units;
    Ok(())
}

fn analyze(program: &Program, function: &Function) -> Result<FunctionLiveness> {
    FunctionLiveness::analyze(program, function)
        .map_err(|error| CompileError::new(format!("O2 liveness analysis failed: {error}")))
}

fn constant_index_of_load(instruction: Instruction) -> Option<usize> {
    match instruction.op {
        Opcode::LoadConst => Some(usize::from(instruction.c)),
        Opcode::LoadConstWide => usize::try_from(instruction.aux).ok(),
        _ => None,
    }
}

fn constant_truthiness(constant: &Constant) -> bool {
    !matches!(constant, Constant::Nil | Constant::Boolean(false))
}

/// `LoadConst t, k; JumpIf/JumpIfNot t` with a dead `t`: the branch decision
/// is known now. Taken branches become one `Jump` carrying both logical
/// units; never-taken pairs are deleted with their units folded into an
/// earlier same-block survivor.
fn plan_constant_branch_folds(program: &Program, function: &Function) -> Result<RewritePlan> {
    let liveness = analyze(program, function)?;
    let mut plan = RewritePlan::new(function.code.len());

    for pc in 0..function.code.len().saturating_sub(1) {
        if plan.touched(pc) || plan.touched(pc + 1) {
            continue;
        }
        let load = function.code[pc];
        let branch = function.code[pc + 1];
        let Some(constant_index) = constant_index_of_load(load) else {
            continue;
        };
        if !matches!(branch.op, Opcode::JumpIf | Opcode::JumpIfNot) || branch.a != load.a {
            continue;
        }
        // The pair must be private to one block: nothing may enter at the
        // branch with a different condition value.
        if liveness.cfg().block_index_of_pc(pc) != liveness.cfg().block_index_of_pc(pc + 1) {
            continue;
        }
        let condition_dead = liveness
            .live_registers_after(pc + 1)
            .is_some_and(|live| !live.contains(load.a));
        if !condition_dead || liveness.captured_registers().contains(load.a) {
            continue;
        }
        let Some(constant) = program.constants().get(constant_index) else {
            continue;
        };
        let truthy = constant_truthiness(constant);
        let taken = match branch.op {
            Opcode::JumpIf => truthy,
            Opcode::JumpIfNot => !truthy,
            _ => unreachable!("filtered above"),
        };
        if taken {
            plan.deleted[pc] = true;
            plan.replacements[pc + 1] =
                Some(Instruction::with_aux(Opcode::Jump, 0, 0, 0, branch.aux));
        } else {
            // Both instructions disappear; require an earlier survivor in the
            // block so the units stay block-local.
            let block = liveness.cfg().block_index_of_pc(pc).expect("pc in range");
            let start = liveness.cfg().blocks()[block].start_pc;
            let has_earlier_survivor = (start..pc).any(|other| !plan.deleted[other]);
            if !has_earlier_survivor {
                continue;
            }
            plan.deleted[pc] = true;
            plan.deleted[pc + 1] = true;
        }
    }
    Ok(plan)
}

/// Binary opcodes compound assignment can produce.
fn is_compound_operation(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::FloorDiv
            | Opcode::Mod
            | Opcode::Pow
            | Opcode::Concat
    )
}

/// The exact uncaptured-local compound-assignment lowering:
///
/// ```text
/// pc0:      Move r, x            (read copy)
/// pc0+1..:  <rhs evaluation>     (never touches r, never writes x)
/// op_pc:    OP r, r, s
/// op_pc+1:  Move x, r            (commit)
/// op_pc+2:  ClearLocalCell s
/// op_pc+3:  ClearLocalCell r
/// ```
///
/// becomes `<rhs>; OP x, x, s; ClearLocalCell s`. The operand values, the
/// metamethod entry, the error instruction, and the commit-after-complete-RHS
/// ordering are identical; only the dead temporary lifetime disappears.
fn plan_commit_late_compound_locals(program: &Program, function: &Function) -> Result<RewritePlan> {
    let liveness = analyze(program, function)?;
    let facts: Vec<_> = function
        .code
        .iter()
        .map(|instruction| instruction_facts(program, function, *instruction))
        .collect();
    let mut plan = RewritePlan::new(function.code.len());

    for op_pc in 0..function.code.len() {
        if op_pc + 3 >= function.code.len() {
            break;
        }
        let operation = function.code[op_pc];
        let commit = function.code[op_pc + 1];
        let clear_rhs = function.code[op_pc + 2];
        let clear_result = function.code[op_pc + 3];
        if !is_compound_operation(operation.op) || operation.aux != 0 {
            continue;
        }
        let result = operation.a;
        let rhs = operation.c;
        if operation.b != result || rhs == result {
            continue;
        }
        if commit.op != Opcode::Move || commit.b != result {
            continue;
        }
        let target = commit.a;
        if target == result || target == rhs {
            continue;
        }
        if clear_rhs.op != Opcode::ClearLocalCell || clear_rhs.a != rhs {
            continue;
        }
        if clear_result.op != Opcode::ClearLocalCell || clear_result.a != result {
            continue;
        }
        // Uncaptured registers only; a captured target lowers through
        // GetLocalCell/SetLocalCell and never matches this shape.
        let captured = liveness.captured_registers();
        if captured.contains(result) || captured.contains(rhs) || captured.contains(target) {
            continue;
        }
        // The temporary must be dead after its (deleted) clear.
        let result_dead = liveness
            .live_registers_after(op_pc + 3)
            .is_some_and(|live| !live.contains(result));
        if !result_dead {
            continue;
        }
        // No cell may be attachable to the temporaries anywhere in the shape.
        let cells = liveness
            .cells_attached_before(op_pc + 3)
            .unwrap_or_default();
        if cells.contains(result) || cells.contains(rhs) {
            continue;
        }

        // Walk backwards to the read copy `Move r, x`: the first instruction
        // touching r must be exactly that copy, nothing in between may write
        // x or bail to a conservative read-all summary, and the whole shape
        // must be straight-line.
        let mut pc0 = None;
        let mut scan = op_pc;
        while scan > 0 {
            scan -= 1;
            let candidate = function.code[scan];
            let candidate_facts = &facts[scan];
            let touches_result = candidate_facts.reads.contains(result)
                || candidate_facts.writes.contains(result)
                || candidate_facts.conditional_writes.contains(result)
                || candidate_facts.cell_attaches.contains(result)
                || candidate_facts.cell_detaches.contains(result);
            if touches_result {
                if candidate.op == Opcode::Move
                    && candidate.a == result
                    && candidate.b == target
                    && !plan.touched(scan)
                {
                    pc0 = Some(scan);
                }
                break;
            }
            if candidate_facts.writes.contains(target)
                || candidate_facts.conditional_writes.contains(target)
                || candidate_facts.cell_attaches.contains(target)
                || candidate_facts.reads_all_registers
            {
                break;
            }
        }
        let Some(pc0) = pc0 else {
            continue;
        };
        // Straight-line only: every instruction of the shape shares a block.
        let block = liveness.cfg().block_index_of_pc(pc0);
        if (pc0..=op_pc + 3).any(|pc| liveness.cfg().block_index_of_pc(pc) != block) {
            continue;
        }
        if (pc0..=op_pc + 3).any(|pc| plan.touched(pc)) {
            continue;
        }

        plan.deleted[pc0] = true;
        plan.replacements[op_pc] = Some(Instruction::new(operation.op, target, target, rhs));
        plan.deleted[op_pc + 1] = true;
        plan.deleted[op_pc + 3] = true;
    }
    Ok(plan)
}

/// Opcodes whose `a` operand is a pure destination: the dispatch writes
/// register `a` and never reads it. Pinned by inspection of the VM dispatch;
/// retargeting `a` on any of these cannot change an input value, the
/// metamethod entry, or the error site.
///
/// Deliberately absent: `SetLocalCell` (reads `a` as the stored value),
/// `SetIndex`/`SetTableKs`/`SetGlobal`/`SetUpvalue` (read `a`), `CheckMethod`
/// (reads `a`), branches (read `a`), window writers (`CallMulti`,
/// `CallVarargs`, `LoadVarargs`), and every expand/append shape.
fn a_is_pure_destination(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::LoadConst
            | Opcode::LoadConstWide
            | Opcode::Move
            | Opcode::GetLocalCell
            | Opcode::GetUpvalue
            | Opcode::LoadCurrentFunction
            | Opcode::LoadFunction
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::Div
            | Opcode::FloorDiv
            | Opcode::Mod
            | Opcode::Pow
            | Opcode::Equal
            | Opcode::NotEqual
            | Opcode::Less
            | Opcode::LessEqual
            | Opcode::Greater
            | Opcode::GreaterEqual
            | Opcode::Not
            | Opcode::Neg
            | Opcode::Abs
            | Opcode::Len
            | Opcode::Concat
            | Opcode::ToNumber
            | Opcode::ToString
            | Opcode::TypeOf
            | Opcode::GetIndex
            | Opcode::GetTableKs
            | Opcode::GetGlobal
            | Opcode::GetImport
            | Opcode::RawGet
            | Opcode::RawLen
            | Opcode::NextKey
            | Opcode::Call
            | Opcode::FastCall
            | Opcode::NameCall
            | Opcode::CallExtension
            | Opcode::StringSub
            | Opcode::StringLen
            | Opcode::StringByte
            | Opcode::StringUpper
            | Opcode::StringLower
            | Opcode::StringReverse
            | Opcode::StringRep
            | Opcode::StringFind
            | Opcode::StringSplit
            | Opcode::TableConcat
            | Opcode::ConcatArgs
            | Opcode::TableGetN
            | Opcode::TableMaxN
            | Opcode::TableFind
            | Opcode::TableCreate
            | Opcode::TableInsert
            | Opcode::TableRemove
            | Opcode::MathFloor
            | Opcode::MathCeil
            | Opcode::MathRound
            | Opcode::MathSign
            | Opcode::MathClamp
            | Opcode::MathMap
            | Opcode::MathLerp
            | Opcode::MathMin
            | Opcode::MathMax
            | Opcode::MathFmod
            | Opcode::MathPow
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
            | Opcode::MathAtan2
            | Opcode::MathSinh
            | Opcode::MathCosh
            | Opcode::MathTanh
            | Opcode::MathExp
            | Opcode::MathLog
            | Opcode::MathLog10
            | Opcode::MathLdexp
    )
}

/// Two rewrites over the result-lifetime tails the census counts:
///
/// - Target-directed fold: `producer …, a = t; Move d, t; ClearLocalCell t`
///   where the producer's `a` is a pure destination and `t` is an uncaptured,
///   cell-free, afterwards-dead temporary becomes `producer …, a = d`.
/// - Cleanup removal: any remaining `ClearLocalCell` the analysis classifies
///   as pure cleanup of a dead value is deleted.
fn plan_dead_register_tails(program: &Program, function: &Function) -> Result<RewritePlan> {
    let liveness = analyze(program, function)?;
    let mut plan = RewritePlan::new(function.code.len());

    // Target-directed folds first: they consume their own clears.
    for pc in 1..function.code.len().saturating_sub(1) {
        let move_tail = function.code[pc];
        let clear_tail = function.code[pc + 1];
        if move_tail.op != Opcode::Move || clear_tail.op != Opcode::ClearLocalCell {
            continue;
        }
        let temporary = move_tail.b;
        let destination = move_tail.a;
        if clear_tail.a != temporary || destination == temporary {
            continue;
        }
        let producer_pc = pc - 1;
        if plan.touched(producer_pc) || plan.touched(pc) || plan.touched(pc + 1) {
            continue;
        }
        let producer = function.code[producer_pc];
        if !a_is_pure_destination(producer.op) || producer.a != temporary {
            continue;
        }
        let producer_facts = instruction_facts(program, function, producer);
        // The producer must define exactly the temporary; window writers and
        // multi-destination shapes stay untouched.
        let single_write = producer_facts.writes.len() == 1
            && producer_facts.writes.contains(temporary)
            && producer_facts.conditional_writes.is_empty();
        if !single_write {
            continue;
        }
        // Producer, move, and clear must share one block, and the temporary
        // must be an uncaptured, cell-free, afterwards-dead slot.
        let block = liveness.cfg().block_index_of_pc(producer_pc);
        if liveness.cfg().block_index_of_pc(pc + 1) != block {
            continue;
        }
        if liveness.captured_registers().contains(temporary) {
            continue;
        }
        let temporary_dead = liveness
            .live_registers_after(pc + 1)
            .is_some_and(|live| !live.contains(temporary));
        if !temporary_dead {
            continue;
        }
        let cells = liveness.cells_attached_before(pc + 1).unwrap_or_default();
        if cells.contains(temporary) {
            continue;
        }

        let mut retargeted = producer;
        retargeted.a = destination;
        plan.replacements[producer_pc] = Some(retargeted);
        plan.deleted[pc] = true;
        plan.deleted[pc + 1] = true;
    }

    // Remaining dead pure-cleanup clears.
    for site in liveness.clear_sites() {
        if site.kind != ClearKind::PureCleanup || !site.value_dead_after {
            continue;
        }
        if plan.touched(site.pc) {
            continue;
        }
        // Keep the units block-local: some other instruction of the block
        // must survive.
        let block = liveness
            .cfg()
            .block_index_of_pc(site.pc)
            .expect("clear site in range");
        let block = &liveness.cfg().blocks()[block];
        let has_survivor =
            (block.start_pc..block.end_pc).any(|pc| pc != site.pc && !plan.deleted[pc]);
        if !has_survivor {
            continue;
        }
        plan.deleted[site.pc] = true;
    }

    Ok(plan)
}

/// Shrinks the declared register file to the analyzed operand high-water
/// mark. Purely a frame-size reduction: no operand changes.
fn tighten_local_slots(program: &Program, function: &mut Function) -> Result<()> {
    let liveness = analyze(program, function)?;
    let diagnostics = liveness.diagnostics(function);
    let captured_floor = function
        .captured_registers
        .iter()
        .copied()
        .map(|register| usize::from(register) + 1)
        .max()
        .unwrap_or(0);
    let tightened = diagnostics
        .register_high_water
        .max(function.parameter_count)
        .max(captured_floor)
        .max(1);
    if tightened < function.max_registers {
        function.max_registers = tightened;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_disable_every_family() {
        let options = ExperimentalO2Options::default();
        assert!(!options.any_enabled());
        assert!(ExperimentalO2Options::ALL.any_enabled());
    }

    #[test]
    fn pure_destination_table_excludes_read_a_operands() {
        for opcode in [
            Opcode::SetLocalCell,
            Opcode::SetIndex,
            Opcode::SetTableKs,
            Opcode::SetGlobal,
            Opcode::SetUpvalue,
            Opcode::CheckMethod,
            Opcode::JumpIf,
            Opcode::JumpIfNot,
            Opcode::CallMulti,
            Opcode::CallVarargs,
            Opcode::LoadVarargs,
            Opcode::AppendVarargs,
            Opcode::Assert,
            Opcode::Return,
            Opcode::ForPrep,
            Opcode::ForLoop,
        ] {
            assert!(!a_is_pure_destination(opcode), "{opcode:?}");
        }
    }
}
