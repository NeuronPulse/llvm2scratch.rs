use std::collections::{HashMap, HashSet};

use petgraph::algo::dominators::simple_fast;
use petgraph::graph::DiGraph;

pub fn unavoidable_nodes(
    graph: &HashMap<String, Vec<String>>,
    source: &str,
    target: &str,
) -> HashSet<String> {
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

    let source_idx = match node_idx.get(source) {
        Some(&idx) => idx,
        None => return HashSet::new(),
    };

    let target_idx = match node_idx.get(target) {
        Some(&idx) => idx,
        None => return HashSet::new(),
    };

    let doms = simple_fast(&g, source_idx);

    let target_doms: HashSet<petgraph::graph::NodeIndex> = match doms.dominators(target_idx) {
        Some(iter) => iter.collect(),
        None => return HashSet::new(),
    };

    let mut result = HashSet::new();
    for dom_idx in &target_doms {
        if let Some(&node) = nodes.get(dom_idx.index()) {
            result.insert(node.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unavoidable_nodes_linear() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string()]);
        graph.insert("b".to_string(), vec!["c".to_string()]);
        graph.insert("c".to_string(), vec![]);

        let result = unavoidable_nodes(&graph, "a", "c");
        assert!(result.contains("a"));
        assert!(result.contains("b"));
        assert!(result.contains("c"));
    }

    #[test]
    fn test_unavoidable_nodes_branch() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec!["b".to_string(), "c".to_string()]);
        graph.insert("b".to_string(), vec!["d".to_string()]);
        graph.insert("c".to_string(), vec!["d".to_string()]);
        graph.insert("d".to_string(), vec![]);

        let result = unavoidable_nodes(&graph, "a", "d");
        assert!(result.contains("a"));
        assert!(result.contains("d"));
        assert!(!result.contains("b"));
        assert!(!result.contains("c"));
    }

    #[test]
    fn test_unavoidable_nodes_no_path() {
        let mut graph = HashMap::new();
        graph.insert("a".to_string(), vec![]);
        graph.insert("b".to_string(), vec![]);

        let result = unavoidable_nodes(&graph, "a", "b");
        assert!(result.is_empty());
    }
}