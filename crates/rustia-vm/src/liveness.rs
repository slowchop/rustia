//! Per-function control-flow and liveness analysis over lowered bytecode.
//!
//! This is the C2.3 analysis layer from [`design/58-vm-core2.md`]: it builds
//! basic blocks over one [`Function`], runs a backward register-liveness
//! fixpoint plus a forward local-cell may-attach fixpoint, classifies register
//! slots and cleanup instructions, and exposes the block-boundary summary the
//! planned whole-function executable tape consumes ([`BlockReceipt`]).
//!
//! The analysis is pure inspection: it never changes execution, bytecode, the
//! fuel schedule, or deterministic collection cadence. Sound conservatism is
//! inherited from [`crate::instruction_facts`] — reads over-approximate and
//! definite writes under-approximate — so every "dead" verdict here is safe to
//! act on and every "live" verdict may be pessimistic.
//!
//! # What the three analyses mean
//!
//! - **Register liveness** answers "may this register's *value* still be read
//!   on some forward path". A register live across a `may_enter_guest`
//!   instruction survives calls, metamethods, and coroutine yields; a register
//!   live at a block boundary survives branches.
//! - **Cell may-attach** answers "may this register slot have a materialized
//!   local cell here". Cells attach at `SetLocalCell`/`GetLocalCell` and when
//!   `LoadFunction` captures a parent local mutably; only `ClearLocalCell`
//!   detaches them. A `ClearLocalCell` whose slot may hold a cell is a
//!   *required close/detach* — removing it would let a captured local keep
//!   writing through a stale frame link.
//! - **Classification** joins both: uncaptured single-block temporaries are
//!   the candidates C2.3's default-disabled cleanup removal may touch; clears
//!   of possibly-attached captured slots never are.

use crate::bytecode::{Function, Opcode, Program};
use crate::instruction_facts::{instruction_facts, InstructionFacts, InstructionFlow, RegisterSet};

/// Structured analysis failure. Never a panic: the analysis accepts any public
/// [`Program`], including hand-built ones, and declines malformed control flow
/// with the offending PC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnalysisError {
    pub pc: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "liveness analysis at pc {}: {}",
            self.pc, self.reason
        )
    }
}

impl std::error::Error for AnalysisError {}

/// How a basic block ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockTerminator {
    /// Falls through to the next block only.
    FallThrough,
    /// Unconditional jump.
    Jump,
    /// Conditional: falls through or jumps.
    Branch,
    /// Leaves the frame (`Return`/`ReturnVarargs`/`ReturnCall*`/`Halt`).
    Terminate,
}

/// One effect-boundary-free run of instructions with a single entry and exit.
#[derive(Clone, Debug)]
pub struct BasicBlock {
    /// First instruction PC.
    pub start_pc: usize,
    /// One past the last instruction PC.
    pub end_pc: usize,
    pub terminator: BlockTerminator,
    /// Successor block indices in deterministic (fall-through first) order.
    pub successors: Vec<usize>,
}

impl BasicBlock {
    pub fn len(&self) -> usize {
        self.end_pc - self.start_pc
    }

    pub fn is_empty(&self) -> bool {
        self.start_pc == self.end_pc
    }
}

/// Basic blocks over one function body.
#[derive(Clone, Debug)]
pub struct ControlFlowGraph {
    blocks: Vec<BasicBlock>,
    /// Block index for each PC.
    block_of_pc: Vec<u32>,
}

impl ControlFlowGraph {
    /// Splits `function.code` at jump targets and control-transfer boundaries.
    ///
    /// Jump targets equal to `code.len()` are tolerated (the VM reports a
    /// structured run-off-the-end error there) and simply contribute no
    /// successor edge; anything past that or negative is rejected.
    pub fn build(program: &Program, function: &Function) -> Result<Self, AnalysisError> {
        let code = &function.code;
        if code.is_empty() {
            return Ok(Self {
                blocks: Vec::new(),
                block_of_pc: Vec::new(),
            });
        }

        let mut leaders = vec![false; code.len()];
        leaders[0] = true;
        for (pc, instruction) in code.iter().enumerate() {
            let facts = instruction_facts(program, function, *instruction);
            match facts.flow {
                InstructionFlow::FallThrough => {}
                InstructionFlow::Jump { target } | InstructionFlow::Branch { target } => {
                    let target = validate_target(target, code.len(), pc)?;
                    if target < code.len() {
                        leaders[target] = true;
                    }
                    if pc + 1 < code.len() {
                        leaders[pc + 1] = true;
                    }
                }
                InstructionFlow::Terminate => {
                    if pc + 1 < code.len() {
                        leaders[pc + 1] = true;
                    }
                }
            }
        }

        let mut blocks = Vec::new();
        let mut block_of_pc = vec![0u32; code.len()];
        let mut start = 0;
        for (pc, leader) in leaders.iter().enumerate() {
            if pc > start && *leader {
                push_block(&mut blocks, &mut block_of_pc, start, pc);
                start = pc;
            }
        }
        push_block(&mut blocks, &mut block_of_pc, start, code.len());

        // Terminators and successor edges need the complete block table.
        for index in 0..blocks.len() {
            let last_pc = blocks[index].end_pc - 1;
            let facts = instruction_facts(program, function, code[last_pc]);
            let mut successors = Vec::new();
            let terminator = match facts.flow {
                InstructionFlow::FallThrough => {
                    if blocks[index].end_pc < code.len() {
                        successors.push(block_of_pc[blocks[index].end_pc] as usize);
                    }
                    BlockTerminator::FallThrough
                }
                InstructionFlow::Jump { target } => {
                    let target = validate_target(target, code.len(), last_pc)?;
                    if target < code.len() {
                        successors.push(block_of_pc[target] as usize);
                    }
                    BlockTerminator::Jump
                }
                InstructionFlow::Branch { target } => {
                    if blocks[index].end_pc < code.len() {
                        successors.push(block_of_pc[blocks[index].end_pc] as usize);
                    }
                    let target = validate_target(target, code.len(), last_pc)?;
                    if target < code.len() {
                        let jump_successor = block_of_pc[target] as usize;
                        if successors.first() != Some(&jump_successor) {
                            successors.push(jump_successor);
                        }
                    }
                    BlockTerminator::Branch
                }
                InstructionFlow::Terminate => BlockTerminator::Terminate,
            };
            blocks[index].terminator = terminator;
            blocks[index].successors = successors;
        }

        Ok(Self {
            blocks,
            block_of_pc,
        })
    }

    pub fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }

    pub fn block_index_of_pc(&self, pc: usize) -> Option<usize> {
        self.block_of_pc.get(pc).map(|index| *index as usize)
    }
}

fn push_block(blocks: &mut Vec<BasicBlock>, block_of_pc: &mut [u32], start: usize, end: usize) {
    let index = blocks.len() as u32;
    for entry in &mut block_of_pc[start..end] {
        *entry = index;
    }
    blocks.push(BasicBlock {
        start_pc: start,
        end_pc: end,
        terminator: BlockTerminator::FallThrough,
        successors: Vec::new(),
    });
}

fn validate_target(target: i32, code_len: usize, pc: usize) -> Result<usize, AnalysisError> {
    if target < 0 {
        return Err(AnalysisError {
            pc,
            reason: "negative absolute jump target",
        });
    }
    let target = target as usize;
    if target > code_len {
        return Err(AnalysisError {
            pc,
            reason: "jump target past the end of the function",
        });
    }
    Ok(target)
}

/// Why a `ClearLocalCell` exists, as proven by the analysis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearKind {
    /// The slot may hold an attached cell here: the clear is the required
    /// close/detach for a captured local and must never be removed.
    RequiredDetach,
    /// No cell can be attached: the clear only nils the value slot. Removing
    /// it is semantically invisible except through GC-observable reachability
    /// timing, which is why removal stays gated on C2.2's block receipts.
    PureCleanup,
}

/// One `ClearLocalCell` site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearSite {
    pub pc: usize,
    pub register: u8,
    pub kind: ClearKind,
    /// The cleared register is dead afterwards (its nil is never read), so a
    /// `PureCleanup` clear is a removal candidate once receipts land.
    pub value_dead_after: bool,
}

/// One `Move` site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoveSite {
    pub pc: usize,
    pub target: u8,
    pub source: u8,
    /// The source register dies at this move (its last read), which makes the
    /// move a retargeting candidate for target-directed lowering.
    pub source_dead_after: bool,
}

/// Per-register classification over one function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegisterClass {
    /// Listed in [`Function::captured_registers`]: some closure captures this
    /// slot through a cell somewhere in the function.
    pub captured: bool,
    /// Live across at least one instruction that may enter guest code — the
    /// value survives calls, metamethods, and coroutine yields.
    pub live_across_guest: bool,
    /// Live at some block boundary — the value survives a branch or loop edge.
    pub live_across_block_boundary: bool,
    /// Written or read at all.
    pub used: bool,
}

impl RegisterClass {
    /// An uncaptured slot whose values never survive a block boundary or a
    /// guest entry: the compiler-temporary class whose `Move`/`Clear` tails
    /// C2.3's default-disabled cleanup removal targets.
    pub fn is_uncaptured_temporary(self) -> bool {
        self.used && !self.captured && !self.live_across_guest && !self.live_across_block_boundary
    }
}

/// Static per-function counters for C2.3 planning and reports.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FunctionDiagnostics {
    pub instruction_count: usize,
    /// Declared register file size.
    pub max_registers: usize,
    /// Highest register index any operand can touch, plus one.
    pub register_high_water: usize,
    pub move_count: usize,
    /// `Move` sites whose source dies at the move.
    pub retargetable_move_count: usize,
    pub clear_count: usize,
    /// Clears that are required captured-local detaches.
    pub captured_clear_count: usize,
    /// Clears with no attachable cell.
    pub uncaptured_clear_count: usize,
    /// Uncaptured clears whose value is also dead: removable once block
    /// receipts make the fuel/GC schedule optimization-independent.
    pub removable_clear_candidates: usize,
    /// `operation t, …; Move d, t; ClearLocalCell t` tails that could lower
    /// directly into `d` (target-directed lowering).
    pub target_directed_candidates: usize,
    /// `LoadConst t, k; <binary op … c = t>; ClearLocalCell t` shapes that a
    /// constant/RK operand form would collapse.
    pub constant_operand_candidates: usize,
    /// `LoadConst t, k; JumpIf/JumpIfNot t` shapes a branch folder removes.
    pub branch_fold_candidates: usize,
    /// Declared registers above the high-water mark that slot tightening
    /// reclaims for free.
    pub slot_tightening_headroom: usize,
}

/// Logical-instruction receipts for one function body.
///
/// This is the O2/tape contract: `units[pc]` is how many *logical* source
/// instructions the emitted instruction at `pc` represents. The default
/// compiler emits identity receipts (every unit is 1). A future O2 pass that
/// removes or fuses instructions must fold the removed instructions' units
/// into a surviving neighbor so the per-block sums — and therefore fuel and
/// deterministic-GC scheduling — stay optimization-independent, exactly the
/// C2.2 receipt rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogicalReceipts {
    units: Vec<u32>,
}

impl LogicalReceipts {
    /// Identity receipts for unoptimized code.
    pub fn identity(instruction_count: usize) -> Self {
        Self {
            units: vec![1; instruction_count],
        }
    }

    /// Receipts produced by an optimizing lowering. Fails when the receipt
    /// stream does not cover the function body exactly.
    pub fn from_units(units: Vec<u32>, instruction_count: usize) -> Result<Self, AnalysisError> {
        if units.len() != instruction_count {
            return Err(AnalysisError {
                pc: units.len().min(instruction_count),
                reason: "logical receipt stream does not match the function body",
            });
        }
        Ok(Self { units })
    }

    pub fn unit_at(&self, pc: usize) -> u32 {
        self.units.get(pc).copied().unwrap_or(1)
    }

    /// The full per-instruction unit stream.
    pub fn units(&self) -> &[u32] {
        &self.units
    }

    pub fn logical_len(&self, start_pc: usize, end_pc: usize) -> u64 {
        match self.units.get(start_pc..end_pc) {
            Some(units) => units.iter().map(|unit| u64::from(*unit)).sum(),
            // Out-of-range intervals fall back to the identity price instead
            // of panicking on a malformed caller.
            None => end_pc.saturating_sub(start_pc) as u64,
        }
    }
}

/// Block-boundary summary consumed by the whole-function executable tape.
///
/// One receipt per basic block gives the tape everything C2.2's next slice
/// needs to replace Region/receipt metadata: the source interval, the logical
/// instruction count to advance fuel/GC accounting by, the registers and cells
/// that must be treated as live at entry and exit (root scanning, cleanup
/// verification), and whether the block needs guest-entry safepoints.
#[derive(Clone, Debug)]
pub struct BlockReceipt {
    pub block_index: usize,
    pub start_pc: usize,
    pub end_pc: usize,
    /// Logical instructions this block represents (equals `end_pc - start_pc`
    /// under identity receipts).
    pub logical_instructions: u64,
    pub terminator: BlockTerminator,
    /// Registers whose values may be read on entry.
    pub live_in: RegisterSet,
    /// Registers whose values may be read after the terminator.
    pub live_out: RegisterSet,
    /// Register slots that may hold attached cells on entry.
    pub cell_live_in: RegisterSet,
    /// Register slots that may hold attached cells at exit.
    pub cell_live_out: RegisterSet,
    /// Some instruction inside may enter guest code (call, metamethod, host).
    pub contains_guest_entry: bool,
    /// Some instruction inside may allocate (collection boundary candidate).
    pub contains_allocation: bool,
}

/// Complete liveness analysis of one function.
#[derive(Debug)]
pub struct FunctionLiveness {
    cfg: ControlFlowGraph,
    facts: Vec<InstructionFacts>,
    live_in: Vec<RegisterSet>,
    live_out: Vec<RegisterSet>,
    cell_in: Vec<RegisterSet>,
    cell_out: Vec<RegisterSet>,
    captured: RegisterSet,
    max_registers: usize,
    parameter_count: usize,
}

impl FunctionLiveness {
    /// Analyzes `function` of `program` to a fixpoint.
    pub fn analyze(program: &Program, function: &Function) -> Result<Self, AnalysisError> {
        let cfg = ControlFlowGraph::build(program, function)?;
        let facts: Vec<InstructionFacts> = function
            .code
            .iter()
            .map(|instruction| instruction_facts(program, function, *instruction))
            .collect();

        let block_count = cfg.blocks().len();
        let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); block_count];
        for (index, block) in cfg.blocks().iter().enumerate() {
            for successor in &block.successors {
                predecessors[*successor].push(index);
            }
        }

        // Backward register liveness: per-block use/def, then a worklist
        // fixpoint. Deterministic: the fixpoint is unique regardless of
        // iteration order.
        let mut block_use = vec![RegisterSet::EMPTY; block_count];
        let mut block_def = vec![RegisterSet::EMPTY; block_count];
        for (index, block) in cfg.blocks().iter().enumerate() {
            let mut uses = RegisterSet::EMPTY;
            let mut defs = RegisterSet::EMPTY;
            for pc in (block.start_pc..block.end_pc).rev() {
                let facts = &facts[pc];
                uses = uses.difference(facts.writes);
                uses.union_with(facts.reads);
                defs.union_with(facts.writes);
            }
            block_use[index] = uses;
            block_def[index] = defs;
        }

        let mut live_in = vec![RegisterSet::EMPTY; block_count];
        let mut live_out = vec![RegisterSet::EMPTY; block_count];
        let mut dirty = vec![true; block_count];
        let mut pending = block_count;
        while pending > 0 {
            for index in (0..block_count).rev() {
                if !std::mem::replace(&mut dirty[index], false) {
                    continue;
                }
                let mut out = RegisterSet::EMPTY;
                for successor in &cfg.blocks()[index].successors {
                    out.union_with(live_in[*successor]);
                }
                live_out[index] = out;
                let new_in = {
                    let mut set = out.difference(block_def[index]);
                    set.union_with(block_use[index]);
                    set
                };
                if new_in != live_in[index] {
                    live_in[index] = new_in;
                    for predecessor in &predecessors[index] {
                        if !dirty[*predecessor] {
                            dirty[*predecessor] = true;
                        }
                    }
                }
            }
            pending = dirty.iter().filter(|flag| **flag).count();
        }

        // Forward cell may-attach: cells start detached at frame entry;
        // attaches accumulate and only unconditional ClearLocalCell detaches.
        let mut cell_gen = vec![RegisterSet::EMPTY; block_count];
        let mut cell_kill = vec![RegisterSet::EMPTY; block_count];
        for (index, block) in cfg.blocks().iter().enumerate() {
            let mut gen = RegisterSet::EMPTY;
            let mut kill = RegisterSet::EMPTY;
            for pc in block.start_pc..block.end_pc {
                let facts = &facts[pc];
                gen = gen.difference(facts.cell_detaches);
                kill.union_with(facts.cell_detaches);
                kill = kill.difference(facts.cell_attaches);
                gen.union_with(facts.cell_attaches);
            }
            cell_gen[index] = gen;
            cell_kill[index] = kill;
        }

        let mut cell_in = vec![RegisterSet::EMPTY; block_count];
        let mut cell_out = vec![RegisterSet::EMPTY; block_count];
        let mut dirty = vec![true; block_count];
        let mut pending = block_count;
        while pending > 0 {
            for index in 0..block_count {
                if !std::mem::replace(&mut dirty[index], false) {
                    continue;
                }
                let mut incoming = RegisterSet::EMPTY;
                for predecessor in &predecessors[index] {
                    incoming.union_with(cell_out[*predecessor]);
                }
                cell_in[index] = incoming;
                let mut outgoing = incoming.difference(cell_kill[index]);
                outgoing.union_with(cell_gen[index]);
                if outgoing != cell_out[index] {
                    cell_out[index] = outgoing;
                    for successor in &cfg.blocks()[index].successors {
                        if !dirty[*successor] {
                            dirty[*successor] = true;
                        }
                    }
                }
            }
            pending = dirty.iter().filter(|flag| **flag).count();
        }

        let captured = function.captured_registers.iter().copied().collect();

        Ok(Self {
            cfg,
            facts,
            live_in,
            live_out,
            cell_in,
            cell_out,
            captured,
            max_registers: function.max_registers,
            parameter_count: function.parameter_count,
        })
    }

    pub fn cfg(&self) -> &ControlFlowGraph {
        &self.cfg
    }

    pub fn instruction_facts(&self) -> &[InstructionFacts] {
        &self.facts
    }

    /// Registers captured by nested closures somewhere in the function.
    pub fn captured_registers(&self) -> RegisterSet {
        self.captured
    }

    /// Registers whose values may be read at or after `pc` (before the
    /// instruction at `pc` executes).
    pub fn live_registers_before(&self, pc: usize) -> Option<RegisterSet> {
        let block = self.cfg.block_index_of_pc(pc)?;
        let end_pc = self.cfg.blocks()[block].end_pc;
        let mut live = self.live_out[block];
        for walk_pc in (pc..end_pc).rev() {
            let facts = &self.facts[walk_pc];
            live = live.difference(facts.writes);
            live.union_with(facts.reads);
        }
        Some(live)
    }

    /// Registers whose values may be read strictly after `pc`.
    pub fn live_registers_after(&self, pc: usize) -> Option<RegisterSet> {
        let block = self.cfg.block_index_of_pc(pc)?;
        let block_ref = &self.cfg.blocks()[block];
        if pc + 1 < block_ref.end_pc {
            self.live_registers_before(pc + 1)
        } else {
            Some(self.live_out[block])
        }
    }

    /// Register slots that may hold an attached cell before `pc` executes.
    pub fn cells_attached_before(&self, pc: usize) -> Option<RegisterSet> {
        let block = self.cfg.block_index_of_pc(pc)?;
        let block_ref = &self.cfg.blocks()[block];
        let mut attached = self.cell_in[block];
        for walk_pc in block_ref.start_pc..pc {
            let facts = &self.facts[walk_pc];
            attached = attached.difference(facts.cell_detaches);
            attached.union_with(facts.cell_attaches);
        }
        Some(attached)
    }

    /// Per-register classification for the whole function.
    pub fn register_classes(&self) -> Vec<RegisterClass> {
        let mut classes = vec![RegisterClass::default(); self.max_registers.min(256)];
        for register in self.captured.iter() {
            if let Some(class) = classes.get_mut(usize::from(register)) {
                class.captured = true;
            }
        }
        // Parameters arrive live from the caller.
        for register in 0..self.parameter_count.min(classes.len()) {
            classes[register].used = true;
        }
        for (index, _block) in self.cfg.blocks().iter().enumerate() {
            for register in self.live_in[index].iter() {
                if index != 0 {
                    if let Some(class) = classes.get_mut(usize::from(register)) {
                        class.live_across_block_boundary = true;
                    }
                }
            }
        }
        for (pc, facts) in self.facts.iter().enumerate() {
            for register in facts.reads.iter().chain(facts.writes.iter()) {
                if let Some(class) = classes.get_mut(usize::from(register)) {
                    class.used = true;
                }
            }
            if facts.may_enter_guest {
                if let Some(live) = self.live_registers_after(pc) {
                    // A value crosses the guest entry when it is live after
                    // the instruction and not produced by it: results written
                    // by the call itself do not survive *across* it.
                    let crossing = live
                        .difference(facts.writes)
                        .difference(facts.conditional_writes);
                    for register in crossing.iter() {
                        if let Some(class) = classes.get_mut(usize::from(register)) {
                            class.live_across_guest = true;
                        }
                    }
                }
            }
        }
        classes
    }

    /// Every `ClearLocalCell` site, classified.
    pub fn clear_sites(&self) -> Vec<ClearSite> {
        let mut sites = Vec::new();
        for (pc, facts) in self.facts.iter().enumerate() {
            // ClearLocalCell is the only opcode with an unconditional detach.
            let Some(register) = facts.cell_detaches.iter().next() else {
                continue;
            };
            let may_have_cell = self
                .cells_attached_before(pc)
                .is_some_and(|cells| cells.contains(register));
            let value_dead_after = self
                .live_registers_after(pc)
                .is_some_and(|live| !live.contains(register));
            sites.push(ClearSite {
                pc,
                register,
                kind: if may_have_cell {
                    ClearKind::RequiredDetach
                } else {
                    ClearKind::PureCleanup
                },
                value_dead_after,
            });
        }
        sites
    }

    /// Every `Move` site with the source-death verdict.
    pub fn move_sites(&self, function: &Function) -> Vec<MoveSite> {
        let mut sites = Vec::new();
        for (pc, instruction) in function.code.iter().enumerate() {
            if instruction.op != Opcode::Move {
                continue;
            }
            let source_dead_after = self
                .live_registers_after(pc)
                .is_some_and(|live| !live.contains(instruction.b));
            sites.push(MoveSite {
                pc,
                target: instruction.a,
                source: instruction.b,
                source_dead_after,
            });
        }
        sites
    }

    /// Static counters for planning and the C2.3 census reports.
    pub fn diagnostics(&self, function: &Function) -> FunctionDiagnostics {
        let mut diagnostics = FunctionDiagnostics {
            instruction_count: function.code.len(),
            max_registers: function.max_registers,
            ..FunctionDiagnostics::default()
        };

        let mut high_water = self.parameter_count;
        for facts in &self.facts {
            for register in facts
                .reads
                .iter()
                .chain(facts.writes.iter())
                .chain(facts.conditional_writes.iter())
            {
                high_water = high_water.max(usize::from(register) + 1);
            }
        }
        diagnostics.register_high_water = high_water;
        diagnostics.slot_tightening_headroom = function.max_registers.saturating_sub(high_water);

        for site in self.move_sites(function) {
            diagnostics.move_count += 1;
            if site.source_dead_after {
                diagnostics.retargetable_move_count += 1;
            }
        }

        for site in self.clear_sites() {
            diagnostics.clear_count += 1;
            match site.kind {
                ClearKind::RequiredDetach => diagnostics.captured_clear_count += 1,
                ClearKind::PureCleanup => {
                    diagnostics.uncaptured_clear_count += 1;
                    if site.value_dead_after {
                        diagnostics.removable_clear_candidates += 1;
                    }
                }
            }
        }

        for pc in 0..function.code.len() {
            if self.is_target_directed_candidate(function, pc) {
                diagnostics.target_directed_candidates += 1;
            }
            if self.is_constant_operand_candidate(function, pc) {
                diagnostics.constant_operand_candidates += 1;
            }
            if self.is_branch_fold_candidate(function, pc) {
                diagnostics.branch_fold_candidates += 1;
            }
        }

        diagnostics
    }

    /// `Move d, t; ClearLocalCell t` at `pc` with an uncaptured, cell-free,
    /// afterwards-dead `t`: the exact result-lifetime tail the compiler emits
    /// after expression lowering (the C2.2 six-operation suffix). Every such
    /// pair marks a producer that target-directed lowering could have pointed
    /// at `d` directly.
    fn is_target_directed_candidate(&self, function: &Function, pc: usize) -> bool {
        let Some([move_tail, clear_tail]) = function
            .code
            .get(pc..pc + 2)
            .map(|window| [window[0], window[1]])
        else {
            return false;
        };
        let temporary = move_tail.b;
        move_tail.op == Opcode::Move
            && move_tail.a != temporary
            && clear_tail.op == Opcode::ClearLocalCell
            && clear_tail.a == temporary
            // Straight-line only: the pair must sit in one block.
            && self.cfg.block_index_of_pc(pc) == self.cfg.block_index_of_pc(pc + 1)
            && !self.captured.contains(temporary)
            && self
                .live_registers_after(pc + 1)
                .is_some_and(|live| !live.contains(temporary))
            && self
                .cells_attached_before(pc + 1)
                .is_some_and(|cells| !cells.contains(temporary))
    }

    /// `LoadConst t, k` at `pc`, immediately consumed as the `c` operand of a
    /// binary arithmetic/comparison/index instruction, with `t` dead after its
    /// cleanup: a constant/RK operand form collapses the triple.
    fn is_constant_operand_candidate(&self, function: &Function, pc: usize) -> bool {
        let Some([load, operation]) = function
            .code
            .get(pc..pc + 2)
            .map(|window| [window[0], window[1]])
        else {
            return false;
        };
        if load.op != Opcode::LoadConst && load.op != Opcode::LoadConstWide {
            return false;
        }
        let temporary = load.a;
        let consumes_constant = matches!(
            operation.op,
            Opcode::Add
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
                | Opcode::GetIndex
        ) && operation.c == temporary
            && operation.a != temporary
            && operation.b != temporary;
        consumes_constant
            && self.cfg.block_index_of_pc(pc) == self.cfg.block_index_of_pc(pc + 1)
            && !self.captured.contains(temporary)
            && self
                .live_registers_after(pc + 1)
                .is_some_and(|live| !live.contains(temporary))
    }

    /// `LoadConst t, k; JumpIf/JumpIfNot t` with `t` dead after the branch:
    /// the branch condition is known at compile time.
    fn is_branch_fold_candidate(&self, function: &Function, pc: usize) -> bool {
        let Some([load, branch]) = function
            .code
            .get(pc..pc + 2)
            .map(|window| [window[0], window[1]])
        else {
            return false;
        };
        (load.op == Opcode::LoadConst || load.op == Opcode::LoadConstWide)
            && matches!(branch.op, Opcode::JumpIf | Opcode::JumpIfNot)
            && branch.a == load.a
            && !self.captured.contains(load.a)
            && self
                .live_registers_after(pc + 1)
                .is_some_and(|live| !live.contains(load.a))
    }

    /// The block-boundary summary the executable tape consumes, priced with
    /// identity logical receipts.
    pub fn block_receipts(&self) -> Vec<BlockReceipt> {
        let total = self.cfg.blocks().last().map_or(0, |block| block.end_pc);
        self.block_receipts_with(&LogicalReceipts::identity(total))
    }

    /// The same summary priced with an explicit receipt stream, for lowering
    /// modes that fuse or remove instructions.
    pub fn block_receipts_with(&self, receipts: &LogicalReceipts) -> Vec<BlockReceipt> {
        self.cfg
            .blocks()
            .iter()
            .enumerate()
            .map(|(index, block)| {
                let mut contains_guest_entry = false;
                let mut contains_allocation = false;
                for pc in block.start_pc..block.end_pc {
                    contains_guest_entry |= self.facts[pc].may_enter_guest;
                    contains_allocation |= self.facts[pc].may_allocate;
                }
                BlockReceipt {
                    block_index: index,
                    start_pc: block.start_pc,
                    end_pc: block.end_pc,
                    logical_instructions: receipts.logical_len(block.start_pc, block.end_pc),
                    terminator: block.terminator,
                    live_in: self.live_in[index],
                    live_out: self.live_out[index],
                    cell_live_in: self.cell_in[index],
                    cell_live_out: self.cell_out[index],
                    contains_guest_entry,
                    contains_allocation,
                }
            })
            .collect()
    }
}

/// Analysis of every function in a program, main first.
#[derive(Debug)]
pub struct ProgramLiveness {
    pub main: FunctionLiveness,
    pub functions: Vec<FunctionLiveness>,
}

impl ProgramLiveness {
    pub fn analyze(program: &Program) -> Result<Self, AnalysisError> {
        let main = FunctionLiveness::analyze(program, &program.main)?;
        let functions = program
            .functions
            .iter()
            .map(|function| FunctionLiveness::analyze(program, function))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { main, functions })
    }

    /// Diagnostics summed across main and every prototype.
    pub fn total_diagnostics(&self, program: &Program) -> FunctionDiagnostics {
        let mut total = self.main.diagnostics(&program.main);
        for (liveness, function) in self.functions.iter().zip(&program.functions) {
            let diagnostics = liveness.diagnostics(function);
            total.instruction_count += diagnostics.instruction_count;
            total.max_registers = total.max_registers.max(diagnostics.max_registers);
            total.register_high_water = total
                .register_high_water
                .max(diagnostics.register_high_water);
            total.move_count += diagnostics.move_count;
            total.retargetable_move_count += diagnostics.retargetable_move_count;
            total.clear_count += diagnostics.clear_count;
            total.captured_clear_count += diagnostics.captured_clear_count;
            total.uncaptured_clear_count += diagnostics.uncaptured_clear_count;
            total.removable_clear_candidates += diagnostics.removable_clear_candidates;
            total.target_directed_candidates += diagnostics.target_directed_candidates;
            total.constant_operand_candidates += diagnostics.constant_operand_candidates;
            total.branch_fold_candidates += diagnostics.branch_fold_candidates;
            total.slot_tightening_headroom += diagnostics.slot_tightening_headroom;
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};

    fn program(code: Vec<Instruction>, max_registers: usize) -> Program {
        Program::new(
            Function::new(code, max_registers),
            vec![Constant::Number(1.0), Constant::Boolean(true)],
        )
    }

    fn analyze(program: &Program) -> FunctionLiveness {
        FunctionLiveness::analyze(program, &program.main).expect("analysis succeeds")
    }

    #[test]
    fn straight_line_code_is_one_block() {
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Move, 1, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        );
        let liveness = analyze(&program);
        assert_eq!(liveness.cfg().blocks().len(), 1);
        assert_eq!(
            liveness.cfg().blocks()[0].terminator,
            BlockTerminator::Terminate
        );
        // Register 0 dies at the move; register 1 is live until the return.
        assert!(liveness
            .live_registers_after(1)
            .is_some_and(|live| live.contains(1) && !live.contains(0)));
    }

    #[test]
    fn branches_split_blocks_and_merge_liveness() {
        // 0: LoadConst r0, k0
        // 1: JumpIfNot r0 -> 4
        // 2: Move r1, r0
        // 3: Jump -> 5
        // 4: LoadConst r1, k1
        // 5: Return r1, 1
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::with_aux(Opcode::JumpIfNot, 0, 0, 0, 4),
                Instruction::new(Opcode::Move, 1, 0, 0),
                Instruction::with_aux(Opcode::Jump, 0, 0, 0, 5),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        );
        let liveness = analyze(&program);
        let blocks = liveness.cfg().blocks();
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].terminator, BlockTerminator::Branch);
        assert_eq!(blocks[0].successors, vec![1, 2]);
        // r0 is live into the true branch (block 1) but not into block 2.
        let receipts = liveness.block_receipts();
        assert!(receipts[1].live_in.contains(0));
        assert!(!receipts[2].live_in.contains(0));
        // r1 is live into the final block from both predecessors.
        assert!(receipts[3].live_in.contains(1));
        assert_eq!(receipts[3].logical_instructions, 1);
    }

    #[test]
    fn loop_backedges_keep_accumulators_live() {
        // Numeric-for shape: r0..r2 control, r3 visible, r4 accumulator.
        // 0: LoadConst r4, k0        (acc)
        // 1: LoadConst r0, k0        (index)
        // 2: LoadConst r1, k0        (limit)
        // 3: LoadConst r2, k0        (step)
        // 4: ForPrep r0 -> 8
        // 5: Add r4, r4, r3
        // 6: ForLoop r0 -> 5
        // 7: Jump -> 8 (unreachable filler)
        // 8: Return r4, 1
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 4, 0, 0),
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 0),
                Instruction::with_aux(Opcode::ForPrep, 0, 0, 0, 8),
                Instruction::new(Opcode::Add, 4, 4, 3),
                Instruction::with_aux(Opcode::ForLoop, 0, 0, 0, 5),
                Instruction::with_aux(Opcode::Jump, 0, 0, 0, 8),
                Instruction::new(Opcode::Return, 4, 1, 0),
            ],
            5,
        );
        let liveness = analyze(&program);
        // The accumulator and the loop-control window survive the backedge.
        let body_block = liveness.cfg().block_index_of_pc(5).unwrap();
        let receipts = liveness.block_receipts();
        for register in [0, 1, 2, 4] {
            assert!(
                receipts[body_block].live_in.contains(register),
                "register {register} should be live into the loop body"
            );
        }
        let classes = liveness.register_classes();
        assert!(classes[4].live_across_block_boundary);
        // The loop-control window survives the Add's potential metamethod;
        // the accumulator's old value is consumed by the Add itself.
        assert!(classes[0].live_across_guest);
        assert!(!classes[4].live_across_guest);
        assert!(!classes[4].is_uncaptured_temporary());
    }

    #[test]
    fn uncaptured_move_clear_tail_is_a_removal_candidate() {
        // The exact C2.2 six-operation lifetime shape:
        // Move t, lhs; Move u, rhs; Add t, t, u; Clear u; Move d, t; Clear t.
        let program = program(
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
        );
        let liveness = analyze(&program);
        let clears = liveness.clear_sites();
        assert_eq!(clears.len(), 2);
        for clear in &clears {
            assert_eq!(clear.kind, ClearKind::PureCleanup);
            assert!(clear.value_dead_after);
        }
        let diagnostics = liveness.diagnostics(&program.main);
        assert_eq!(diagnostics.clear_count, 2);
        assert_eq!(diagnostics.uncaptured_clear_count, 2);
        assert_eq!(diagnostics.removable_clear_candidates, 2);
        assert_eq!(diagnostics.captured_clear_count, 0);
        // `Add t; Move d, t; Clear t` is one target-directed candidate.
        assert_eq!(diagnostics.target_directed_candidates, 1);
        // Both temporaries are uncaptured single-block slots.
        let classes = liveness.register_classes();
        assert!(classes[3].is_uncaptured_temporary());
    }

    #[test]
    fn attached_cells_make_clears_required_detaches() {
        // SetLocalCell attaches a cell to r0; the later clear must detach it.
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::SetLocalCell, 0, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 0, 0),
            ],
            1,
        );
        let liveness = analyze(&program);
        let clears = liveness.clear_sites();
        assert_eq!(clears.len(), 1);
        assert_eq!(clears[0].kind, ClearKind::RequiredDetach);
        let diagnostics = liveness.diagnostics(&program.main);
        assert_eq!(diagnostics.captured_clear_count, 1);
        assert_eq!(diagnostics.removable_clear_candidates, 0);
    }

    #[test]
    fn cell_attachment_flows_across_branches() {
        // A cell attached in one branch keeps the post-join clear required.
        // 0: LoadConst r0
        // 1: JumpIfNot r0 -> 3
        // 2: SetLocalCell r0, r0
        // 3: ClearLocalCell r0
        // 4: Return r0, 0
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::with_aux(Opcode::JumpIfNot, 0, 0, 0, 3),
                Instruction::new(Opcode::SetLocalCell, 0, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 0, 0),
            ],
            1,
        );
        let liveness = analyze(&program);
        let clears = liveness.clear_sites();
        assert_eq!(clears.len(), 1);
        assert_eq!(clears[0].kind, ClearKind::RequiredDetach);
    }

    #[test]
    fn closure_capture_attaches_the_parent_cell() {
        let prototype = Function::with_params_and_upvalues(
            vec![Instruction::new(Opcode::Return, 0, 0, 0)],
            1,
            0,
            vec![crate::bytecode::UpvalueDescriptor::ParentLocal(0)],
            Vec::new(),
        );
        let mut main = Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::with_aux(Opcode::LoadFunction, 1, 0, 0, 0),
                Instruction::new(Opcode::ClearLocalCell, 0, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        );
        main.captured_registers = vec![0];
        let program = Program::with_functions(main, vec![Constant::Number(1.0)], vec![prototype]);
        let liveness = analyze(&program);
        let clears = liveness.clear_sites();
        assert_eq!(clears.len(), 1);
        assert_eq!(clears[0].register, 0);
        assert_eq!(clears[0].kind, ClearKind::RequiredDetach);
        assert!(liveness.captured_registers().contains(0));
        let classes = liveness.register_classes();
        assert!(classes[0].captured);
        assert!(!classes[0].is_uncaptured_temporary());
    }

    #[test]
    fn call_boundaries_mark_surviving_registers() {
        // r0 = callee, r1 = argument, r2 survives the call.
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 0),
                Instruction::with_aux(Opcode::Call, 3, 0, 1, 1),
                Instruction::new(Opcode::Add, 3, 3, 2),
                Instruction::new(Opcode::Return, 3, 1, 0),
            ],
            4,
        );
        let liveness = analyze(&program);
        let classes = liveness.register_classes();
        assert!(classes[2].live_across_guest);
        assert!(!classes[1].live_across_guest);
        assert!(!classes[2].is_uncaptured_temporary());
    }

    #[test]
    fn constant_operand_and_branch_fold_candidates_are_counted() {
        // LoadConst t; Add d, b, t; Clear t  → constant-operand candidate.
        // LoadConst t; JumpIfNot t → branch-fold candidate.
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 2, 0, 0),
                Instruction::new(Opcode::Add, 1, 0, 2),
                Instruction::new(Opcode::ClearLocalCell, 2, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 1),
                Instruction::with_aux(Opcode::JumpIfNot, 2, 0, 0, 6),
                Instruction::new(Opcode::Move, 1, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            3,
        );
        let liveness = analyze(&program);
        let diagnostics = liveness.diagnostics(&program.main);
        assert_eq!(diagnostics.constant_operand_candidates, 1);
        assert_eq!(diagnostics.branch_fold_candidates, 1);
    }

    #[test]
    fn malformed_jump_targets_are_structured_errors() {
        let negative = program(vec![Instruction::with_aux(Opcode::Jump, 0, 0, 0, -1)], 1);
        let error = FunctionLiveness::analyze(&negative, &negative.main).unwrap_err();
        assert_eq!(error.pc, 0);

        let past_end = program(vec![Instruction::with_aux(Opcode::Jump, 0, 0, 0, 9)], 1);
        assert!(FunctionLiveness::analyze(&past_end, &past_end.main).is_err());

        // A jump to exactly code.len() is tolerated with no successor, the
        // same boundary the VM reports as running off the end.
        let to_end = program(vec![Instruction::with_aux(Opcode::Jump, 0, 0, 0, 1)], 1);
        let liveness = analyze(&to_end);
        assert!(liveness.cfg().blocks()[0].successors.is_empty());
    }

    #[test]
    fn empty_function_analyzes_to_nothing() {
        let program = program(Vec::new(), 0);
        let liveness = analyze(&program);
        assert!(liveness.cfg().blocks().is_empty());
        assert!(liveness.block_receipts().is_empty());
        let diagnostics = liveness.diagnostics(&program.main);
        assert_eq!(diagnostics.instruction_count, 0);
    }

    #[test]
    fn logical_receipts_price_blocks_and_reject_mismatches() {
        let program = program(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Move, 1, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        );
        let liveness = analyze(&program);
        let identity = liveness.block_receipts();
        assert_eq!(identity[0].logical_instructions, 3);

        // A fused lowering that folded one instruction into a neighbor keeps
        // the block's logical sum intact.
        let receipts = LogicalReceipts::from_units(vec![2, 0, 1], 3).unwrap();
        let priced = liveness.block_receipts_with(&receipts);
        assert_eq!(priced[0].logical_instructions, 3);

        assert!(LogicalReceipts::from_units(vec![1, 1], 3).is_err());
    }

    #[test]
    fn program_liveness_covers_every_prototype() {
        let prototype = Function::with_params(
            vec![
                Instruction::new(Opcode::Move, 1, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
            1,
        );
        let program = Program::with_functions(
            Function::new(
                vec![
                    Instruction::with_aux(Opcode::LoadFunction, 0, 0, 0, 0),
                    Instruction::new(Opcode::Return, 0, 1, 0),
                ],
                1,
            ),
            Vec::new(),
            vec![prototype],
        );
        let liveness = ProgramLiveness::analyze(&program).expect("program analysis");
        assert_eq!(liveness.functions.len(), 1);
        let total = liveness.total_diagnostics(&program);
        assert_eq!(total.instruction_count, 4);
        assert_eq!(total.move_count, 1);
    }
}
