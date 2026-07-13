use std::collections::{HashMap, HashSet};

use crate::ir;
use super::config::BlockVarUse;

/// Collect the values used by an instruction.
fn instr_values(instr: &ir::Instr, include_called_funcs: bool) -> Vec<ir::Value> {
    match instr {
        ir::Instr::Unreachable
        | ir::Instr::Alloca(_)
        | ir::Instr::Phi(_)
        | ir::Instr::ExtractElement(_)
        | ir::Instr::InsertElement(_)
        | ir::Instr::ShuffleVector(_) => Vec::new(),
        ir::Instr::Ret(r) => r.value.clone().into_iter().collect(),
        ir::Instr::Conversion(c) => vec![c.value.clone()],
        ir::Instr::Freeze(f) => vec![f.value.clone()],
        ir::Instr::Load(l) => vec![l.address.clone()],
        ir::Instr::Store(s) => vec![s.address.clone(), s.value.clone()],
        ir::Instr::Call(c) => {
            let mut vals = c.args.clone();
            if include_called_funcs {
                vals.insert(0, c.func.clone());
            }
            vals
        }
        ir::Instr::UnaryOp(u) => vec![u.operand.clone()],
        ir::Instr::BinaryOp(b) => vec![b.left.clone(), b.right.clone()],
        ir::Instr::ICmp(c) => vec![c.left.clone(), c.right.clone()],
        ir::Instr::FCmp(c) => vec![c.left.clone(), c.right.clone()],
        ir::Instr::UncondBr(_) => Vec::new(),
        ir::Instr::CondBr(c) => vec![c.cond.clone()],
        ir::Instr::Switch(s) => vec![s.cond.clone()],
        ir::Instr::Select(s) => vec![s.cond.clone(), s.true_value.clone(), s.false_value.clone()],
        ir::Instr::GetElementPtr(g) => {
            let mut vals = vec![g.base_ptr.clone()];
            vals.extend(g.indices.clone());
            vals
        }
        ir::Instr::ExtractValue(e) => vec![e.agg.clone()],
        ir::Instr::InsertValue(i) => vec![i.agg.clone(), i.element.clone()],
        ir::Instr::VaArg(v) => vec![v.arglist.clone()],
    }
}

/// Labels a terminator may branch to. Ret branches to the special "ret" label.
fn terminator_branch_labels(instr: &ir::Instr) -> HashSet<String> {
    let mut labels = HashSet::new();
    match instr {
        ir::Instr::Ret(_) => {
            labels.insert("ret".to_string());
        }
        ir::Instr::UncondBr(b) => {
            labels.insert(b.branch.label.clone());
        }
        ir::Instr::CondBr(b) => {
            labels.insert(b.branch_true.label.clone());
            labels.insert(b.branch_false.label.clone());
        }
        ir::Instr::Switch(s) => {
            labels.insert(s.branch_default.label.clone());
            for (_, label) in &s.branch_table {
                labels.insert(label.label.clone());
            }
        }
        _ => {}
    }
    labels
}

#[derive(Debug, Clone)]
struct NodeInfo {
    depends: HashSet<String>,
    modifies: HashSet<String>,
    calls: HashSet<String>,
    direct_modifies: HashSet<String>,
    direct_calls: HashSet<String>,
}

fn analyze_node(
    name: &str,
    info: &mut HashMap<String, NodeInfo>,
    analyzed: &mut HashSet<String>,
) -> bool {
    let direct_calls: Vec<String> = info[name].direct_calls.iter().cloned().collect();
    let mut changed = false;
    for callee in direct_calls {
        if analyzed.insert(callee.clone()) {
            changed = analyze_node(&callee, info, analyzed) || changed;
        }
        let (callee_depends, callee_modifies, callee_calls) = {
            let c = &info[&callee];
            (c.depends.clone(), c.modifies.clone(), c.calls.clone())
        };
        let direct_modifies = info[name].direct_modifies.clone();
        let new_modifies = &callee_modifies - &info[name].modifies;
        let new_depends = &(&callee_depends - &direct_modifies) - &info[name].depends;
        let new_calls = &callee_calls - &info[name].calls;
        if !new_modifies.is_empty() || !new_depends.is_empty() || !new_calls.is_empty() {
            let node = info.get_mut(name).unwrap();
            node.modifies.extend(new_modifies);
            node.depends.extend(new_depends);
            node.calls.extend(new_calls);
            changed = true;
        }
    }
    changed
}

fn analyze(entrypoint: &str, info: &mut HashMap<String, NodeInfo>) {
    loop {
        let mut analyzed = HashSet::new();
        if !analyze_node(entrypoint, info, &mut analyzed) {
            break;
        }
    }
}

/// Compute transitive block variable use for a function, matching Python's
/// `getFuncBranchesVarUse`.
pub fn analyze_function_block_var_use(
    func: &ir::Function,
    outgoing_phi_values: &HashMap<String, HashMap<String, Vec<ir::Value>>>,
) -> HashMap<String, BlockVarUse> {
    let mut direct: HashMap<String, BlockVarUse> = HashMap::new();

    for (label, block) in &func.blocks {
        let mut res = BlockVarUse::default();
        for instr in &block.instrs {
            let mut vals = instr_values(instr, true);
            if let Some(outgoing) = outgoing_phi_values.get(label) {
                if matches!(
                    instr,
                    ir::Instr::UncondBr(_) | ir::Instr::CondBr(_) | ir::Instr::Switch(_)
                ) {
                    let targets = terminator_branch_labels(instr);
                    for (target, phi_vals) in outgoing {
                        if targets.contains(target) {
                            vals.extend(phi_vals.iter().cloned());
                        }
                    }
                }
            }

            let mut instr_depends = HashSet::new();
            for val in vals {
                match val {
                    ir::Value::Argument(arg) => {
                        instr_depends.insert(arg.name.clone());
                    }
                    ir::Value::LocalVar(lv) => {
                        instr_depends.insert(lv.name.clone());
                    }
                    _ => {}
                }
            }
            res.depends.extend(&instr_depends - &res.modifies);

            if let Some(result) = instr.result() {
                res.modifies.insert(result.name.clone());
            }
        }

        if let Some(last) = block.instrs.last() {
            res.branches = &terminator_branch_labels(last) - &HashSet::from(["ret".to_string()]);
        }

        direct.insert(label.clone(), res);
    }

    let entrypoint = func.blocks.keys().next().cloned().unwrap_or_default();
    let mut info: HashMap<String, NodeInfo> = HashMap::new();
    for (label, var_use) in &direct {
        info.insert(
            label.clone(),
            NodeInfo {
                depends: var_use.depends.clone(),
                modifies: var_use.modifies.clone(),
                calls: var_use.branches.clone(),
                direct_modifies: var_use.modifies.clone(),
                direct_calls: var_use.branches.clone(),
            },
        );
    }

    analyze(&entrypoint, &mut info);

    info.into_iter()
        .map(|(label, node_info)| {
            let direct_branches = direct.get(&label).map(|u| u.branches.clone()).unwrap_or_default();
            (
                label,
                BlockVarUse {
                    depends: node_info.depends,
                    modifies: node_info.modifies,
                    branches: direct_branches,
                    depends_var_sizes: HashMap::new(),
                },
            )
        })
        .collect()
}

pub fn find_nodes_with_cycle(graph: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let nodes: Vec<String> = graph.keys().cloned().collect();
    let mut in_cycle = HashSet::new();

    for node in &nodes {
        if graph.get(node).map_or(false, |edges| edges.contains(node)) {
            in_cycle.insert(node.clone());
        }
    }

    let mut index = 0usize;
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut lowlinks: HashMap<String, usize> = HashMap::new();
    let mut sccs: Vec<HashSet<String>> = Vec::new();

    fn strongconnect(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        index: &mut usize,
        stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        indices: &mut HashMap<String, usize>,
        lowlinks: &mut HashMap<String, usize>,
        sccs: &mut Vec<HashSet<String>>,
    ) {
        indices.insert(node.to_string(), *index);
        lowlinks.insert(node.to_string(), *index);
        *index += 1;
        stack.push(node.to_string());
        on_stack.insert(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !indices.contains_key(neighbor) {
                    strongconnect(neighbor, graph, index, stack, on_stack, indices, lowlinks, sccs);
                    let low = lowlinks.get(neighbor).copied().unwrap_or(0);
                    let entry = lowlinks.get_mut(node).unwrap();
                    *entry = (*entry).min(low);
                } else if on_stack.contains(neighbor) {
                    let idx = indices.get(neighbor).copied().unwrap_or(0);
                    let entry = lowlinks.get_mut(node).unwrap();
                    *entry = (*entry).min(idx);
                }
            }
        }

        if lowlinks.get(node) == indices.get(node) {
            let mut scc = HashSet::new();
            loop {
                let w = stack.pop().unwrap();
                on_stack.remove(&w);
                scc.insert(w.clone());
                if w == node {
                    break;
                }
            }
            sccs.push(scc);
        }
    }

    for node in &nodes {
        if !indices.contains_key(node) {
            strongconnect(node, graph, &mut index, &mut stack, &mut on_stack, &mut indices, &mut lowlinks, &mut sccs);
        }
    }

    for scc in sccs {
        if scc.len() > 1 {
            in_cycle.extend(scc);
        }
    }

    in_cycle
}

fn find_all_simple_cycles(graph: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    let nodes: Vec<String> = graph.keys().cloned().collect();
    let mut cycles = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut path: Vec<String> = Vec::new();

    fn dfs(
        current: &str,
        start: &str,
        graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        path: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        if let Some(neighbors) = graph.get(current) {
            for neighbor in neighbors {
                if neighbor == start && path.len() >= 1 {
                    cycles.push(path.clone());
                } else if !visited.contains(neighbor) {
                    visited.insert(neighbor.to_string());
                    path.push(neighbor.to_string());
                    dfs(neighbor, start, graph, visited, path, cycles);
                    path.pop();
                    visited.remove(neighbor);
                }
            }
        }
    }

    for start in &nodes {
        path.clear();
        path.push(start.clone());
        visited.clear();
        visited.insert(start.clone());
        dfs(start, start, graph, &mut visited, &mut path, &mut cycles);
    }

    cycles
}

pub fn select_cycle_checks(graph: &HashMap<String, Vec<String>>) -> Vec<String> {
    let cycles = find_all_simple_cycles(graph);
    if cycles.is_empty() {
        return Vec::new();
    }

    let mut self_loop_nodes: HashSet<String> = HashSet::new();
    let mut remaining_cycles: Vec<HashSet<String>> = Vec::new();

    for cycle in &cycles {
        if cycle.len() == 1 {
            self_loop_nodes.insert(cycle[0].clone());
        } else {
            remaining_cycles.push(cycle.iter().cloned().collect());
        }
    }

    remaining_cycles.retain(|cycle| cycle.is_disjoint(&self_loop_nodes));

    if remaining_cycles.is_empty() {
        let mut result: Vec<String> = self_loop_nodes.into_iter().collect();
        result.sort();
        return result;
    }

    let all_nodes: HashSet<String> = remaining_cycles.iter().flatten().cloned().collect();
    let mut all_nodes_list: Vec<String> = all_nodes.iter().cloned().collect();
    all_nodes_list.sort();
    let exact = all_nodes.len() <= 15;

    let mut greedy_nodes: HashSet<String> = HashSet::new();
    let mut uncovered: Vec<HashSet<String>> = remaining_cycles.clone();

    while !uncovered.is_empty() {
        let mut best_node: Option<String> = None;
        let mut best_count = 0usize;

        for node in &all_nodes_list {
            let count = uncovered.iter().filter(|cycle| cycle.contains(node)).count();
            if count > best_count {
                best_count = count;
                best_node = Some(node.clone());
            }
        }

        if let Some(node) = best_node {
            greedy_nodes.insert(node.clone());
            uncovered.retain(|cycle| !cycle.contains(&node));
        } else {
            break;
        }
    }

    let hitting_set = if exact {
        match min_hitting_set_exact(&remaining_cycles, &all_nodes_list) {
            Some(set) => set,
            None => greedy_nodes,
        }
    } else {
        greedy_nodes
    };

    let mut result: Vec<String> = self_loop_nodes.union(&hitting_set).cloned().collect();
    result.sort();
    result
}

fn min_hitting_set_exact(
    cycles: &[HashSet<String>],
    all_nodes: &[String],
) -> Option<HashSet<String>> {
    if cycles.is_empty() {
        return Some(HashSet::new());
    }

    let sorted_nodes: Vec<String> = all_nodes.to_vec();
    for r in 1..=sorted_nodes.len() {
        let mut indices: Vec<usize> = (0..r).collect();
        loop {
            let set: HashSet<String> = indices.iter().map(|&i| sorted_nodes[i].clone()).collect();
            if cycles.iter().all(|cycle| !cycle.is_disjoint(&set)) {
                return Some(set);
            }
            if !next_combination(&mut indices, sorted_nodes.len()) {
                break;
            }
        }
    }

    None
}

fn next_combination(indices: &mut [usize], n: usize) -> bool {
    let k = indices.len();
    if k == 0 {
        return false;
    }
    for i in (0..k).rev() {
        if indices[i] < n - k + i {
            indices[i] += 1;
            for j in i + 1..k {
                indices[j] = indices[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

pub fn unavoidable_nodes(
    graph: &HashMap<String, Vec<String>>,
    source: &str,
    target: &str,
) -> HashSet<String> {
    let mut result = HashSet::new();
    result.insert(source.to_string());

    if !graph.contains_key(target) {
        return result;
    }

    let mut reachable_from_source = HashSet::new();
    reachable_from_source.insert(source.to_string());
    let mut stack = vec![source.to_string()];
    while let Some(node) = stack.pop() {
        if let Some(neighbors) = graph.get(&node) {
            for neighbor in neighbors {
                if reachable_from_source.insert(neighbor.clone()) {
                    stack.push(neighbor.clone());
                }
            }
        }
    }

    if !reachable_from_source.contains(target) {
        return result;
    }

    let nodes: Vec<String> = reachable_from_source.into_iter().collect();
    for candidate in &nodes {
        if candidate == source || candidate == target {
            continue;
        }
        let mut visited = HashSet::new();
        visited.insert(source.to_string());
        let mut stack = vec![source.to_string()];
        let mut can_avoid = false;
        while let Some(node) = stack.pop() {
            if node == *target {
                can_avoid = true;
                break;
            }
            if let Some(neighbors) = graph.get(&node) {
                for neighbor in neighbors {
                    if neighbor == candidate || visited.contains(neighbor) {
                        continue;
                    }
                    visited.insert(neighbor.clone());
                    stack.push(neighbor.clone());
                }
            }
        }
        if !can_avoid {
            result.insert(candidate.clone());
        }
    }

    result.insert(target.to_string());
    result
}
