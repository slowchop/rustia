use rustia::{Function, Rustia, ServiceModuleResolver, Table, Value};

pub struct Garden {
    vm: Rustia,
    update: Option<Function>,
    legacy_frame: bool,
    frame_arguments: Table,
    save_flowers: Function,
}

impl Garden {
    pub fn load(
        source: &str,
        drawing: &str,
        flowerbed: &str,
        previous: Vec<f64>,
        radius: Option<f64>,
    ) -> rustia::Result<Self> {
        Self::load_with_host(source, drawing, flowerbed, previous, radius, None)
    }

    pub(crate) fn load_with_host(
        source: &str,
        drawing: &str,
        flowerbed: &str,
        previous: Vec<f64>,
        radius: Option<f64>,
        project: Option<(&str, ServiceModuleResolver)>,
    ) -> rustia::Result<Self> {
        // Each edit gets its own interpreter. Fuel limits how long a script can loop.
        let builder = Rustia::builder()
            .without_stdout()
            .fuel(120_000)
            .memory_limit_bytes(8 * 1024 * 1024);
        let interactive_project = project.is_some();
        let entry = project.as_ref().map_or("garden.luau", |(name, _)| *name);
        let mut vm = match project {
            Some((_, resolver)) => builder
                .module_resolver(resolver)
                .stdout(crate::playground::write),
            None => builder,
        }
        .build()?;

        // The examples use tau as a convenience. Keep this alias local to the browser host.
        let math = vm
            .load("local m = table.clone(math); m.tau = 2 * m.pi; return table.freeze(m)")
            .source_name("garden-math.luau")
            .eval_live()?;
        vm.globals()?.set_live(&mut vm, "math", &math)?;

        // Keep numeric IDs stable for existing gardens. Completion reads the same list.
        let kinds = vm.create_table()?;
        for (id, name) in include_str!("sprite-names.txt").lines().enumerate() {
            kinds.set(&mut vm, name, id as i32)?;
        }
        vm.globals()?
            .set_live(&mut vm, "Sprite", &kinds.as_value())?;
        vm.load(include_str!("garden-api.luau"))
            .source_name("garden-api.luau")
            .exec()?;

        // Scripts draw at a Luau vector. The SVG bridge takes its two coordinates.
        vm.add_fn("draw_sprite", crate::draw_sprite)?;
        load_garden_script(&mut vm, "drawing.luau", drawing)
            .map_err(|err| rustia::Error::embed(format!("In drawing.luau: {err}")))?;

        vm.globals()?.set(&mut vm, "previousFlowers", previous)?;
        load_garden_script(&mut vm, "flowerbed.luau", flowerbed)
            .map_err(|err| rustia::Error::embed(format!("In flowerbed.luau: {err}")))?;
        let bed = vm
            .globals()?
            .get(&mut vm, "FlowerBed")?
            .as_table(&vm)?
            .ok_or_else(|| rustia::Error::embed("Define FlowerBed in flowerbed.luau"))?;
        let save_flowers = bed
            .get(&mut vm, "save")?
            .as_function(&vm)?
            .ok_or_else(|| rustia::Error::embed("Define FlowerBed.save() in flowerbed.luau"))?;

        // Run the script once so it can define its functions.
        let result = load_garden_script(&mut vm, entry, source)?;

        // Scripts can define their callbacks as globals or return them in a table.
        let bindings = match result.as_table(&vm)? {
            Some(exports) => exports,
            None => vm.globals()?,
        };

        // Older saved scripts use frame for their per-tick callback.
        let mut callback = bindings.get(&mut vm, "update")?;
        let legacy_frame = callback.is_nil(&vm)?;
        if legacy_frame {
            callback = bindings.get(&mut vm, "frame")?;
        }
        let update = callback.as_function(&vm)?;
        if update.is_none() && (!interactive_project || !callback.is_nil(&vm)?) {
            return Err(rustia::Error::embed(
                "Define function update(time, mouse, radius)",
            ));
        }

        // Setup sees the initial layout. Only a successfully initialized garden can draw frames.
        let setup = bindings.get(&mut vm, "setup")?;
        if !setup.is_nil(&vm)? {
            let setup = setup.as_function(&vm)?.ok_or_else(|| {
                rustia::Error::embed(
                    "Garden setup must be a function, for example function setup(radius)",
                )
            })?;
            match radius {
                Some(radius) => setup.call::<_, ()>(&mut vm, (radius,))?,
                None => setup.call::<_, ()>(&mut vm, ())?,
            };
        }

        let frame_arguments = vm.create_table()?;
        Ok(Self {
            vm,
            frame_arguments,
            update,
            legacy_frame,
            save_flowers,
        })
    }

    pub(crate) fn animated(&self) -> bool {
        self.update.is_some()
    }

    pub fn saved_flowers(&mut self) -> rustia::Result<Vec<f64>> {
        self.save_flowers.call(&mut self.vm, ())
    }

    pub fn tick(
        &mut self,
        time: f64,
        pointer: Option<(f64, f64)>,
        radius: Option<f64>,
    ) -> rustia::Result<()> {
        let pointer = pointer.or_else(|| self.legacy_frame.then_some((-10000.0, -10000.0)));
        if let Some((x, y)) = pointer {
            let mouse = self.vm.create_vector(x as f32, y as f32, 0.0)?;
            self.call_update(time, mouse, radius)
        } else {
            let mouse = self.frame_arguments.get(&mut self.vm, "no_pointer")?;
            self.call_update(time, mouse, radius)
        }
    }

    fn call_update(&mut self, time: f64, mouse: Value, radius: Option<f64>) -> rustia::Result<()> {
        if let Some(update) = &self.update {
            // The released facade accepts live values for mixed scalar and vector arguments.
            self.frame_arguments.set(&mut self.vm, "time", time)?;
            let time = self.frame_arguments.get(&mut self.vm, "time")?;
            let mut args = vec![time, mouse];
            if let Some(radius) = radius {
                self.frame_arguments.set(&mut self.vm, "radius", radius)?;
                args.push(self.frame_arguments.get(&mut self.vm, "radius")?);
            }
            update.call_values(&mut self.vm, &args)?;
        }
        Ok(())
    }
}

fn load_garden_script(vm: &mut Rustia, name: &str, source: &str) -> rustia::Result<rustia::Value> {
    // Each file gets its own local alias. Keep diagnostics relative to the editor text.
    const PREFIX: &str = "local kinds = Sprite; ";
    vm.load(&format!("{PREFIX}{source}"))
        .source_name(name)
        .source_map(rustia::SourceMap::generated_prefix(PREFIX.len()))
        .eval_live()
        .map_err(|error| {
            use rustia::{ErrorKind, SourceError};
            let Some(span) = error.diagnostic().and_then(|d| d.span) else {
                return error;
            };
            let (phase, detail) = match error.kind() {
                ErrorKind::Source(SourceError::Parse(e)) => ("parse", &e.message),
                ErrorKind::Source(SourceError::Lex(e)) => ("lex", &e.message),
                ErrorKind::Compile(e) => ("compile", &e.message),
                _ => return error,
            };
            rustia::Error::embed(format!(
                "{phase} error at {}..{}: {detail}",
                span.start, span.end
            ))
        })
}
