//! Host-resolved module loading. Loading ownership survives suspension without
//! keeping a whole-VM transaction open across other coroutine execution.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(super) struct ModuleLoad {
    pub(super) identity: String,
    // An identifier, not a GC root: abandoning a coroutine abandons its loads.
    pub(super) owner: Option<u32>,
}

#[derive(Clone)]
pub(super) struct SuspendedRequireFrame {
    pub(super) cohort: u32,
    pub(super) load: ModuleLoad,
}

pub(super) fn try_clone_module_load(load: &ModuleLoad) -> Result<ModuleLoad> {
    let mut identity = String::new();
    identity
        .try_reserve_exact(load.identity.len())
        .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
    identity.push_str(&load.identity);
    Ok(ModuleLoad {
        identity,
        owner: load.owner,
    })
}

pub(super) fn try_clone_module_loads(loads: &[ModuleLoad]) -> Result<Vec<ModuleLoad>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(loads.len())
        .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
    for load in loads {
        result.push(try_clone_module_load(load)?);
    }
    Ok(result)
}

impl Vm {
    pub(super) fn native_require_resumable(
        &mut self,
        args: &[Value],
        requiring_program: &Program,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Execution<'_>,
    ) -> VmFlowResult<Vec<Value>> {
        self.check_recursion_depth(depth)?;
        validate_native_arity(args, 1, 1, "require arguments")?;
        let requested_bytes = self.value_to_string(args[0])?;
        let requested = std::str::from_utf8(&requested_bytes).map_err(|_| {
            Error::new(ErrorKind::RuntimeError {
                message: "require path must be valid UTF-8".to_owned(),
            })
        })?;
        let resolver = self.settings.module_resolver.clone().ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: "require resolver is not configured".to_owned(),
            })
        })?;
        let referrer = requiring_program.source_name.as_deref();
        let profile = self.profile;
        let resolved = self
            .catch_host_callback(MODULE_RESOLVER_SERVICE_NAME, || {
                resolver.resolve(requested, referrer, profile)
            })
            .map_err(|error| {
                self.annotate_host_boundary(error, requiring_program, call_site.as_ref())
            })?;
        let resolved = resolved.map_err(|mut error| {
            if let ErrorKind::RuntimeError { message } = &mut error.kind {
                *message = format!("error requiring module {requested:?}: {message}");
            }
            if let Some(diagnostic) = &mut error.diagnostic {
                self.admit_host_diagnostic(Arc::make_mut(diagnostic));
            }
            error
        })?;
        if resolved.identity.is_empty() {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "error requiring module {requested:?}: resolver returned an empty identity"
                ),
            }));
        }
        let cohort_id = self.current_module_cohort();
        let environment = self.current_environment()?;
        let owner = self
            .running_coroutines
            .last()
            .map(|index| self.coroutines[*index].handle);
        let cohort = self.module_cohorts.get(&cohort_id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "module cache cohort is missing",
            })
            .without_source_location()
        })?;
        if let Some(cached) = cohort.cache.get(&resolved.identity).copied() {
            return Ok(VmFlow::Return(cached.into_iter().collect()));
        }
        if let Some(start) = cohort
            .loading
            .iter()
            .position(|load| load.identity == resolved.identity)
        {
            let loading = &cohort.loading[start];
            let message = if loading.owner == owner {
                let mut cycle = cohort.loading[start..]
                    .iter()
                    .filter(|load| load.owner == owner)
                    .map(|load| load.identity.as_str())
                    .collect::<Vec<_>>();
                cycle.push(&resolved.identity);
                format!(
                    "requiring a module that is in the process of being required: {}",
                    cycle.join(" -> ")
                )
            } else {
                format!(
                    "module {:?} is already being required by another coroutine",
                    resolved.identity
                )
            };
            return Err(Error::new(ErrorKind::RuntimeError { message }));
        }
        // The loading registry and suspended continuation own separate identity
        // strings. Admit both before marking the identity in progress.
        self.ensure_memory_growth(
            size_of::<ModuleLoad>().saturating_add(resolved.identity.len().saturating_mul(2)),
            &[],
        )?;
        let load = ModuleLoad {
            identity: resolved.identity,
            owner,
        };
        let registry_load = try_clone_module_load(&load)?;
        let cohort = self.module_cohorts.get_mut(&cohort_id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "module cache cohort is missing",
            })
            .without_source_location()
        })?;
        cohort
            .loading
            .try_reserve(1)
            .map_err(|_| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
        cohort.loading.push(registry_load);
        self.mark_control_memory_dirty();
        let frame = SuspendedRequireFrame {
            cohort: cohort_id,
            load,
        };
        let outcome = (|| {
            let program_id = self.retain_dynamic_program(resolved.program)?;
            let handle = self
                .heap
                .alloc_dynamic_main_function_in_cohort(program_id, environment, cohort_id)
                .ok_or_else(|| Error::new(ErrorKind::OutOfMemory).without_source_location())?;
            self.call_value_with_site_resumable(
                requiring_program,
                Value::function(handle),
                &[],
                depth.saturating_add(1),
                call_site,
                fuel,
            )
        })();
        match outcome {
            Ok(VmFlow::Return(results)) => self
                .finish_required_module(frame, results.into_vec())
                .map(VmFlow::Return),
            Ok(VmFlow::Yield(mut suspension)) => {
                self.push_coroutine_frame(&mut suspension, CoroutineFrame::Require(frame))?;
                Ok(VmFlow::Yield(suspension))
            }
            Err(error) => {
                self.release_module_load(&frame);
                Err(error)
            }
        }
    }

    pub(super) fn finish_required_module(
        &mut self,
        frame: SuspendedRequireFrame,
        results: Vec<Value>,
    ) -> Result<Vec<Value>> {
        let owns_load = self
            .module_cohorts
            .get(&frame.cohort)
            .is_some_and(|cohort| {
                cohort.loading.iter().any(|load| {
                    load.owner == frame.load.owner && load.identity == frame.load.identity
                })
            });
        if !owns_load {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "required module continuation does not own its loading identity",
            })
            .without_source_location());
        }
        self.release_module_load(&frame);
        if results.len() > 1 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "module must return a single value".to_owned(),
            }));
        }
        let value = results.first().copied();
        let growth = frame
            .load
            .identity
            .len()
            .saturating_add(size_of::<Option<Value>>())
            .saturating_add(size_of::<usize>().saturating_mul(4));
        let roots = value.into_iter().collect::<SmallVec<[Value; 1]>>();
        self.ensure_memory_growth(growth, &roots)?;
        self.module_cohorts
            .get_mut(&frame.cohort)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "module cache cohort is missing",
                })
                .without_source_location()
            })?
            .cache
            .insert(frame.load.identity, value);
        self.mark_control_memory_dirty();
        Ok(results)
    }

    pub(super) fn release_module_load(&mut self, frame: &SuspendedRequireFrame) {
        if let Some(cohort) = self.module_cohorts.get_mut(&frame.cohort) {
            cohort.loading.retain(|load| {
                load.owner != frame.load.owner || load.identity != frame.load.identity
            });
            self.mark_control_memory_dirty();
        }
    }

    pub(super) fn release_require_frames(&mut self, frames: &[CoroutineFrame]) {
        for frame in frames {
            if let CoroutineFrame::Require(frame) = frame {
                self.release_module_load(frame);
            }
        }
    }

    pub(super) fn push_coroutine_frame(
        &mut self,
        suspension: &mut CoroutineYield,
        frame: CoroutineFrame,
    ) -> Result<()> {
        if suspension.frames.try_reserve(1).is_err() {
            self.release_require_frames(&suspension.frames);
            self.release_require_frames(std::slice::from_ref(&frame));
            return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
        }
        suspension.frames.push(frame);
        Ok(())
    }

    pub(super) fn release_module_owner(&mut self, owner: u32) {
        for cohort in self.module_cohorts.values_mut() {
            cohort.loading.retain(|load| load.owner != Some(owner));
        }
        self.mark_control_memory_dirty();
    }

    pub(super) fn prune_abandoned_module_loads(&mut self) {
        let heap = &mut self.heap;
        let mut removed = false;
        for cohort in self.module_cohorts.values_mut() {
            let before = cohort.loading.len();
            cohort
                .loading
                .retain(|load| load.owner.is_none_or(|owner| heap.has_userdata(owner)));
            removed |= before != cohort.loading.len();
        }
        if removed {
            self.mark_control_memory_dirty();
        }
    }
}
