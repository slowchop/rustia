//! Borrowed execution state for host operations that call back into the VM.
use super::{Execution, Vm, VmExecution, VmHostDispatcher};
use crate::{Function, Instruction, Opcode, Program, Result, Value};

/// Low-level callback scope; never stores borrowed host state in the interpreter.
///
/// The same dispatcher and execution must be used for nested guest calls. This
/// does not change the facade's ordinary callback API or host-panic policy.
#[doc(hidden)]
pub struct VmHostCall<'a> {
    vm: &'a mut Vm,
    execution: &'a mut VmExecution,
    depth: usize,
}

impl<'a> VmHostCall<'a> {
    pub(super) fn new(vm: &'a mut Vm, execution: &'a mut VmExecution, depth: usize) -> Self {
        Self {
            vm,
            execution,
            depth,
        }
    }

    /// Reborrows the current VM, budget and caller depth for one host operation.
    ///
    /// Trusted embedders may temporarily transfer owned VM state at this
    /// boundary, but must restore it and this exact budget before returning.
    /// This method does not itself establish scheduler ownership or permit
    /// concurrent access to the same interpreter.
    pub fn with_parts<R>(
        &mut self,
        operation: impl FnOnce(&mut Vm, &mut VmExecution, usize) -> R,
    ) -> R {
        operation(self.vm, self.execution, self.depth)
    }

    /// Calls guest code with this callback's budget, dispatcher and nesting.
    pub fn call<D: VmHostDispatcher>(
        &mut self,
        dispatcher: &mut D,
        function: Value,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        self.vm.call_retained_function_from_host(
            function,
            args,
            self.execution,
            dispatcher,
            self.depth,
        )
    }
}

impl Vm {
    /// Re-enters from a host boundary using the unchanged outer budget/depth.
    ///
    /// This low-level seam supports owned-state handoff by trusted embedders.
    /// `caller_depth` must be the value supplied by `VmHostCall::with_parts`,
    /// and `state` must be that same execution. Prefer `VmHostCall::call` when
    /// no ownership handoff is required. It does not open a new fuel allowance,
    /// replace the scoped dispatcher or release the outer error traceback.
    #[doc(hidden)]
    pub fn call_retained_function_from_host<D: VmHostDispatcher>(
        &mut self,
        function: Value,
        args: &[Value],
        state: &mut VmExecution,
        dispatcher: &mut D,
        caller_depth: usize,
    ) -> Result<Vec<Value>> {
        self.ensure_healthy()?;
        self.ensure_hardened_transaction(state)?;
        self.seal_hardened_guest_environment()?;
        let depth = caller_depth
            .checked_add(1)
            .ok_or_else(|| self.recursion_limit_error())?;
        self.check_recursion_depth(depth)?;
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            Vec::new(),
        );
        let mut execution = Execution {
            state,
            hosts: dispatcher,
        };
        self.call_value(&program, function, args, depth, &mut execution)
    }
}
