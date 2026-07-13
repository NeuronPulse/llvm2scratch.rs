pub mod cycle;
pub mod dominator;

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub depends: HashSet<String>,
    pub modifies: HashSet<String>,
    pub calls: HashSet<String>,
    pub direct_modifies: HashSet<String>,
    pub direct_calls: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct CallGraphAnalysis {
    pub entrypoint: String,
    pub info: HashMap<String, NodeInfo>,
    pub analyzed: HashSet<String>,
}

impl CallGraphAnalysis {
    pub fn new(entrypoint: String, info: HashMap<String, NodeInfo>) -> Self {
        CallGraphAnalysis {
            entrypoint,
            info,
            analyzed: HashSet::new(),
        }
    }

    pub fn analyze_node(&mut self, name: &str) -> bool {
        let mut changed = false;
        let info = self.info.get(name).cloned();
        if let Some(mut node_info) = info {
            let direct_calls = node_info.direct_calls.clone();
            for callee in &direct_calls {
                if node_info.calls.insert(callee.clone()) {
                    changed = true;
                }
                if let Some(callee_info) = self.info.get(callee) {
                    for dep in &callee_info.depends {
                        if node_info.depends.insert(dep.clone()) {
                            changed = true;
                        }
                    }
                    for mod_ in &callee_info.modifies {
                        if node_info.modifies.insert(mod_.clone()) {
                            changed = true;
                        }
                    }
                    for call in &callee_info.calls {
                        if node_info.calls.insert(call.clone()) {
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                self.info.insert(name.to_string(), node_info);
            }
        }
        if self.analyzed.insert(name.to_string()) {
            true
        } else {
            changed
        }
    }

    pub fn analyze(&mut self) {
        loop {
            let mut any_changed = false;
            let names: Vec<String> = self.info.keys().cloned().collect();
            for name in &names {
                if self.analyze_node(name) {
                    any_changed = true;
                }
            }
            if !any_changed {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_graph_analysis() {
        let mut info = HashMap::new();
        info.insert("main".to_string(), NodeInfo {
            depends: HashSet::new(),
            modifies: HashSet::new(),
            calls: HashSet::new(),
            direct_modifies: HashSet::new(),
            direct_calls: HashSet::from(["foo".to_string()]),
        });
        info.insert("foo".to_string(), NodeInfo {
            depends: HashSet::from(["x".to_string()]),
            modifies: HashSet::from(["y".to_string()]),
            calls: HashSet::new(),
            direct_modifies: HashSet::from(["y".to_string()]),
            direct_calls: HashSet::new(),
        });

        let mut cga = CallGraphAnalysis::new("main".to_string(), info);
        cga.analyze();

        let main_info = cga.info.get("main").unwrap();
        assert!(main_info.depends.contains("x"));
        assert!(main_info.modifies.contains("y"));
        assert!(main_info.calls.contains("foo"));
    }

    #[test]
    fn test_node_info_default() {
        let info = NodeInfo {
            depends: HashSet::new(),
            modifies: HashSet::new(),
            calls: HashSet::new(),
            direct_modifies: HashSet::new(),
            direct_calls: HashSet::new(),
        };
        assert!(info.depends.is_empty());
        assert!(info.modifies.is_empty());
    }
}