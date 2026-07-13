use std::collections::{HashMap, HashSet};

use petgraph::algo::kosaraju_scc;
use petgraph::graph::DiGraph;

pub fn find_nodes_with_cycle(graph: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let nodes: Vec<&String> = graph.keys().collect();
    let mut node_idx: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();
    let mut g = DiGraph::<(), ()>::new();

    for node in &nodes {
        let idx = g.add_node(());
        node_idx.insert(node.as_str(), idx);
    }

    for (u, neighbors) in graph {
        if let Some(&u_idx) = node_idx.get(u.as_str()) {
            for v in neighbors {
                if let Some(&v_idx) = node_idx.get(v.as_str()) {
                    g.add_edge(u_idx, v_idx, ());
                }
            }
        }
    }

    let sccs = kosaraju_scc(&g);
    let mut result = HashSet::new();

    for scc in &sccs {
        if scc.len() > 1 {
            for &idx in scc {
                result.insert(nodes[idx.index()].clone());
            }
        } else if scc.len() == 1 {
            let idx = scc[0];
            if g.neighbors(idx).any(|n| n == idx) {
                result.insert(nodes[idx.index()].clone());
            }
        }
    }

    result
}

pub fn select_cycle_checks(graph: &HashMap<String, Vec<String>>) -> Vec<String> {
    let nodes_with_cycle = find_nodes_with_cycle(graph);

    let mut subgraph: HashMap<String, Vec<String>> = HashMap::new();
    for (node, neighbors) in graph {
        if nodes_with_cycle.contains(node) {
            let filtered: Vec<String> = neighbors
                .iter()
                .filter(|n| nodes_with_cycle.contains(&**n))
                .cloned()
                .collect();
            subgraph.insert(node.clone(), filtered);
        }
    }

    let nodes: Vec<&String> = subgraph.keys().collect();
    let mut node_idx: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();
    let mut g = DiGraph::<(), ()>::new();

    for node in &nodes {
        let idx = g.add_node(());
        node_idx.insert(node.as_str(), idx);
    }

    for (u, neighbors) in &subgraph {
        if let Some(&u_idx) = node_idx.get(u.as_str()) {
            for v in neighbors {
                if let Some(&v_idx) = node_idx.get(v.as_str()) {
                    g.add_edge(u_idx, v_idx, ());
                }
            }
        }
    }

    let sccs = kosaraju_scc(&g);
    let mut result = Vec::new();
    for scc in &sccs {
        if scc.len() > 1 || (scc.len() == 1 && g.neighbors(scc[0]).any(|n| n == scc[0])) {
            result.push(nodes[scc[0].index()].clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_nodes_with_cycle_no_cycle() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);

        let result = find_nodes_with_cycle(&graph);
        assert!(result.is_empty());
    }

    #[test]
    fn test_find_nodes_with_cycle_simple() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["a".to_string()]);

        let result = find_nodes_with_cycle(&graph);
        assert!(result.contains("a"));
        assert!(result.contains("b"));
    }

    #[test]
    fn test_find_nodes_with_cycle_self_loop() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["a".to_string()]);

        let result = find_nodes_with_cycle(&graph);
        assert!(result.contains("a"));
    }

    #[test]
    fn test_select_cycle_checks() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["a".to_string()]);

        let result = select_cycle_checks(&graph);
        assert!(!result.is_empty());
    }
}