pub mod known_value_prop;
pub mod assignment_elision;

use std::collections::HashSet;

use crate::scratch::Project;
use crate::target::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Optimization {
    KnownValuePropagation,
    AssignmentElision,
}

impl Optimization {
    pub fn all() -> Vec<Optimization> {
        vec![
            Optimization::KnownValuePropagation,
            Optimization::AssignmentElision,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Optimization::KnownValuePropagation => "known_value_propagation",
            Optimization::AssignmentElision => "assignment_elision",
        }
    }
}

#[derive(Debug, Clone)]
pub struct OptimizationInfo {
    pub optimization: Optimization,
    pub enabled: bool,
}

#[derive(Debug)]
pub struct OptimizerException(pub String);

impl std::fmt::Display for OptimizerException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OptimizerException: {}", self.0)
    }
}

impl std::error::Error for OptimizerException {}

#[derive(Debug, Clone)]
pub struct BlockListInfo {
    pub times_used: std::collections::HashMap<String, f64>,
}

pub fn optimize(
    proj: &Project,
    targets: &[Target],
    max_iterations: usize,
    dont_remove: Option<HashSet<String>>,
    passes: &HashSet<Optimization>,
) -> Project {
    let mut proj = proj.clone();
    if passes.is_empty() {
        return proj;
    }

    let mut iterations = 0;

    loop {
        let mut any_changed = false;

        if passes.contains(&Optimization::KnownValuePropagation) {
            let (new_proj, changed) = known_value_prop::known_value_propagation(&proj, None);
            if changed {
                any_changed = true;
                proj = new_proj;
            }
        }

        if passes.contains(&Optimization::AssignmentElision) {
            let (new_proj, changed) = assignment_elision::assignment_elision(&proj, targets, dont_remove.as_ref());
            if changed {
                any_changed = true;
                proj = new_proj;
            }
        }

        iterations += 1;
        if !any_changed || iterations >= max_iterations {
            break;
        }
    }

    proj
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimization_all() {
        let all = Optimization::all();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&Optimization::KnownValuePropagation));
        assert!(all.contains(&Optimization::AssignmentElision));
    }

    #[test]
    fn test_optimization_name() {
        assert_eq!(Optimization::KnownValuePropagation.name(), "known_value_propagation");
        assert_eq!(Optimization::AssignmentElision.name(), "assignment_elision");
    }

    #[test]
    fn test_optimization_info() {
        let info = OptimizationInfo {
            optimization: Optimization::KnownValuePropagation,
            enabled: true,
        };
        assert!(info.enabled);
        assert_eq!(info.optimization.name(), "known_value_propagation");
    }
}