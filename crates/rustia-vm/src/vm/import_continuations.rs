//! A fused import continues at its next path lane after an implicit callback.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct ImportState {
    components: [u32; IMPORT_CACHE_LANES],
    len: usize,
    next: usize,
}
impl ImportState {
    pub(super) fn new(program: &Program, instruction: Instruction) -> Result<Self> {
        if instruction.aux < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "negative import descriptor index",
            }));
        }
        if instruction.c != 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "import instruction has non-zero reserved operand",
            }));
        }
        let path_len = usize::from(instruction.b);
        if !(1..=IMPORT_CACHE_LANES).contains(&path_len) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "import path length must be between one and three",
            }));
        }
        let descriptor_index = instruction.aux as usize;
        let descriptor = program.constants().get(descriptor_index).ok_or_else(|| {
            Error::new(ErrorKind::ConstantOutOfBounds {
                index: descriptor_index,
                len: program.constants().len(),
            })
        })?;
        let Constant::Import(path) = descriptor else {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "import instruction constant must be an import descriptor",
            }));
        };
        if path.len() != path_len {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "import descriptor length does not match instruction",
            }));
        }
        for component in path {
            let index = usize::try_from(*component).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "import component index overflow",
                })
            })?;
            if !matches!(program.constants().get(index), Some(Constant::String(_))) {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "import components must reference string constants",
                }));
            }
        }

        let mut components = [0; IMPORT_CACHE_LANES];
        components[..path_len].copy_from_slice(path);
        Ok(Self {
            components,
            len: path_len,
            next: 0,
        })
    }
}
impl Vm {
    pub(super) fn continue_import(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        pc: usize,
        state: &mut ImportState,
        mut object: Value,
        depth: usize,
        fuel: &mut Execution<'_>,
    ) -> Result<ImplicitStep<Value>> {
        while state.next < state.len {
            let lane = state.next;
            let component = state.components[lane];
            if lane > 0 {
                self.advance_fused_instruction_steps_with_root(1, object)?;
            }
            let component = usize::try_from(component).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "import component index overflow",
                })
            })?;
            let atom = self
                .constant_string_atom(program, program_id, component)?
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "import component string atom is missing",
                    })
                })?;
            if lane > 0 {
                self.advance_fused_instruction_steps_with_root(1, object)?;
            }
            let step = self.get_index_value_with_import_inline_cache_step(
                program,
                program_id,
                prototype,
                pc,
                lane,
                object,
                Value::string_atom(atom),
                depth,
                fuel,
            )?;
            // The lookup has already selected its callback. Never replay it
            // when that callback changes a global, metatable or receiver.
            state.next += 1;
            match step {
                ImplicitStep::Ready(value) => object = value,
                ImplicitStep::Call(request) => return Ok(ImplicitStep::Call(request)),
            }
        }
        Ok(ImplicitStep::Ready(object))
    }
}
