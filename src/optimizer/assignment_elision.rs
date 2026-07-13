use std::collections::{HashMap, HashSet};

use crate::scratch::{Block, BlockList, Project, Value};
use crate::scratch::ast::VarOp;
use crate::target::{Target, TargetPerf};

use super::BlockListInfo;

pub fn get_value_var_use(value: &Value) -> (HashSet<String>, HashMap<String, usize>) {
    let mut reads = HashSet::new();
    let mut writes: HashMap<String, usize> = HashMap::new();

    collect_value_var_use(value, &mut reads, &mut writes);

    (reads, writes)
}

fn collect_value_var_use(value: &Value, reads: &mut HashSet<String>, writes: &mut HashMap<String, usize>) {
    match value {
        Value::GetVar { name } => {
            reads.insert(name.clone());
        }
        Value::Op(op) => {
            collect_value_var_use(op.left(), reads, writes);
            collect_value_var_use(op.right(), reads, writes);
        }
        Value::BoolOp(bop) => {
            collect_value_var_use(bop.left(), reads, writes);
            collect_value_var_use(bop.right(), reads, writes);
        }
        Value::GetOfList(gol) => {
            collect_value_var_use(&gol.value, reads, writes);
        }
        Value::Known(_) | Value::KnownBool(_) | Value::GetParam { .. } |
        Value::GetList { .. } | Value::GetListLength { .. } |
        Value::CostumeInfo { .. } | Value::GetCounter | Value::GetAnswer | Value::DaysSince2000 => {}
    }
}

pub fn get_blocklist_var_use(
    blocklist: &BlockList,
    func_info: Option<&HashMap<String, BlockListInfo>>,
) -> HashMap<String, f64> {
    let mut times_used: HashMap<String, f64> = HashMap::new();

    for block in &blocklist.blocks {
        match block {
            Block::EditVar(data) => {
                let (reads, _) = get_value_var_use(&data.value);
                for var in reads {
                    *times_used.entry(var).or_insert(0.0) += 1.0;
                }
                if data.op != VarOp::Set {
                    *times_used.entry(data.name.clone()).or_insert(0.0) += 1.0;
                }
            }
            Block::Say { value } => {
                let (reads, _) = get_value_var_use(value);
                for var in reads {
                    *times_used.entry(var).or_insert(0.0) += 1.0;
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(cond) = &cf.condition {
                    let (reads, _) = get_value_var_use(cond);
                    for var in reads {
                        *times_used.entry(var).or_insert(0.0) += 1.0;
                    }
                }
                if let Some(body) = &cf.body {
                    let body_use = get_blocklist_var_use(body, func_info);
                    for (var, count) in body_use {
                        *times_used.entry(var).or_insert(0.0) += count;
                    }
                }
                if let Some(else_body) = &cf.else_body {
                    let else_use = get_blocklist_var_use(else_body, func_info);
                    for (var, count) in else_use {
                        *times_used.entry(var).or_insert(0.0) += count;
                    }
                }
            }
            Block::ProcedureDef(data) => {
                let _ = &data.name;
            }
            Block::ProcedureCall(data) => {
                for arg in &data.args {
                    let (reads, _) = get_value_var_use(arg);
                    for var in reads {
                        *times_used.entry(var).or_insert(0.0) += 1.0;
                    }
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
    cost * times_used <= cost + times_used * perf.get_var
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
            perf.get_list + get_value_cost(&gol.value, perf)
        }
        Value::GetList { .. } | Value::GetListLength { .. } => perf.get_list,
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

    let mut new_proj = proj.clone();
    let mut any_changed = false;

    for (_code_idx, code) in new_proj.code.iter_mut().enumerate() {
        let var_use = get_blocklist_var_use(code, None);
        let mut to_elide: HashMap<String, Value> = HashMap::new();

        let mut blocks = Vec::new();
        for block in &code.blocks {
            if let Block::EditVar(data) = block
                && data.op == VarOp::Set {
                    let is_protected = dont_remove.map_or(false, |dr| dr.contains(&data.name));
                    let times = var_use.get(&data.name).copied().unwrap_or(0.0);
                    let elide = !is_protected && should_elide(&data.value, times, perf);
                    if elide {
                        to_elide.insert(data.name.clone(), data.value.clone());
                        continue;
                    } else {
                        to_elide.remove(&data.name);
                    }
                }

            let (new_block, changed) = elide_block(block, &to_elide);
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
        let (reads, _) = get_value_var_use(&val);
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