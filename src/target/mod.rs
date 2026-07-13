pub mod loader;

use serde::Deserialize;
use std::fmt;

pub const DEFAULT_TARGETS: &[&str] = &["scratch3", "turbowarp3"];
pub const DEFAULT_OPT_TARGET: &str = "turbowarp3";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BranchMethod {
    ProcCall,
    JumpTable,
}

impl fmt::Display for BranchMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BranchMethod::ProcCall => write!(f, "proc-call"),
            BranchMethod::JumpTable => write!(f, "jump-table"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Target {
    #[serde(skip)]
    pub id: String,
    pub info: TargetInfo,
    pub exec: TargetExec,
    pub perf: TargetPerf,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Target({})", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TargetInfo {
    pub name: String,
    pub url: String,
    pub desc: String,
    pub formats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TargetExec {
    pub preferred_branch_method: BranchMethod,
    pub compiler_type_hints: bool,
    pub max_branch_recursion: u32,
    pub preferred_branch_recursion: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TargetPerf {
    pub cost_num: f64,
    pub cost_name: f64,
    pub counter: f64,
    pub answer: f64,
    pub add: f64,
    pub sub: f64,
    pub mul: f64,
    pub div: f64,
    pub rand: f64,
    pub gt: f64,
    pub lt: f64,
    pub eq: f64,
    pub and_: f64,
    pub or_: f64,
    pub not_: f64,
    pub join: f64,
    pub letter_of: f64,
    pub length_of_str: f64,
    pub contains_str: f64,
    pub r#mod: f64,
    pub round: f64,
    pub abs: f64,
    pub floor: f64,
    pub ceil: f64,
    pub sqrt: f64,
    pub sin: f64,
    pub cos: f64,
    pub tan: f64,
    pub asin: f64,
    pub acos: f64,
    pub atan: f64,
    pub ln: f64,
    pub log: f64,
    pub exp: f64,
    pub pow10: f64,
    pub get_var: f64,
    pub set_var: f64,
    pub get_list: f64,
    pub at_index: f64,
    pub index_of: f64,
    pub length_of_list: f64,
    pub param: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_targets() {
        assert_eq!(DEFAULT_TARGETS, &["scratch3", "turbowarp3"]);
        assert_eq!(DEFAULT_OPT_TARGET, "turbowarp3");
    }

    #[test]
    fn test_branch_method_display() {
        assert_eq!(format!("{}", BranchMethod::ProcCall), "proc-call");
        assert_eq!(format!("{}", BranchMethod::JumpTable), "jump-table");
    }

    #[test]
    fn test_target_display() {
        let target = Target {
            id: "scratch3".to_string(),
            info: TargetInfo {
                name: "Scratch 3.0".to_string(),
                url: "https://scratch.mit.edu".to_string(),
                desc: "test".to_string(),
                formats: vec!["project3".to_string()],
            },
            exec: TargetExec {
                preferred_branch_method: BranchMethod::ProcCall,
                compiler_type_hints: false,
                max_branch_recursion: 3_000_000,
                preferred_branch_recursion: 300_000,
            },
            perf: TargetPerf {
                cost_num: 0.1,
                cost_name: 0.1,
                counter: 0.2,
                answer: 0.4,
                add: 1.0,
                sub: 1.0,
                mul: 1.0,
                div: 1.0,
                rand: 1.3,
                gt: 1.0,
                lt: 1.0,
                eq: 1.0,
                and_: 0.5,
                or_: 0.5,
                not_: 0.4,
                join: 0.7,
                letter_of: 0.6,
                length_of_str: 0.5,
                contains_str: 1.1,
                r#mod: 1.4,
                round: 0.5,
                abs: 0.8,
                floor: 0.7,
                ceil: 0.8,
                sqrt: 0.8,
                sin: 2.5,
                cos: 2.8,
                tan: 2.9,
                asin: 0.8,
                acos: 0.9,
                atan: 1.1,
                ln: 1.2,
                log: 1.0,
                exp: 1.1,
                pow10: 1.1,
                get_var: 0.6,
                set_var: 3.4,
                get_list: 10.0,
                at_index: 2.8,
                index_of: 130.8,
                length_of_list: 2.0,
                param: 0.6,
            },
        };
        assert_eq!(format!("{}", target), "Target(scratch3)");
    }
}