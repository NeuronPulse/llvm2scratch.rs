use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;

use crate::scratch::{Block, BlockList, Project, Value};
use crate::scratch::ast::{BoolOp, ControlOp, CostumeInfoOp, KnownVal, ListOp, Op, PenOp, VarOp};
use crate::target::{Target, TargetPerf};

use super::BlockListInfo;

const LOOP_USE_MULTIPLIER: f64 = 5000.0;

pub fn get_value_var_use(value: &Value) -> HashSet<String> {
    let mut reads = HashSet::new();
    collect_value_var_use(value, &mut reads);
    reads
}

fn collect_value_var_use(value: &Value, reads: &mut HashSet<String>) {
    match value {
        Value::GetVar { name } => {
            reads.insert(name.clone());
        }
        Value::Op(op) => {
            collect_value_var_use(op.left(), reads);
            collect_value_var_use(op.right(), reads);
        }
        Value::BoolOp(bop) => {
            collect_value_var_use(bop.left(), reads);
            collect_value_var_use(bop.right(), reads);
        }
        Value::GetOfList(gol) => {
            collect_value_var_use(&gol.value, reads);
        }
        Value::Known(_) | Value::KnownBool(_) | Value::GetParam { .. } |
        Value::GetList { .. } | Value::GetListLength { .. } |
        Value::CostumeInfo { .. } | Value::GetCounter | Value::GetAnswer | Value::DaysSince2000 => {}
    }
}

/// Like `get_value_var_use` but returns a count per variable (matching Python's
/// `getValueVarUse` Counter semantics). A variable used N times in the same
/// expression gets count N, not 1.
fn get_value_var_use_counts(value: &Value) -> HashMap<String, f64> {
    let mut counts: HashMap<String, f64> = HashMap::new();
    collect_value_var_use_counts(value, &mut counts);
    counts
}

fn collect_value_var_use_counts(value: &Value, counts: &mut HashMap<String, f64>) {
    match value {
        Value::GetVar { name } => {
            *counts.entry(name.clone()).or_insert(0.0) += 1.0;
        }
        Value::Op(op) => {
            collect_value_var_use_counts(op.left(), counts);
            collect_value_var_use_counts(op.right(), counts);
        }
        Value::BoolOp(bop) => {
            collect_value_var_use_counts(bop.left(), counts);
            collect_value_var_use_counts(bop.right(), counts);
        }
        Value::GetOfList(gol) => {
            collect_value_var_use_counts(&gol.value, counts);
        }
        Value::Known(_) | Value::KnownBool(_) | Value::GetParam { .. } |
        Value::GetList { .. } | Value::GetListLength { .. } |
        Value::CostumeInfo { .. } | Value::GetCounter | Value::GetAnswer | Value::DaysSince2000 => {}
    }
}

fn repeat_count_multiplier(condition: &Option<Value>, op: ControlOp) -> f64 {
    match op {
        ControlOp::RepTimes => {
            if let Some(Value::Known(KnownVal::Num(n))) = condition {
                *n
            } else {
                LOOP_USE_MULTIPLIER
            }
        }
        ControlOp::ForEach => {
            if let Some(Value::Known(KnownVal::Num(n))) = condition {
                *n
            } else {
                LOOP_USE_MULTIPLIER
            }
        }
        ControlOp::Until
        | ControlOp::While
        | ControlOp::Forever => LOOP_USE_MULTIPLIER,
        _ => 1.0,
    }
}

pub fn get_blocklist_var_use(
    blocklist: &BlockList,
    func_info: Option<&HashMap<String, BlockListInfo>>,
) -> HashMap<String, f64> {
    let mut times_used: HashMap<String, f64> = HashMap::new();

    fn add_reads(times: &mut HashMap<String, f64>, reads: HashMap<String, f64>, multiplier: f64) {
        for (var, count) in reads {
            *times.entry(var).or_insert(0.0) += count * multiplier;
        }
    }

    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                add_reads(&mut times_used, get_value_var_use_counts(&data.value), 1.0);
                if data.op != VarOp::Set {
                    *times_used.entry(data.name.clone()).or_insert(0.0) += 1.0;
                }
            }
            Block::Say { value }
            | Block::SwitchCostume { value }
            | Block::EditVolume { value, .. }
            | Block::Broadcast { value, .. }
            | Block::Wait { value } => {
                add_reads(&mut times_used, get_value_var_use_counts(value), 1.0);
            }
            Block::Ask { value, .. } => {
                add_reads(&mut times_used, get_value_var_use_counts(value), 1.0);
            }
            Block::EditList(data) => {
                if let Some(idx) = &data.index {
                    add_reads(&mut times_used, get_value_var_use_counts(idx), 1.0);
                }
                if let Some(val) = &data.value {
                    add_reads(&mut times_used, get_value_var_use_counts(val), 1.0);
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    let cond_mult = match cf.op {
                        ControlOp::Until | ControlOp::While | ControlOp::Forever => LOOP_USE_MULTIPLIER,
                        _ => 1.0,
                    };
                    add_reads(&mut times_used, get_value_var_use_counts(cond), cond_mult);
                }
                if cf.op == ControlOp::ForEach {
                    if let Some(var) = &cf.var {
                        *times_used.entry(var.clone()).or_insert(0.0) += 1.0;
                    }
                }
                let multiplier = repeat_count_multiplier(&cf.condition, cf.op);
                if let Some(body) = &cf.body {
                    let body_use = get_blocklist_var_use(body, func_info);
                    for (var, count) in body_use {
                        let add = if cf.op == ControlOp::IfElse {
                            count * 0.5
                        } else {
                            count * multiplier
                        };
                        *times_used.entry(var).or_insert(0.0) += add;
                    }
                }
                if let Some(else_body) = &cf.else_body {
                    let else_use = get_blocklist_var_use(else_body, func_info);
                    for (var, count) in else_use {
                        let add = if cf.op == ControlOp::IfElse {
                            count * 0.5
                        } else {
                            count
                        };
                        *times_used.entry(var).or_insert(0.0) += add;
                    }
                }
            }
            Block::ProcedureDef(_) => {}
            Block::ProcedureCall(data) => {
                for arg in &data.args {
                    add_reads(&mut times_used, get_value_var_use_counts(arg), 1.0);
                }
                if let Some(info) = func_info
                    && let Some(bli) = info.get(&data.name) {
                        for (var, count) in &bli.times_used {
                            *times_used.entry(var.clone()).or_insert(0.0) += count;
                        }
                    }
            }
            Block::Pen(pen_op) => {
                match pen_op {
                    PenOp::SetColor { color } => {
                        add_reads(&mut times_used, get_value_var_use_counts(color), 1.0);
                    }
                    PenOp::SetSize { size } => {
                        add_reads(&mut times_used, get_value_var_use_counts(size), 1.0);
                    }
                    _ => {}
                }
            }
            Block::MotionGoto { x, y } => {
                add_reads(&mut times_used, get_value_var_use_counts(x), 1.0);
                add_reads(&mut times_used, get_value_var_use_counts(y), 1.0);
            }
            _ => {}
        }
    }

    times_used
}

pub fn should_elide(value: &Value, times_used: f64, perf: &TargetPerf) -> bool {
    if times_used <= 1.0 {
        return true;
    }

    let cost = get_value_cost(value, perf);
    cost * times_used < perf.set_var + cost + times_used * perf.get_var
}

fn collect_value_dependencies(value: &Value, deps: &mut HashSet<String>) {
    match value {
        Value::GetVar { name } => { deps.insert(format!("var:{}", name)); }
        Value::GetList { name } | Value::GetListLength { name } => {
            deps.insert(format!("list:{}", name));
        }
        Value::GetOfList(gol) => {
            deps.insert(format!("list:{}", gol.name));
            collect_value_dependencies(&gol.value, deps);
        }
        Value::Op(op) => {
            collect_value_dependencies(op.left(), deps);
            collect_value_dependencies(op.right(), deps);
        }
        Value::BoolOp(bop) => {
            collect_value_dependencies(bop.left(), deps);
            collect_value_dependencies(bop.right(), deps);
        }
        Value::GetCounter => { deps.insert("counter:".to_string()); }
        Value::GetAnswer => { deps.insert("answer:".to_string()); }
        Value::CostumeInfo { .. } => { deps.insert("costume:".to_string()); }
        _ => {}
    }
}

fn collect_block_modifications(block: &Block, mods: &mut HashSet<String>) {
    match block {
        Block::EditVar(data) => {
            mods.insert(format!("var:{}", data.name));
        }
        Block::EditList(data) => {
            mods.insert(format!("list:{}", data.name));
        }
        Block::EditCounter(_) => {
            mods.insert("counter:".to_string());
        }
        Block::Ask { var_name, .. } => {
            if let Some(name) = var_name {
                mods.insert(format!("var:{}", name));
            }
            mods.insert("answer:".to_string());
        }
        Block::SwitchCostume { .. } => {
            mods.insert("costume:".to_string());
        }
        Block::ControlFlow(cf) => {
            if let Some(body) = &cf.body {
                collect_blocklist_modifications(body, mods);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_modifications(else_body, mods);
            }
        }
        _ => {}
    }
}

fn collect_blocklist_modifications(blocklist: &BlockList, mods: &mut HashSet<String>) {
    for block in &blocklist.blocks {
        collect_block_modifications(block, mods);
    }
}

/// Collect variable names (without the "var:" prefix) that a blocklist depends on or modifies.
/// This mirrors the information Python uses to populate `cannot_elide` from other functions.
///
/// Input reads are filtered by `mods` (matching Python's
/// `info.dependent |= all_value_dependent - info.always_modify`): a variable
/// that has already been (always) modified before a read is NOT counted as a
/// dependency. This is critical for correctness: e.g. if `fib:6` does
/// `set %fib:7 ...; fib(%fib:7)`, the read of `%fib:7` in the `fib()` call is
/// filtered because `%fib:7` was set earlier, so `fn_deps["func:fib:6"]` does
/// NOT include `%fib:7`. Without this filtering, `fn_deps["func:fib"]` would
/// transitively include `%fib:7`, causing the elision loop to think `fib()`
/// reads `%fib:7` in its body, preventing elision of `%fib:7`.
fn collect_blocklist_var_names(
    blocklist: &BlockList,
    deps: &mut HashSet<String>,
    mods: &mut HashSet<String>,
) {
    fn add_var_deps(value: &Value, deps: &mut HashSet<String>, mods: &HashSet<String>) {
        let mut d = HashSet::new();
        collect_value_dependencies(value, &mut d);
        for dep in d {
            if let Some(stripped) = dep.strip_prefix("var:") {
                if !mods.contains(stripped) {
                    deps.insert(stripped.to_string());
                }
            }
        }
    }

    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                add_var_deps(&data.value, deps, mods);
                if data.op != VarOp::Set && !mods.contains(&data.name) {
                    deps.insert(data.name.clone());
                }
                mods.insert(data.name.clone());
            }
            Block::Say { value }
            | Block::SwitchCostume { value }
            | Block::EditVolume { value, .. }
            | Block::Broadcast { value, .. }
            | Block::Wait { value } => {
                add_var_deps(value, deps, mods);
            }
            Block::Ask { value, var_name } => {
                add_var_deps(value, deps, mods);
                if let Some(name) = var_name {
                    mods.insert(name.clone());
                    deps.insert(name.clone());
                }
            }
            Block::EditList(data) => {
                if let Some(idx) = &data.index {
                    add_var_deps(idx, deps, mods);
                }
                if let Some(val) = &data.value {
                    add_var_deps(val, deps, mods);
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    add_var_deps(cond, deps, mods);
                }
                if let Some(body) = &cf.body {
                    collect_blocklist_var_names(body, deps, mods);
                }
                if let Some(else_body) = &cf.else_body {
                    collect_blocklist_var_names(else_body, deps, mods);
                }
            }
            Block::ProcedureCall(data) => {
                for arg in &data.args {
                    add_var_deps(arg, deps, mods);
                }
            }
            Block::Pen(pen_op) => {
                match pen_op {
                    PenOp::SetColor { color } => {
                        add_var_deps(color, deps, mods);
                    }
                    PenOp::SetSize { size } => {
                        add_var_deps(size, deps, mods);
                    }
                    _ => {}
                }
            }
            Block::MotionGoto { x, y } => {
                add_var_deps(x, deps, mods);
                add_var_deps(y, deps, mods);
            }
            _ => {}
        }
    }
}

/// Collect variable names (without "var:" prefix) that a block reads.
/// This mirrors Python's `use.dependent` for a single block, used by the
/// `changed_but_unread` logic to detect whether a variable is read after its
/// dependencies have been modified.
fn block_variable_reads(block: &Block) -> HashSet<String> {
    let mut reads = HashSet::new();
    collect_block_reads(block, &mut reads);
    reads
}

fn collect_block_reads(block: &Block, reads: &mut HashSet<String>) {
    match block {
        Block::EditVar(data) => {
            collect_value_var_use(&data.value, reads);
            // For "change" op, the variable itself is also read (old value).
            if data.op != VarOp::Set {
                reads.insert(data.name.clone());
            }
        }
        Block::Say { value }
        | Block::SwitchCostume { value }
        | Block::EditVolume { value, .. }
        | Block::Broadcast { value, .. }
        | Block::Wait { value } => {
            collect_value_var_use(value, reads);
        }
        Block::Ask { value, .. } => {
            collect_value_var_use(value, reads);
        }
        Block::EditList(data) => {
            if let Some(idx) = &data.index {
                collect_value_var_use(idx, reads);
            }
            if let Some(val) = &data.value {
                collect_value_var_use(val, reads);
            }
        }
        Block::ControlFlow(cf) => {
            if let Some(cond) = &cf.condition {
                collect_value_var_use(cond, reads);
            }
            // ForEach var is written per-iteration, not read at the control level.
            if let Some(body) = &cf.body {
                collect_blocklist_reads(body, reads);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_reads(else_body, reads);
            }
        }
        Block::ProcedureCall(data) => {
            for arg in &data.args {
                collect_value_var_use(arg, reads);
            }
            // Note: callee body reads are not included here. Variables read by
            // callees are already in `cannot_elide` (via other functions' info),
            // preventing them from entering `to_elide` in the first place.
        }
        Block::Pen(pen_op) => {
            match pen_op {
                PenOp::SetColor { color } => {
                    collect_value_var_use(color, reads);
                }
                PenOp::SetSize { size } => {
                    collect_value_var_use(size, reads);
                }
                _ => {}
            }
        }
        Block::MotionGoto { x, y } => {
            collect_value_var_use(x, reads);
            collect_value_var_use(y, reads);
        }
        _ => {}
    }
}

fn collect_blocklist_reads(blocklist: &BlockList, reads: &mut HashSet<String>) {
    for block in &blocklist.blocks {
        collect_block_reads(block, reads);
    }
}

/// Combined variable reads: includes both direct input reads (condition for
/// ControlFlow, arguments for ProcedureCall, message for Broadcast) and body
/// reads (body blocks / callee transitive deps). Used for the
/// `changed_but_unread` removal check — a variable read anywhere in the block
/// (inputs or body) after its dependency was modified cannot be elided.
///
/// `is_ending` propagates to ControlFlow bodies and controls callee dep
/// inclusion for ending ProcedureCall/Broadcast (same semantics as
/// `block_modifications_with_callees`).
fn block_variable_reads_combined(
    block: &Block,
    callee_deps: &HashMap<String, HashSet<String>>,
    is_ending: bool,
) -> HashSet<String> {
    let mut reads = block_variable_reads(block);
    // For ProcedureCall/Broadcast, also include the callee's transitive
    // variable reads (body reads) — but only if not an ending call.
    // For ControlFlow, the body reads are already included by
    // `block_variable_reads` (via `collect_blocklist_reads`), but that
    // function doesn't include callee deps for nested ProcedureCalls.
    // We need to add callee deps for non-ending nested calls too.
    match block {
        Block::ProcedureCall(data) => {
            if !is_ending {
                let key = format!("func:{}", data.name);
                if let Some(deps) = callee_deps.get(&key) {
                    for dep in deps {
                        reads.insert(dep.clone());
                    }
                }
            }
        }
        Block::Broadcast { value, .. } => {
            if !is_ending {
                if let Value::Known(KnownVal::Str(s)) = value {
                    let key = format!("broadcast:{}", s);
                    if let Some(deps) = callee_deps.get(&key) {
                        for dep in deps {
                            reads.insert(dep.clone());
                        }
                    }
                }
            }
        }
        Block::ControlFlow(cf) => {
            // Add callee deps from nested non-ending ProcedureCalls in the body.
            let body_is_ending = match cf.op {
                ControlOp::If | ControlOp::IfElse => is_ending,
                _ => false,
            };
            if let Some(body) = &cf.body {
                collect_blocklist_reads_combined(body, &mut reads, callee_deps, body_is_ending);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_reads_combined(else_body, &mut reads, callee_deps, body_is_ending);
            }
        }
        _ => {}
    }
    reads
}

/// Helper: collect callee deps from a blocklist, respecting is_ending.
fn collect_blocklist_reads_combined(
    blocklist: &BlockList,
    reads: &mut HashSet<String>,
    callee_deps: &HashMap<String, HashSet<String>>,
    is_ending_blocklist: bool,
) {
    for (i, block) in blocklist.blocks.iter().enumerate() {
        let is_end_of_blocklist = i == blocklist.blocks.len() - 1;
        let is_ending = (is_end_of_blocklist && is_ending_blocklist)
            || matches!(blocklist.blocks.get(i + 1), Some(Block::StopScript(crate::scratch::ast::StopOption::This)));
        match block {
            Block::ProcedureCall(data) => {
                if !is_ending {
                    let key = format!("func:{}", data.name);
                    if let Some(deps) = callee_deps.get(&key) {
                        for dep in deps {
                            reads.insert(dep.clone());
                        }
                    }
                }
            }
            Block::Broadcast { value, .. } => {
                if !is_ending {
                    if let Value::Known(KnownVal::Str(s)) = value {
                        let key = format!("broadcast:{}", s);
                        if let Some(deps) = callee_deps.get(&key) {
                            for dep in deps {
                                reads.insert(dep.clone());
                            }
                        }
                    }
                }
            }
            Block::ControlFlow(cf) => {
                let body_is_ending = match cf.op {
                    ControlOp::If | ControlOp::IfElse => is_ending,
                    _ => false,
                };
                if let Some(body) = &cf.body {
                    collect_blocklist_reads_combined(body, reads, callee_deps, body_is_ending);
                }
                if let Some(else_body) = &cf.else_body {
                    collect_blocklist_reads_combined(else_body, reads, callee_deps, body_is_ending);
                }
            }
            _ => {}
        }
    }
}

/// Body-only variable reads: excludes direct input reads (condition for
/// ControlFlow, arguments for ProcedureCall, message for Broadcast). This
/// mirrors Python's two-phase logic where pass 2 uses `ignore_inputs=True`,
/// so only body/callee reads count as "read after modification". Used for the
/// `changed_but_unread` add check.
///
/// `is_ending` controls callee dep inclusion for ending ProcedureCall/
/// Broadcast (callee deps excluded when ending), and propagates to
/// ControlFlow bodies as `is_ending_blocklist` (if/if_else propagate,
/// loops don't) — matching Python's `getBlockListVarUse` semantics.
fn block_variable_reads_body(
    block: &Block,
    callee_deps: &HashMap<String, HashSet<String>>,
    is_ending: bool,
) -> HashSet<String> {
    let mut reads = HashSet::new();
    match block {
        Block::ControlFlow(cf) => {
            // Body-only: exclude condition reads. Collect direct reads from
            // body blocks (via collect_blocklist_reads) AND callee deps from
            // nested non-ending ProcedureCalls (via collect_blocklist_reads_combined).
            let body_is_ending = match cf.op {
                ControlOp::If | ControlOp::IfElse => is_ending,
                _ => false,
            };
            if let Some(body) = &cf.body {
                collect_blocklist_reads(body, &mut reads);
                collect_blocklist_reads_combined(body, &mut reads, callee_deps, body_is_ending);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_reads(else_body, &mut reads);
                collect_blocklist_reads_combined(else_body, &mut reads, callee_deps, body_is_ending);
            }
        }
        Block::ProcedureCall(data) => {
            // Callee body reads only: exclude argument reads.
            if !is_ending {
                let key = format!("func:{}", data.name);
                if let Some(deps) = callee_deps.get(&key) {
                    for dep in deps {
                        reads.insert(dep.clone());
                    }
                }
            }
        }
        Block::Broadcast { value, .. } => {
            // Handler body reads only: exclude message reads.
            if !is_ending {
                if let Value::Known(KnownVal::Str(s)) = value {
                    let key = format!("broadcast:{}", s);
                    if let Some(deps) = callee_deps.get(&key) {
                        for dep in deps {
                            reads.insert(dep.clone());
                        }
                    }
                }
            }
        }
        _ => {
            // For other blocks, body reads = all reads (inputs are evaluated
            // atomically before any modification).
            collect_block_reads(block, &mut reads);
        }
    }
    reads
}

/// Return the function-key for the first block of a blocklist (mirrors Python's
/// `"func:" + proc_name` / `"broadcast:" + name` / `"start:"` convention).
fn fn_key_of(block: &Block) -> String {
    match block {
        Block::ProcedureDef(data) => format!("func:{}", data.name),
        Block::OnBroadcast { name } => format!("broadcast:{}", name),
        Block::OnStartFlag => "start:".to_string(),
        _ => String::new(),
    }
}

/// Collect the set of function-keys called by a blocklist (ProcedureCall and
/// Broadcast). Used to compute the transitive closure of modifications.
fn collect_blocklist_callees(blocklist: &BlockList, callees: &mut HashSet<String>) {
    for block in &blocklist.blocks {
        collect_block_callees(block, callees);
    }
}

fn collect_block_callees(block: &Block, callees: &mut HashSet<String>) {
    match block {
        Block::ProcedureCall(data) => {
            callees.insert(format!("func:{}", data.name));
        }
        Block::Broadcast { value, .. } => {
            if let Value::Known(KnownVal::Str(s)) = value {
                callees.insert(format!("broadcast:{}", s));
            }
        }
        Block::ControlFlow(cf) => {
            if let Some(body) = &cf.body {
                collect_blocklist_callees(body, callees);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_callees(else_body, callees);
            }
        }
        _ => {}
    }
}

/// Like `block_modifications`, but also includes modifications performed by
/// callees (ProcedureCall / Broadcast), using the precomputed transitive
/// closure `callee_mods`. This mirrors Python's `getBlockListVarUse` which
/// merges `callee_info.might_modify` for procedure/broadcast calls.
///
/// `is_ending` indicates whether this block is an "ending" block (last in an
/// ending blocklist, or followed by `stop this script`). For ProcedureCall/
/// Broadcast, ending calls exclude callee mods (Python: `if not is_ending`).
/// For ControlFlow, `is_ending` propagates as `is_ending_blocklist` to the body:
///   - if/if_else: body gets the parent's `is_ending`
///   - loops (reptimes/until/while/forever/for_each): body gets `false`
fn block_modifications_with_callees(
    block: &Block,
    callee_mods: &HashMap<String, HashSet<String>>,
    ignore_prefixed: &HashSet<String>,
    is_ending: bool,
) -> HashSet<String> {
    let mut mods = HashSet::new();
    collect_block_modifications_with_callees(block, &mut mods, callee_mods, ignore_prefixed, is_ending);
    mods
}

fn collect_block_modifications_with_callees(
    block: &Block,
    mods: &mut HashSet<String>,
    callee_mods: &HashMap<String, HashSet<String>>,
    ignore_prefixed: &HashSet<String>,
    is_ending: bool,
) {
    match block {
        Block::EditVar(data) => {
            mods.insert(format!("var:{}", data.name));
        }
        Block::EditList(data) => {
            mods.insert(format!("list:{}", data.name));
        }
        Block::EditCounter(_) => {
            mods.insert("counter:".to_string());
        }
        Block::Ask { var_name, .. } => {
            if let Some(name) = var_name {
                mods.insert(format!("var:{}", name));
            }
            mods.insert("answer:".to_string());
        }
        Block::SwitchCostume { .. } => {
            mods.insert("costume:".to_string());
        }
        Block::ControlFlow(cf) => {
            // Propagate is_ending to body: if/if_else propagate, loops don't.
            let body_is_ending = match cf.op {
                ControlOp::If | ControlOp::IfElse => is_ending,
                _ => false,
            };
            if let Some(body) = &cf.body {
                collect_blocklist_mods_with_callees(body, mods, callee_mods, ignore_prefixed, body_is_ending);
            }
            if let Some(else_body) = &cf.else_body {
                collect_blocklist_mods_with_callees(else_body, mods, callee_mods, ignore_prefixed, body_is_ending);
            }
        }
        Block::ProcedureCall(data) => {
            if !is_ending {
                let key = format!("func:{}", data.name);
                if let Some(c_mods) = callee_mods.get(&key) {
                    for m in c_mods {
                        if !ignore_prefixed.contains(m) {
                            mods.insert(m.clone());
                        }
                    }
                }
            }
        }
        Block::Broadcast { value, .. } => {
            if !is_ending {
                if let Value::Known(KnownVal::Str(s)) = value {
                    let key = format!("broadcast:{}", s);
                    if let Some(c_mods) = callee_mods.get(&key) {
                        for m in c_mods {
                            if !ignore_prefixed.contains(m) {
                                mods.insert(m.clone());
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Process a blocklist (e.g. a ControlFlow body) for modifications, computing
/// `is_ending` for each block based on position and `is_ending_blocklist`.
fn collect_blocklist_mods_with_callees(
    blocklist: &BlockList,
    mods: &mut HashSet<String>,
    callee_mods: &HashMap<String, HashSet<String>>,
    ignore_prefixed: &HashSet<String>,
    is_ending_blocklist: bool,
) {
    for (i, block) in blocklist.blocks.iter().enumerate() {
        let is_end_of_blocklist = i == blocklist.blocks.len() - 1;
        let is_ending = (is_end_of_blocklist && is_ending_blocklist)
            || matches!(blocklist.blocks.get(i + 1), Some(Block::StopScript(crate::scratch::ast::StopOption::This)));
        collect_block_modifications_with_callees(block, mods, callee_mods, ignore_prefixed, is_ending);
    }
}

pub fn get_value_cost(value: &Value, perf: &TargetPerf) -> f64 {
    let cost = match value {
        Value::Known(_) | Value::KnownBool(_) => 0.0,
        Value::GetVar { .. } => perf.get_var,
        Value::GetParam { .. } => perf.param,
        Value::Op(op) => {
            let op_cost = match op {
                Op::Add(_, _) => perf.add,
                Op::Sub(_, _) => perf.sub,
                Op::Mul(_, _) => perf.mul,
                Op::Div(_, _) => perf.div,
                Op::Mod(_, _) => perf.r#mod,
                Op::Rand(_, _) => perf.rand,
                Op::Join(_, _) => perf.join,
                Op::LetterOf(_, _) => perf.letter_of,
                Op::LengthOf(_) => perf.length_of_str,
                Op::Round(_) => perf.round,
                Op::Not(_) => perf.not_,
                Op::Contains(_, _) => perf.contains_str,
                Op::Abs(_) => perf.abs,
                Op::Floor(_) => perf.floor,
                Op::Ceiling(_) => perf.ceil,
                Op::Sqrt(_) => perf.sqrt,
                Op::Sin(_) => perf.sin,
                Op::Cos(_) => perf.cos,
                Op::Tan(_) => perf.tan,
                Op::Asin(_) => perf.asin,
                Op::Acos(_) => perf.acos,
                Op::Atan(_) => perf.atan,
                Op::Ln(_) => perf.ln,
                Op::Log(_) => perf.log,
                Op::Exp(_) => perf.exp,
                Op::Exp10(_) => perf.pow10,
                // Internally uses _ + 0
                Op::StrToFloat(_) => perf.add,
                // Internally uses round(_)
                Op::BoolToFloat(_) => perf.round,
            };
            op_cost + get_value_cost(op.left(), perf) + get_value_cost(op.right(), perf)
        }
        Value::BoolOp(bop) => {
            let op_cost = match bop {
                BoolOp::And(_, _) => perf.and_,
                BoolOp::Or(_, _) => perf.or_,
                BoolOp::Eq(_, _) => perf.eq,
                BoolOp::Lt(_, _) => perf.lt,
                BoolOp::Gt(_, _) => perf.gt,
                BoolOp::Not(_) => perf.not_,
            };
            op_cost + get_value_cost(bop.left(), perf) + get_value_cost(bop.right(), perf)
        }
        Value::GetOfList(gol) => {
            let op_cost = match gol.op {
                ListOp::AtIndex => perf.at_index,
                _ => perf.index_of,
            };
            op_cost + get_value_cost(&gol.value, perf)
        }
        Value::GetList { .. } => perf.get_list,
        Value::GetListLength { .. } => perf.length_of_list,
        Value::CostumeInfo { op } => match op {
            CostumeInfoOp::Name => perf.cost_name,
            CostumeInfoOp::Number => perf.cost_num,
        },
        Value::GetCounter => perf.counter,
        Value::GetAnswer => perf.answer,
        // Temporary solution to prevent it from being elided across another var
        Value::DaysSince2000 => f64::INFINITY,
    };
    cost
}

pub fn assignment_elision_value(
    value: &Value,
    to_elide: &HashMap<String, Value>,
) -> (Value, bool) {
    match value {
        Value::GetVar { name } => {
            if let Some(replacement) = to_elide.get(name) {
                (replacement.clone(), true)
            } else {
                (value.clone(), false)
            }
        }
        Value::Op(op) => {
            let (left, lc) = assignment_elision_value(op.left(), to_elide);
            let (right, rc) = assignment_elision_value(op.right(), to_elide);
            if lc || rc {
                (Value::Op(op.with_values(left, right)), true)
            } else {
                (value.clone(), false)
            }
        }
        Value::BoolOp(bop) => {
            let (left, lc) = assignment_elision_value(bop.left(), to_elide);
            let (right, rc) = assignment_elision_value(bop.right(), to_elide);
            if lc || rc {
                (Value::BoolOp(bop.with_values(left, right)), true)
            } else {
                (value.clone(), false)
            }
        }
        Value::GetOfList(gol) => {
            let (val, vc) = assignment_elision_value(&gol.value, to_elide);
            if vc {
                (Value::GetOfList(crate::scratch::ast::GetOfList {
                    op: gol.op,
                    name: gol.name.clone(),
                    value: Box::new(val),
                }), true)
            } else {
                (value.clone(), false)
            }
        }
        _ => (value.clone(), false),
    }
}

pub fn assignment_elision_block(
    blocklist: &BlockList,
    to_elide: &HashMap<String, Value>,
) -> (BlockList, bool) {
    let mut new_blocks = Vec::new();
    let mut any_changed = false;

    for block in &blocklist.blocks {
        let (new_block, changed) = elide_block(block, to_elide);
        if changed {
            any_changed = true;
        }
        new_blocks.push(new_block);
    }

    let mut result = BlockList::new();
    for b in new_blocks {
        result.add_block(b);
    }

    (result, any_changed)
}

fn elide_block(block: &Block, to_elide: &HashMap<String, Value>) -> (Block, bool) {
    match block {
        Block::EditVar(data) => {
            let (new_value, changed) = assignment_elision_value(&data.value, to_elide);
            if changed {
                (Block::EditVar(crate::scratch::ast::EditVarData {
                    op: data.op,
                    name: data.name.clone(),
                    value: new_value,
                }), true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Say { value } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::Say { value: new_value }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::SwitchCostume { value } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::SwitchCostume { value: new_value }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::EditVolume { op, value } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::EditVolume { op: *op, value: new_value }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Broadcast { value, wait } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::Broadcast { value: new_value, wait: *wait }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Wait { value } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::Wait { value: new_value }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Ask { value, var_name } => {
            let (new_value, changed) = assignment_elision_value(value, to_elide);
            if changed {
                (Block::Ask { value: new_value, var_name: var_name.clone() }, true)
            } else {
                (block.clone(), false)
            }
        }
        Block::ControlFlow(cf) => {
            let mut changed = false;

            let new_condition = if let Some(cond) = &cf.condition {
                let (val, c) = assignment_elision_value(cond, to_elide);
                if c { changed = true; }
                Some(val)
            } else {
                None
            };

            let new_body = if let Some(body) = &cf.body {
                let (bl, c) = assignment_elision_block(body, to_elide);
                if c { changed = true; }
                Some(bl)
            } else {
                None
            };

            let new_else_body = if let Some(else_body) = &cf.else_body {
                let (bl, c) = assignment_elision_block(else_body, to_elide);
                if c { changed = true; }
                Some(bl)
            } else {
                None
            };

            if changed {
                (Block::ControlFlow(crate::scratch::ast::ControlFlow {
                    op: cf.op,
                    condition: new_condition,
                    var: cf.var.clone(),
                    body: new_body,
                    else_body: new_else_body,
                }), true)
            } else {
                (block.clone(), false)
            }
        }
        Block::ProcedureCall(data) => {
            let mut changed = false;
            let new_args: Vec<Value> = data.args.iter().map(|arg| {
                let (val, c) = assignment_elision_value(arg, to_elide);
                if c { changed = true; }
                val
            }).collect();

            if changed {
                (Block::ProcedureCall(crate::scratch::ast::ProcedureCallData {
                    name: data.name.clone(),
                    args: new_args,
                    run_without_refresh: data.run_without_refresh,
                }), true)
            } else {
                (block.clone(), false)
            }
        }
        Block::EditList(data) => {
            let mut changed = false;
            let new_index = if let Some(idx) = &data.index {
                let (val, c) = assignment_elision_value(idx, to_elide);
                if c { changed = true; }
                Some(val)
            } else {
                None
            };
            let new_value = if let Some(val) = &data.value {
                let (v, c) = assignment_elision_value(val, to_elide);
                if c { changed = true; }
                Some(v)
            } else {
                None
            };

            if changed {
                (Block::EditList(crate::scratch::ast::EditListData {
                    op: data.op,
                    name: data.name.clone(),
                    index: new_index,
                    value: new_value,
                }), true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Pen(pen_op) => {
            match pen_op {
                PenOp::SetColor { color } => {
                    let (new_color, changed) = assignment_elision_value(color, to_elide);
                    if changed {
                        (Block::Pen(PenOp::SetColor { color: new_color }), true)
                    } else {
                        (block.clone(), false)
                    }
                }
                PenOp::SetSize { size } => {
                    let (new_size, changed) = assignment_elision_value(size, to_elide);
                    if changed {
                        (Block::Pen(PenOp::SetSize { size: new_size }), true)
                    } else {
                        (block.clone(), false)
                    }
                }
                _ => (block.clone(), false),
            }
        }
        Block::MotionGoto { x, y } => {
            let (new_x, cx) = assignment_elision_value(x, to_elide);
            let (new_y, cy) = assignment_elision_value(y, to_elide);
            if cx || cy {
                (Block::MotionGoto { x: new_x, y: new_y }, true)
            } else {
                (block.clone(), false)
            }
        }
        _ => (block.clone(), false),
    }
}

fn collect_elisions(
    blocklist: &BlockList,
    cannot_elide: &HashSet<String>,
    var_use: &HashMap<String, f64>,
    perf: &TargetPerf,
    callee_mods: &HashMap<String, HashSet<String>>,
    callee_deps: &HashMap<String, HashSet<String>>,
    ignore_prefixed: &HashSet<String>,
) -> HashMap<String, Value> {
    // Each entry: (value, dependents). Dependents use the "var:"/"list:" prefix
    // convention to match `block_modifications`.
    let mut to_elide: IndexMap<String, (Value, HashSet<String>)> = IndexMap::new();
    // Variables whose dependencies were modified but may still be elided if not
    // read after this point. Mirrors Python's `changed_but_unread`.
    let mut changed_but_unread: IndexMap<String, (Value, HashSet<String>)> = IndexMap::new();
    let mut cannot: HashSet<String> = cannot_elide.clone();

    for (blk_idx, block) in blocklist.blocks.iter().enumerate() {
        // Determine if this block is an "ending" call. Python's
        // `getBlockListVarUse` excludes callee mods/deps for ending
        // ProcedureCall/Broadcast blocks, because elisions cannot happen
        // across ending calls. An ending call is one that is the last block
        // in the blocklist, or is immediately followed by a `stop this script`.
        let is_end_of_blocklist = blk_idx == blocklist.blocks.len() - 1;
        let is_ending = is_end_of_blocklist
            || matches!(
                blocklist.blocks.get(blk_idx + 1),
                Some(Block::StopScript(crate::scratch::ast::StopOption::This))
            );

        let is_call_block = matches!(block,
            Block::ProcedureCall(_) | Block::Broadcast { .. });

        // `cannot_write_before_read` is True for blocks that evaluate all their
        // inputs before performing any modification (i.e. everything except
        // ControlFlow, ProcedureCall, Broadcast). For those three, the block
        // may modify a dependency before reading the variable.
        let cannot_write_before_read = !is_call_block
            && !matches!(block, Block::ControlFlow(_));

        // Use unified is_ending-aware functions. For ending ProcedureCall/
        // Broadcast, callee mods/deps are excluded by the functions themselves.
        let block_mods = block_modifications_with_callees(block, callee_mods, ignore_prefixed, is_ending);

        // Combined reads (inputs + body) for `changed_but_unread` removal.
        let block_reads = block_variable_reads_combined(block, callee_deps, is_ending);

        // Body-only reads (excluding inputs) for `changed_but_unread` addition.
        // Only needed when the block can write before read; for other blocks
        // the cbu_add check short-circuits on `cannot_write_before_read`.
        let body_reads = if !cannot_write_before_read {
            block_variable_reads_body(block, callee_deps, is_ending)
        } else {
            HashSet::new()
        };

        // First, drop from `changed_but_unread` any variable read by this block:
        // it was read after its dependency was modified, so it cannot be elided.
        let cbu_remove: Vec<String> = changed_but_unread
            .keys()
            .filter(|var| block_reads.contains(*var))
            .cloned()
            .collect();
        for var in cbu_remove {
            changed_but_unread.shift_remove(&var);
        }

        // Process `to_elide`: remove variables that are overwritten or whose
        // dependencies are modified by this block.
        let mut remove = Vec::new();
        let mut cbu_add: Vec<(String, Value, HashSet<String>)> = Vec::new();
        for (var, (val, deps)) in &to_elide {
            let var_key = format!("var:{}", var);
            if block_mods.contains(&var_key) {
                // The variable itself was modified → permanently cannot elide.
                remove.push(var.clone());
            } else if deps.iter().any(|d| block_mods.contains(d)) {
                // A dependency was modified → remove from `to_elide`.
                remove.push(var.clone());
                // If the block cannot write before read, or doesn't read this
                // variable in its body (after the modification), the variable
                // was not read after the modification, so it may still be
                // elided. For ControlFlow/ProcedureCall/Broadcast, only body
                // reads count (inputs are evaluated before modification),
                // mirroring Python's two-phase `ignore_inputs=True` pass.
                if cannot_write_before_read || !body_reads.contains(var) {
                    cbu_add.push((var.clone(), val.clone(), deps.clone()));
                }
            }
        }
        for (var, val, deps) in cbu_add {
            changed_but_unread.insert(var, (val, deps));
        }
        for var in remove {
            to_elide.shift_remove(&var);
            cannot.insert(var);
        }

        // For non-EditVar blocks, all variable modifications prevent future
        // elision of those variables (mirrors Python's
        // `cannot_elide |= use.might_modify`).
        if !matches!(block, Block::EditVar(_)) {
            for mod_key in &block_mods {
                if let Some(name) = mod_key.strip_prefix("var:") {
                    cannot.insert(name.to_string());
                }
            }
        }

        // Add a new `set` assignment as a candidate for elision.
        if let Block::EditVar(data) = block {
            if data.op == VarOp::Set && !cannot.contains(&data.name) {
                let mut deps = HashSet::new();
                collect_value_dependencies(&data.value, &mut deps);
                let self_key = format!("var:{}", data.name);
                if !deps.contains(&self_key) {
                    to_elide.insert(data.name.clone(), (data.value.clone(), deps));
                }
            }
        }
    }

    // Variables in `changed_but_unread` were never read after their dependency
    // was modified, so they can still be elided.
    for (var, (val, deps)) in changed_but_unread {
        to_elide.insert(var, (val, deps));
    }

    let mut final_elisions: HashMap<String, Value> = HashMap::new();
    let mut current_deps: HashSet<String> = HashSet::new();
    for (var, (val, deps)) in to_elide {
        let times = var_use.get(&var).copied().unwrap_or(0.0);
        let should = should_elide(&val, times, perf);
        if !should {
            continue;
        }
        let var_key = format!("var:{}", var);
        if current_deps.contains(&var_key) || deps.iter().any(|d| current_deps.contains(d)) {
            continue;
        }
        final_elisions.insert(var, val);
        current_deps.insert(var_key);
        current_deps.extend(deps);
    }

    final_elisions
}

pub fn assignment_elision(
    proj: &Project,
    opt_target: &Target,
    dont_remove: Option<&HashSet<String>>,
    ignore_external_change: Option<&HashSet<String>>,
) -> (Project, bool) {
    let perf = &opt_target.perf;

    // `ignore_external_change` holds variable names (without "var:" prefix)
    // whose modification by callees should not block elision (e.g. the stack
    // pointer, which is restored by the prologue/epilogue).
    let ignore_set: HashSet<String> = ignore_external_change.cloned().unwrap_or_default();
    let ignore_prefixed: HashSet<String> = ignore_set
        .iter()
        .map(|v| format!("var:{}", v))
        .collect();

    let cannot_elide: HashSet<String> = dont_remove.cloned().unwrap_or_default();

    // Gather per-function dependency/modification information so that variables used
    // or modified by other functions are not elided. This matches Python's behaviour of
    // adding `other_info.dependent | other_info.might_modify | other_info.always_modify`
    // to `cannot_elide` for each function.
    let mut code_info: Vec<(HashSet<String>, HashSet<String>)> = Vec::new();
    for code in &proj.code {
        let mut deps = HashSet::new();
        let mut mods = HashSet::new();
        collect_blocklist_var_names(code, &mut deps, &mut mods);
        code_info.push((deps, mods));
    }

    // Build function-key → direct mods, function-key → direct deps, and
    // function-key → direct callees maps, then compute the transitive closure
    // of both modifications and dependencies. This mirrors Python's
    // `recu_fn_info` worklist algorithm which propagates callee
    // `might_modify`/`always_modify`/`dependent` to callers.
    let mut fn_direct_mods: HashMap<String, HashSet<String>> = HashMap::new();
    let mut fn_direct_deps: HashMap<String, HashSet<String>> = HashMap::new();
    let mut fn_direct_calls: HashMap<String, HashSet<String>> = HashMap::new();
    for (i, code) in proj.code.iter().enumerate() {
        if let Some(first) = code.blocks.first() {
            let key = fn_key_of(first);
            let mut mods = HashSet::new();
            collect_blocklist_modifications(code, &mut mods);
            fn_direct_mods.insert(key.clone(), mods);
            fn_direct_deps.insert(key.clone(), code_info[i].0.clone());

            let mut calls = HashSet::new();
            collect_blocklist_callees(code, &mut calls);
            fn_direct_calls.insert(key, calls);
        }
    }
    // Single-pass propagation of modifications in `proj.code` order. This
    // deliberately mirrors a bug in Python's `recu_fn_info` worklist:
    // `info = recu_fn_info[name]` returns a reference (not a copy), so
    // `info != recu_fn_info[name]` is always False and the worklist is never
    // refilled. Each function is therefore processed exactly once, in
    // `proj.code` order, and sees only the already-processed callees'
    // propagated state. Callees that appear later in `proj.code` contribute
    // only their direct modifications.
    //
    // Concretely, for `fib` (which calls `fib:6`/`fib:8` ending, which call
    // `fib:16` which sets `!return value`), `fib` is processed before
    // `fib:6`/`fib:8`/`fib:16`, so `fn_mods["func:fib"]` does NOT include
    // `!return value`. A correct fixed-point iteration would include it.
    let mut fn_mods: HashMap<String, HashSet<String>> = fn_direct_mods.clone();
    for code in &proj.code {
        let key = match code.blocks.first() {
            Some(b) => fn_key_of(b),
            None => continue,
        };
        let calls = match fn_direct_calls.get(&key) {
            Some(c) => c.clone(),
            None => continue,
        };
        let mut merged = match fn_mods.get(&key) {
            Some(m) => m.clone(),
            None => HashSet::new(),
        };
        for callee in &calls {
            if let Some(callee_mods) = fn_mods.get(callee) {
                for m in callee_mods {
                    merged.insert(m.clone());
                }
            }
        }
        fn_mods.insert(key, merged);
    }
    // Transitive closure of dependencies (fixed-point iteration). Used by the
    // `changed_but_unread` body-only-reads check to determine whether a
    // ProcedureCall/Broadcast reads a variable in its (transitive) body.
    let mut fn_deps: HashMap<String, HashSet<String>> = fn_direct_deps.clone();
    loop {
        let mut changed = false;
        let keys: Vec<String> = fn_direct_calls.keys().cloned().collect();
        for key in keys {
            let calls = match fn_direct_calls.get(&key) {
                Some(c) => c.clone(),
                None => continue,
            };
            let mut merged = match fn_deps.get(&key) {
                Some(d) => d.clone(),
                None => HashSet::new(),
            };
            for callee in &calls {
                if let Some(callee_deps) = fn_deps.get(callee) {
                    for d in callee_deps {
                        if merged.insert(d.clone()) {
                            changed = true;
                        }
                    }
                }
            }
            fn_deps.insert(key, merged);
        }
        if !changed {
            break;
        }
    }

    let mut new_proj = proj.clone();
    let mut any_changed = false;

    for (i, code) in new_proj.code.iter_mut().enumerate() {
        let mut per_code_cannot_elide = cannot_elide.clone();
        for (j, (deps, mods)) in code_info.iter().enumerate() {
            if i != j {
                per_code_cannot_elide.extend(deps.iter().cloned());
                per_code_cannot_elide.extend(mods.iter().cloned());
            }
        }

        let var_use = get_blocklist_var_use(code, None);
        let elisions = collect_elisions(
            code,
            &per_code_cannot_elide,
            &var_use,
            perf,
            &fn_mods,
            &fn_deps,
            &ignore_prefixed,
        );

        if elisions.is_empty() {
            continue;
        }

        let mut blocks = Vec::new();
        for block in &code.blocks {
            if let Block::EditVar(data) = block {
                if data.op == VarOp::Set && elisions.contains_key(&data.name) {
                    any_changed = true;
                    continue;
                }
            }

            let (new_block, changed) = elide_block(block, &elisions);
            if changed {
                any_changed = true;
            }
            blocks.push(new_block);
        }

        let mut new_code = BlockList::new();
        for b in blocks {
            new_code.add_block(b);
        }
        *code = new_code;
    }

    (new_proj, any_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ast::{KnownVal, Op};
    use crate::target::{Target, TargetPerf, TargetInfo, TargetExec, BranchMethod};

    fn test_perf() -> TargetPerf {
        TargetPerf {
            cost_num: 0.0,
            cost_name: 0.0,
            counter: 0.0,
            answer: 0.0,
            add: 1.0,
            sub: 1.0,
            mul: 1.0,
            div: 1.0,
            rand: 1.0,
            gt: 1.0,
            lt: 1.0,
            eq: 1.0,
            and_: 1.0,
            or_: 1.0,
            not_: 1.0,
            join: 1.0,
            letter_of: 1.0,
            length_of_str: 1.0,
            contains_str: 1.0,
            r#mod: 1.0,
            round: 1.0,
            abs: 1.0,
            floor: 1.0,
            ceil: 1.0,
            sqrt: 1.0,
            sin: 1.0,
            cos: 1.0,
            tan: 1.0,
            asin: 1.0,
            acos: 1.0,
            atan: 1.0,
            ln: 1.0,
            log: 1.0,
            exp: 1.0,
            pow10: 1.0,
            get_var: 1.0,
            set_var: 1.0,
            get_list: 1.0,
            at_index: 1.0,
            index_of: 1.0,
            length_of_list: 1.0,
            param: 0.0,
        }
    }

    fn test_target() -> Target {
        Target {
            id: "test".to_string(),
            info: TargetInfo {
                name: "test".to_string(),
                url: String::new(),
                desc: String::new(),
                formats: vec!["scratch3".to_string()],
            },
            exec: TargetExec {
                preferred_branch_method: BranchMethod::ProcCall,
                compiler_type_hints: false,
                max_branch_recursion: 100,
                preferred_branch_recursion: 50,
            },
            perf: test_perf(),
        }
    }

    #[test]
    fn test_get_value_var_use() {
        let val = Value::Op(Op::Add(
            Box::new(Value::GetVar { name: "x".to_string() }),
            Box::new(Value::Known(KnownVal::Num(1.0))),
        ));
        let reads = get_value_var_use(&val);
        assert!(reads.contains("x"));
        assert_eq!(reads.len(), 1);
    }

    #[test]
    fn test_should_elide_used_once() {
        let val = Value::Known(KnownVal::Num(5.0));
        let perf = test_perf();
        assert!(should_elide(&val, 1.0, &perf));
    }

    #[test]
    fn test_should_elide_used_many() {
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(1.0))),
            Box::new(Value::Known(KnownVal::Num(2.0))),
        ));
        let mut perf = test_perf();
        perf.get_var = 0.01;
        assert!(!should_elide(&val, 100.0, &perf));
    }

    #[test]
    fn test_assignment_elision_value() {
        let mut to_elide = HashMap::new();
        to_elide.insert("x".to_string(), Value::Known(KnownVal::Num(5.0)));

        let val = Value::GetVar { name: "x".to_string() };
        let (result, changed) = assignment_elision_value(&val, &to_elide);
        assert!(changed);
        assert_eq!(result, Value::Known(KnownVal::Num(5.0)));
    }

    #[test]
    fn test_assignment_elision_no_change() {
        let to_elide = HashMap::new();
        let val = Value::GetVar { name: "x".to_string() };
        let (_, changed) = assignment_elision_value(&val, &to_elide);
        assert!(!changed);
    }

    #[test]
    fn test_assignment_elision_project() {
        let proj = Project::new(crate::scratch::ScratchConfig::default());
        let target = test_target();
        let (new_proj, _) = assignment_elision(&proj, &target, None, None);
        assert_eq!(new_proj.code.len(), proj.code.len());
    }
}

