use std::collections::{HashMap, HashSet};
use indexmap::IndexMap;

use crate::scratch::{Block, BlockList, Project, Value};
use crate::scratch::ast::{ControlOp, KnownVal, ListOp, VarOp};
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

    fn add_reads(times: &mut HashMap<String, f64>, reads: HashSet<String>, multiplier: f64) {
        for var in reads {
            *times.entry(var).or_insert(0.0) += multiplier;
        }
    }

    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                add_reads(&mut times_used, get_value_var_use(&data.value), 1.0);
                if data.op != VarOp::Set {
                    *times_used.entry(data.name.clone()).or_insert(0.0) += 1.0;
                }
            }
            Block::Say { value }
            | Block::SwitchCostume { value }
            | Block::EditVolume { value, .. }
            | Block::Broadcast { value, .. }
            | Block::Wait { value } => {
                add_reads(&mut times_used, get_value_var_use(value), 1.0);
            }
            Block::Ask { value, .. } => {
                add_reads(&mut times_used, get_value_var_use(value), 1.0);
            }
            Block::EditList(data) => {
                if let Some(idx) = &data.index {
                    add_reads(&mut times_used, get_value_var_use(idx), 1.0);
                }
                if let Some(val) = &data.value {
                    add_reads(&mut times_used, get_value_var_use(val), 1.0);
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    let cond_mult = match cf.op {
                        ControlOp::Until | ControlOp::While | ControlOp::Forever => LOOP_USE_MULTIPLIER,
                        _ => 1.0,
                    };
                    add_reads(&mut times_used, get_value_var_use(cond), cond_mult);
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
                    add_reads(&mut times_used, get_value_var_use(arg), 1.0);
                }
                if let Some(info) = func_info
                    && let Some(bli) = info.get(&data.name) {
                        for (var, count) in &bli.times_used {
                            *times_used.entry(var.clone()).or_insert(0.0) += count;
                        }
                    }
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
fn collect_blocklist_var_names(
    blocklist: &BlockList,
    deps: &mut HashSet<String>,
    mods: &mut HashSet<String>,
) {
    fn add_var_deps(value: &Value, deps: &mut HashSet<String>) {
        let mut d = HashSet::new();
        collect_value_dependencies(value, &mut d);
        for dep in d {
            if let Some(stripped) = dep.strip_prefix("var:") {
                deps.insert(stripped.to_string());
            }
        }
    }

    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                add_var_deps(&data.value, deps);
                mods.insert(data.name.clone());
                if data.op != VarOp::Set {
                    deps.insert(data.name.clone());
                }
            }
            Block::Say { value }
            | Block::SwitchCostume { value }
            | Block::EditVolume { value, .. }
            | Block::Broadcast { value, .. }
            | Block::Wait { value } => {
                add_var_deps(value, deps);
            }
            Block::Ask { value, var_name } => {
                add_var_deps(value, deps);
                if let Some(name) = var_name {
                    mods.insert(name.clone());
                    deps.insert(name.clone());
                }
            }
            Block::EditList(data) => {
                if let Some(idx) = &data.index {
                    add_var_deps(idx, deps);
                }
                if let Some(val) = &data.value {
                    add_var_deps(val, deps);
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    add_var_deps(cond, deps);
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
                    add_var_deps(arg, deps);
                }
            }
            _ => {}
        }
    }
}

fn block_modifications(block: &Block) -> HashSet<String> {
    let mut mods = HashSet::new();
    collect_block_modifications(block, &mut mods);
    mods
}

pub fn get_value_cost(value: &Value, perf: &TargetPerf) -> f64 {
    match value {
        Value::Known(_) | Value::KnownBool(_) => 0.0,
        Value::GetVar { .. } => perf.get_var,
        Value::GetParam { .. } => 0.0,
        Value::Op(op) => {
            perf.add
                + get_value_cost(op.left(), perf)
                + get_value_cost(op.right(), perf)
        }
        Value::BoolOp(bop) => {
            perf.eq
                + get_value_cost(bop.left(), perf)
                + get_value_cost(bop.right(), perf)
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
        Value::CostumeInfo { .. } | Value::GetCounter | Value::GetAnswer | Value::DaysSince2000 => 0.0,
    }
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
        _ => (block.clone(), false),
    }
}

fn collect_elisions(
    blocklist: &BlockList,
    cannot_elide: &HashSet<String>,
    var_use: &HashMap<String, f64>,
    perf: &TargetPerf,
) -> HashMap<String, Value> {
    let mut to_elide: IndexMap<String, (Value, HashSet<String>)> = IndexMap::new();
    let mut cannot: HashSet<String> = cannot_elide.clone();

    for block in &blocklist.blocks {
        let block_mods = block_modifications(block);

        let mut remove = Vec::new();
        for (var, (_, deps)) in &to_elide {
            let var_key = format!("var:{}", var);
            if block_mods.contains(&var_key) || deps.iter().any(|d| block_mods.contains(d)) {
                remove.push(var.clone());
            }
        }
        for var in remove {
            to_elide.shift_remove(&var);
            cannot.insert(var);
        }

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

    let mut final_elisions: HashMap<String, Value> = HashMap::new();
    let mut current_deps: HashSet<String> = HashSet::new();
    for (var, (val, deps)) in to_elide {
        let times = var_use.get(&var).copied().unwrap_or(0.0);
        if !should_elide(&val, times, perf) {
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

fn collect_all_variable_names(blocklist: &BlockList, names: &mut HashSet<String>) {
    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                names.insert(data.name.clone());
                collect_value_var_names(&data.value, names);
            }
            Block::Ask { var_name: Some(name), .. } => {
                names.insert(name.clone());
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    collect_value_var_names(cond, names);
                }
                if let Some(body) = &cf.body {
                    collect_all_variable_names(body, names);
                }
                if let Some(else_body) = &cf.else_body {
                    collect_all_variable_names(else_body, names);
                }
            }
            Block::EditList(data) => {
                if let Some(idx) = &data.index {
                    collect_value_var_names(idx, names);
                }
                if let Some(val) = &data.value {
                    collect_value_var_names(val, names);
                }
            }
            Block::Say { value }
            | Block::SwitchCostume { value }
            | Block::EditVolume { value, .. }
            | Block::Broadcast { value, .. }
            | Block::Wait { value } => {
                collect_value_var_names(value, names);
            }
            Block::ProcedureCall(data) => {
                for arg in &data.args {
                    collect_value_var_names(arg, names);
                }
            }
            _ => {}
        }
    }
}

fn collect_value_var_names(value: &Value, names: &mut HashSet<String>) {
    match value {
        Value::GetVar { name } => { names.insert(name.clone()); }
        Value::Op(op) => {
            collect_value_var_names(op.left(), names);
            collect_value_var_names(op.right(), names);
        }
        Value::BoolOp(bop) => {
            collect_value_var_names(bop.left(), names);
            collect_value_var_names(bop.right(), names);
        }
        Value::GetOfList(gol) => {
            names.insert(gol.name.clone());
            collect_value_var_names(&gol.value, names);
        }
        Value::GetList { name } | Value::GetListLength { name } => {
            names.insert(name.clone());
        }
        _ => {}
    }
}

pub fn assignment_elision(
    proj: &Project,
    targets: &[Target],
    dont_remove: Option<&HashSet<String>>,
) -> (Project, bool) {
    let perf = if let Some(t) = targets.first() {
        &t.perf
    } else {
        return (proj.clone(), false);
    };

    let mut cannot_elide: HashSet<String> = dont_remove.cloned().unwrap_or_default();
    // Protect indexed return values (e.g. "!return value:0") the same way the Python
    // implementation does by extending the dont_remove set with all such names.
    let return_prefix: Option<String> = cannot_elide.iter().find_map(|n| {
        if n.starts_with("!return value") {
            Some("!return value".to_string())
        } else {
            None
        }
    });
    if let Some(prefix) = return_prefix {
        let mut all_names = HashSet::new();
        for code in &proj.code {
            collect_all_variable_names(code, &mut all_names);
        }
        for name in all_names {
            if name.starts_with(&prefix) {
                cannot_elide.insert(name);
            }
        }
    }

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
        let elisions = collect_elisions(code, &per_code_cannot_elide, &var_use, perf);

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
        let targets = vec![test_target()];
        let (new_proj, _) = assignment_elision(&proj, &targets, None);
        assert_eq!(new_proj.code.len(), proj.code.len());
    }
}

