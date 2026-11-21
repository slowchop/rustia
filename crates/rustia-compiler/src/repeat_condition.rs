//! Luau 0.709 Compiler.cpp::validateContinueUntil checks lexical reads in the
//! condition itself, but only surviving upvalues of functions in the condition.
//! Our bytecode keeps branches and captures Luau folds, so its raw capture list
//! alone cannot answer that source-level question.
use super::*;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

// Limit only bytes synthesized for speculative folding. Original literals are borrowed.
// Length and truth remain known when a computed string exceeds the scratch budget.
const FOLD_STRING_BUDGET: usize = 1 << 20;

#[derive(Clone)]
enum Folded<'a> {
    Scalar(Constant),
    Literal(&'a [u8]),
    Bytes(Rc<[u8]>),
    StringLength(usize),
}

impl Folded<'_> {
    fn string_len(&self) -> Option<usize> {
        match self {
            Self::Literal(b) => Some(b.len()),
            Self::Bytes(b) => Some(b.len()),
            Self::StringLength(len) => Some(*len),
            Self::Scalar(_) => None,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Literal(b) => Some(b),
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    fn truth(&self) -> bool {
        match self {
            Self::Scalar(c) => constant_truthy(c),
            _ => true,
        }
    }

    fn materialize(&self) -> Option<Constant> {
        match self {
            Self::Scalar(c) => Some(c.clone()),
            _ => Some(Constant::String(self.bytes()?.to_vec())),
        }
    }
}

pub(super) type CapturedConstants = BTreeMap<usize, BTreeMap<String, Constant>>;

pub(super) fn check(chunk: &Chunk, options: &CompileOptions) -> Result<CapturedConstants> {
    let mut checker = Checker {
        options,
        collecting_writes: true,
        written: BTreeSet::new(),
        next_binding: 0,
        required: BTreeSet::new(),
        gathering_required: false,
        reads: Vec::new(),
        dependencies: Vec::new(),
        string_budget: Cell::new(FOLD_STRING_BUDGET),
        bindings: Vec::new(),
        loops: Vec::new(),
        conditions: Vec::new(),
        function_depth: 0,
        captured_constants: BTreeMap::new(),
    };
    // A later assignment, even in another closure, prevents constant folding.
    // Both walks visit all syntax and allocate the same lexical binding IDs.
    checker.block(&chunk.statements, true)?;
    // Initializers depend only on bindings that already existed. Follow those
    // dependencies backwards; unrelated local expressions never get materialized.
    for (binding, reads) in checker.dependencies.drain(..).rev() {
        if checker.required.contains(&binding) && !checker.written.contains(&binding) {
            checker.required.extend(&checker.reads[reads]);
        }
    }
    checker.reads.clear();
    checker.collecting_writes = false;
    checker.next_binding = 0;
    checker.block(&chunk.statements, true)?;
    Ok(checker.captured_constants)
}

struct Local<'a> {
    name: &'a str,
    id: usize,
    value: Option<Folded<'a>>,
}

struct Condition {
    skipped: std::ops::Range<usize>,
    continue_span: Span,
    span: Span,
    depth: usize,
}

struct Checker<'a> {
    options: &'a CompileOptions,
    collecting_writes: bool,
    written: BTreeSet<usize>,
    next_binding: usize,
    required: BTreeSet<usize>,
    gathering_required: bool,
    reads: Vec<usize>,
    dependencies: Vec<(usize, Range<usize>)>,
    string_budget: Cell<usize>,
    bindings: Vec<Local<'a>>,
    loops: Vec<Option<Span>>,
    conditions: Vec<Condition>,
    function_depth: usize,
    captured_constants: CapturedConstants,
}

impl<'a> Checker<'a> {
    fn bind(&mut self, name: &'a str, value: Option<Folded<'a>>) {
        let id = self.next_binding;
        self.next_binding += 1;
        let value = if self.written.contains(&id) {
            None
        } else {
            value
        };
        self.bindings.push(Local { name, id, value });
    }

    fn name(&mut self, name: &str, write: bool, reachable: bool) -> Result<()> {
        let Some(local) = self.bindings.iter().rev().find(|var| var.name == name) else {
            return Ok(());
        };
        if self.collecting_writes {
            if write {
                self.written.insert(local.id);
            } else {
                self.reads.push(local.id);
                if self.gathering_required {
                    self.required.insert(local.id);
                }
            }
            return Ok(());
        }
        for condition in self.conditions.iter().rev() {
            if !condition.skipped.contains(&local.id) {
                continue;
            }
            if self.function_depth > condition.depth {
                if !reachable {
                    continue;
                }
                if let Some(value) = &local.value {
                    // Luau embeds the folded value in the closure. Our closure
                    // reads its captured register, even on the continue edge.
                    let constants = self
                        .captured_constants
                        .entry(condition.span.start)
                        .or_default();
                    if !constants.contains_key(name) {
                        let value = value.materialize().ok_or_else(|| CompileError::at(
                            "folded string captured by the repeat..until condition exceeds the compiler's string folding budget", condition.span
                        ))?;
                        constants.insert(name.to_owned(), value);
                    }
                    continue;
                }
            }
            let line = self
                .options
                .source_line_starts
                .partition_point(|s| *s <= condition.continue_span.start)
                .max(1);
            return Err(CompileError::at(format!(
                "Local {name} used in the repeat..until condition is undefined because continue statement on line {line} jumps over it"
            ), condition.span));
        }
        Ok(())
    }

    fn joined(&self, parts: &[Folded<'a>]) -> Option<Folded<'a>> {
        let len = parts
            .iter()
            .try_fold(0usize, |n, p| n.checked_add(p.string_len()?))?;
        if len == 0 {
            return Some(Folded::Literal(b""));
        }
        if let Some(part) = parts.iter().find(|p| p.string_len() == Some(len)) {
            return Some(part.clone());
        }
        if len > self.string_budget.get() || parts.iter().any(|p| p.bytes().is_none()) {
            return Some(Folded::StringLength(len));
        }
        self.string_budget.set(self.string_budget.get() - len);
        let mut bytes = Vec::with_capacity(len);
        for part in parts {
            bytes.extend_from_slice(part.bytes()?);
        }
        Some(Folded::Bytes(bytes.into()))
    }

    fn constant(&self, expr: &'a Expr) -> Option<Folded<'a>> {
        if self.collecting_writes {
            return None;
        }
        match expr {
            Expr::Name(name, _) => self
                .bindings
                .iter()
                .rev()
                .find(|v| v.name == name)?
                .value
                .clone(),
            Expr::String(value, _) => Some(Folded::Literal(value)),
            Expr::Unary { op, expr, .. } => {
                let value = self.constant(expr)?;
                Some(Folded::Scalar(match op {
                    UnaryOp::Not => Constant::Boolean(!value.truth()),
                    UnaryOp::Neg => {
                        let Folded::Scalar(Constant::Number(v)) = value else {
                            return None;
                        };
                        Constant::Number(binary(NumericOp::Neg, v, 0.))
                    }
                    UnaryOp::Len => Constant::Number(value.string_len()? as f64),
                }))
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                let left = self.constant(lhs)?;
                match op {
                    BinaryOp::And => {
                        return if left.truth() {
                            self.constant(rhs)
                        } else {
                            Some(left)
                        }
                    }
                    BinaryOp::Or => {
                        return if left.truth() {
                            Some(left)
                        } else {
                            self.constant(rhs)
                        }
                    }
                    _ => {}
                }
                let right = self.constant(rhs)?;
                if *op == BinaryOp::Concat {
                    return self.joined(&[left, right]);
                }
                if left.string_len().is_some() || right.string_len().is_some() {
                    // O1 folds string equality, but ordered comparisons fold only numbers.
                    if !matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                        return None;
                    }
                    let equal = left.string_len() == right.string_len()
                        && left.bytes()? == right.bytes()?;
                    return Some(Folded::Scalar(Constant::Boolean(
                        equal == (*op == BinaryOp::Eq),
                    )));
                }
                let (Folded::Scalar(a), Folded::Scalar(b)) = (left, right) else {
                    return None;
                };
                fold_binary_constants(*op, a, b).map(Folded::Scalar)
            }
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => self.constant(if self.truth(condition)? {
                then_expr
            } else {
                else_expr
            }),
            Expr::Paren { expr, .. } | Expr::TypeInstantiation { expr, .. } => self.constant(expr),
            Expr::InterpolatedString {
                parts, expressions, ..
            } => {
                let parts = parts
                    .iter()
                    .map(|p| match p {
                        InterpolatedStringPart::Text(t) => Some(Folded::Literal(t)),
                        InterpolatedStringPart::Expr(i) => self.constant(expressions.get(*i)?),
                    })
                    .collect::<Option<Vec<_>>>()?;
                self.joined(&parts)
            }
            _ => const_eval(expr).map(Folded::Scalar),
        }
    }

    fn truth(&self, expr: &'a Expr) -> Option<bool> {
        Some(self.constant(expr)?.truth())
    }

    fn branch_truth(&self, expr: &'a Expr) -> Option<bool> {
        if self.truth(expr) == Some(false)
            || matches!(expr, Expr::Binary { op: BinaryOp::And, rhs, .. } if self.truth(rhs) == Some(false))
        {
            // compileStatIf omits this then-block altogether.
            Some(false)
        } else if !self.condition_jumps(expr, false) {
            // Luau emits the else-block only if a conditional jump needs it.
            Some(true)
        } else {
            None
        }
    }

    fn condition_jumps(&self, expr: &'a Expr, truth: bool) -> bool {
        if let Some(value) = self.truth(expr) {
            return value == truth;
        }
        match expr {
            Expr::Paren { expr, .. } => self.condition_jumps(expr, truth),
            Expr::Unary {
                op: UnaryOp::Not,
                expr,
                ..
            } => self.condition_jumps(expr, !truth),
            Expr::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                lhs,
                rhs,
                ..
            } => {
                // Under compileConditionValue, a short circuit to fallthrough
                // contributes no jump to the caller's list.
                (truth != (*op == BinaryOp::And) && self.condition_jumps(lhs, truth))
                    || self.condition_jumps(rhs, truth)
            }
            _ => true,
        }
    }

    fn block(&mut self, stmts: &'a [Stmt], reachable: bool) -> Result<()> {
        let outer = self.bindings.len();
        for stmt in stmts {
            self.stmt(stmt, reachable)?;
        }
        self.bindings.truncate(outer);
        Ok(())
    }

    fn function(
        &mut self,
        params: &'a [FunctionParam],
        body: &'a [Stmt],
        method: bool,
        reachable: bool,
    ) -> Result<()> {
        let outer = self.bindings.len();
        // Upstream compiles each function independently. A function in a dead
        // branch still checks its own repeats but contributes no parent capture.
        let saved = (!reachable).then(|| std::mem::take(&mut self.conditions));
        self.function_depth += 1;
        if method {
            self.bind("self", None);
        }
        for param in params {
            self.bind(&param.name, None);
        }
        self.block(body, true)?;
        self.function_depth -= 1;
        if let Some(saved) = saved {
            self.conditions = saved;
        }
        self.bindings.truncate(outer);
        Ok(())
    }

    fn target(&mut self, target: &'a AssignmentTarget, reachable: bool) -> Result<()> {
        match target {
            AssignmentTarget::Local { name, .. } => self.name(name, true, reachable),
            AssignmentTarget::Field { object, .. } => self.expr(object, reachable),
            AssignmentTarget::Index { object, index, .. } => {
                self.expr(object, reachable)?;
                self.expr(index, reachable)
            }
        }
    }

    fn repeat(&mut self, body: &'a [Stmt], condition: &'a Expr, reachable: bool) -> Result<()> {
        let outer = self.bindings.len();
        self.loops.push(None);
        let mut first = None;
        for stmt in body {
            self.stmt(stmt, reachable)?;
            if let (None, Some(span)) = (first, self.loops.last().copied().flatten()) {
                first = Some((span, self.next_binding));
            }
        }
        self.loops.pop();
        if let Some((continue_span, start)) = first {
            self.conditions.push(Condition {
                skipped: start..self.next_binding,
                continue_span,
                span: condition.span(),
                depth: self.function_depth,
            });
            self.condition_expr(condition, reachable)?;
            self.conditions.pop();
        } else {
            self.condition_expr(condition, reachable)?;
        }
        self.bindings.truncate(outer);
        Ok(())
    }

    fn stmt(&mut self, stmt: &'a Stmt, reachable: bool) -> Result<()> {
        match stmt {
            Stmt::Local { bindings, init, .. } => {
                let mut ranges = Vec::new();
                for expr in init {
                    let start = self.reads.len();
                    self.expr(expr, reachable)?;
                    if self.collecting_writes {
                        ranges.push(start..self.reads.len());
                    }
                }
                // All RHS expressions see the scope before any new binding.
                let multret = matches!(
                    init.last(),
                    Some(Expr::Call { .. } | Expr::MethodCall { .. } | Expr::Varargs(_))
                );
                let values: Vec<_> = (0..bindings.len())
                    .map(|i| {
                        let id = self.next_binding + i;
                        if self.collecting_writes
                            || !self.required.contains(&id)
                            || self.written.contains(&id)
                        {
                            return None;
                        }
                        match init.get(i) {
                            Some(expr) => self.constant(expr),
                            None if !multret => Some(Folded::Scalar(Constant::Nil)),
                            None => None,
                        }
                    })
                    .collect();
                for (i, (binding, value)) in bindings.iter().zip(values).enumerate() {
                    if let Some(reads) = ranges.get(i) {
                        self.dependencies.push((self.next_binding, reads.clone()));
                    }
                    self.bind(&binding.name, value);
                }
            }
            Stmt::LocalFunction {
                name, params, body, ..
            } => {
                self.bind(name, None);
                self.function(params, body, false, reachable)?;
            }
            Stmt::Function {
                target,
                params,
                body,
                is_method,
                ..
            } => {
                self.target(target, reachable)?;
                self.function(params, body, *is_method, reachable)?;
            }
            Stmt::Assignment {
                targets, values, ..
            } => {
                for target in targets {
                    self.target(target, reachable)?;
                }
                self.exprs(values, reachable)?;
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.condition_expr(condition, reachable)?;
                let truth = self.branch_truth(condition);
                self.block(then_body, reachable && truth != Some(false))?;
                self.block(else_body, reachable && truth != Some(true))?;
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.condition_expr(condition, reachable)?;
                self.loops.push(None);
                self.block(body, reachable && self.truth(condition) != Some(false))?;
                self.loops.pop();
            }
            Stmt::ForNumeric {
                variable,
                start,
                limit,
                step,
                body,
                ..
            } => {
                self.expr(start, reachable)?;
                self.expr(limit, reachable)?;
                if let Some(step) = step {
                    self.expr(step, reachable)?;
                }
                let outer = self.bindings.len();
                self.bind(variable, None);
                self.loops.push(None);
                self.block(body, reachable)?;
                self.loops.pop();
                self.bindings.truncate(outer);
            }
            Stmt::ForGeneric {
                variables,
                iter,
                body,
                ..
            } => {
                self.exprs(iter, reachable)?;
                let outer = self.bindings.len();
                for var in variables {
                    self.bind(&var.name, None);
                }
                self.loops.push(None);
                self.block(body, reachable)?;
                self.loops.pop();
                self.bindings.truncate(outer);
            }
            Stmt::Repeat {
                body, condition, ..
            } => self.repeat(body, condition, reachable)?,
            Stmt::Do { body, .. } => self.block(body, reachable)?,
            Stmt::Continue { span } if reachable && !self.collecting_writes => {
                if let Some(slot) = self.loops.last_mut() {
                    slot.get_or_insert(*span);
                }
            }
            Stmt::Return {
                value,
                extra_values,
                ..
            } => {
                if let Some(value) = value {
                    self.expr(value, reachable)?;
                }
                self.exprs(extra_values, reachable)?;
            }
            Stmt::Expr { expr, .. } => self.expr(expr, reachable)?,
            Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::TypeAlias { .. }
            | Stmt::TypeFunction { .. } => {}
        }
        Ok(())
    }

    fn condition_expr(&mut self, expr: &'a Expr, reachable: bool) -> Result<()> {
        let previous = self.gathering_required;
        self.gathering_required = true;
        self.expr(expr, reachable)?;
        self.gathering_required = previous;
        Ok(())
    }

    fn exprs(&mut self, exprs: &'a [Expr], reachable: bool) -> Result<()> {
        for expr in exprs {
            self.expr(expr, reachable)?;
        }
        Ok(())
    }

    fn expr(&mut self, expr: &'a Expr, reachable: bool) -> Result<()> {
        // The condition's own AST is checked even for an unexecuted operand.
        // A nested function, however, contributes only its compiled captures.
        let lexical = self
            .conditions
            .last()
            .is_some_and(|c| c.depth == self.function_depth);
        match expr {
            Expr::Name(name, _) => self.name(name, false, reachable)?,
            Expr::Unary { expr, .. }
            | Expr::Paren { expr, .. }
            | Expr::TypeInstantiation { expr, .. }
            | Expr::Field { object: expr, .. } => self.expr(expr, reachable)?,
            Expr::Binary { op, lhs, rhs, .. } => {
                self.expr(lhs, reachable)?;
                let skip = !lexical
                    && !self.conditions.is_empty()
                    && match op {
                        BinaryOp::And => self.truth(lhs) == Some(false),
                        BinaryOp::Or => self.truth(lhs) == Some(true),
                        _ => false,
                    };
                self.expr(rhs, reachable && !skip)?;
            }
            Expr::IfElse {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.condition_expr(condition, reachable)?;
                let truth = if lexical || self.conditions.is_empty() {
                    None
                } else {
                    self.truth(condition)
                };
                self.expr(then_expr, reachable && truth != Some(false))?;
                self.expr(else_expr, reachable && truth != Some(true))?;
            }
            Expr::Index { object, index, .. } => {
                self.expr(object, reachable)?;
                self.expr(index, reachable)?;
            }
            Expr::Table { fields, .. } => {
                for field in fields {
                    match field {
                        TableField::Keyed { key, value, .. } => {
                            self.expr(key, reachable)?;
                            self.expr(value, reachable)?;
                        }
                        TableField::Named { value, .. } | TableField::Array { value, .. } => {
                            self.expr(value, reachable)?
                        }
                    }
                }
            }
            Expr::Function { params, body, .. } => self.function(params, body, false, reachable)?,
            Expr::Call { callee, args, .. }
            | Expr::MethodCall {
                object: callee,
                args,
                ..
            } => {
                self.expr(callee, reachable)?;
                self.exprs(args, reachable)?;
            }
            Expr::InterpolatedString { expressions, .. } => self.exprs(expressions, reachable)?,
            Expr::Nil(_)
            | Expr::Bool(..)
            | Expr::Number(..)
            | Expr::String(..)
            | Expr::Varargs(_) => {}
        }
        Ok(())
    }
}
