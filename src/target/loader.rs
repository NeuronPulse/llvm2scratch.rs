use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::de::Error as SerdeDeError;

use super::{Target, TargetPerf, TargetExec, TargetInfo, BranchMethod};

static TARGET_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let exe_dir = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let candidate = exe_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join("data")
        .join("targets");
    if candidate.is_dir() {
        return candidate;
    }
    PathBuf::from("data/targets")
});

static TARGET_LIST_CACHE: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut res = Vec::new();
    if let Ok(entries) = fs::read_dir(&*TARGET_DIR) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(name) = path.file_stem()
            {
                res.push(name.to_string_lossy().to_string());
            }
        }
    }
    res.sort();
    res
});

static TARGET_CACHE: LazyLock<std::sync::Mutex<HashMap<String, Target>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub fn list_targets() -> Vec<String> {
    TARGET_LIST_CACHE.clone()
}

pub fn get_target(name: &str) -> Result<Target, TargetLoadError> {
    {
        let cache = TARGET_CACHE.lock().unwrap();
        if let Some(t) = cache.get(name) {
            return Ok(t.clone());
        }
    }

    let toml_path = TARGET_DIR.join(format!("{}.toml", name));
    let raw = fs::read_to_string(&toml_path).map_err(|e| TargetLoadError::Io {
        name: name.to_string(),
        source: e,
    })?;

    let target = parse_target_toml(name, &raw)?;
    {
        let mut cache = TARGET_CACHE.lock().unwrap();
        cache.insert(name.to_string(), target.clone());
    }
    Ok(target)
}

#[derive(Debug, thiserror::Error)]
pub enum TargetLoadError {
    #[error("IO error loading target '{name}': {source}")]
    Io {
        name: String,
        #[source]
        source: std::io::Error,
    },
    #[error("TOML parse error for target '{name}': {source}")]
    Toml {
        name: String,
        #[source]
        source: toml::de::Error,
    },
}

fn parse_target_toml(name: &str, raw: &str) -> Result<Target, TargetLoadError> {
    let data: toml::Table = toml::from_str(raw).map_err(|e| TargetLoadError::Toml {
        name: name.to_string(),
        source: e,
    })?;

    let info = parse_target_info(name, &data)?;
    let exec = parse_target_exec(name, &data)?;
    let perf = parse_target_perf(name, &data)?;

    Ok(Target {
        id: name.to_string(),
        info,
        exec,
        perf,
    })
}

fn parse_target_info(name: &str, data: &toml::Table) -> Result<TargetInfo, TargetLoadError> {
    let info_table = data
        .get("info")
        .and_then(|v| v.as_table())
        .ok_or_else(|| TargetLoadError::Toml {
            name: name.to_string(),
            source: <toml::de::Error as SerdeDeError>::custom("missing [info] section"),
        })?;

    Ok(TargetInfo {
        name: get_string(info_table, "name"),
        url: get_string(info_table, "url"),
        desc: get_string(info_table, "desc"),
        formats: get_string_array(info_table, "formats"),
    })
}

fn parse_target_exec(name: &str, data: &toml::Table) -> Result<TargetExec, TargetLoadError> {
    let exec_table = data
        .get("exec")
        .and_then(|v| v.as_table())
        .ok_or_else(|| TargetLoadError::Toml {
            name: name.to_string(),
            source: <toml::de::Error as SerdeDeError>::custom("missing [exec] section"),
        })?;

    let branch_method_str = get_string(exec_table, "preferred-branch-method");
    let preferred_branch_method = match branch_method_str.as_str() {
        "proc-call" => BranchMethod::ProcCall,
        "jump-table" => BranchMethod::JumpTable,
        _ => BranchMethod::ProcCall,
    };

    Ok(TargetExec {
        preferred_branch_method,
        compiler_type_hints: get_bool(exec_table, "compiler-type-hints"),
        max_branch_recursion: get_u32(exec_table, "max-branch-recursion"),
        preferred_branch_recursion: get_u32(exec_table, "preferred-branch-recursion"),
    })
}

fn parse_target_perf(name: &str, data: &toml::Table) -> Result<TargetPerf, TargetLoadError> {
    let perf_table = data
        .get("perf")
        .and_then(|v| v.as_table())
        .ok_or_else(|| TargetLoadError::Toml {
            name: name.to_string(),
            source: <toml::de::Error as SerdeDeError>::custom("missing [perf] section"),
        })?;

    Ok(TargetPerf {
        cost_num: get_f64(perf_table, "cost-num"),
        cost_name: get_f64(perf_table, "cost-name"),
        counter: get_f64(perf_table, "counter"),
        answer: get_f64(perf_table, "answer"),
        add: get_f64(perf_table, "add"),
        sub: get_f64(perf_table, "sub"),
        mul: get_f64(perf_table, "mul"),
        div: get_f64(perf_table, "div"),
        rand: get_f64(perf_table, "rand"),
        gt: get_f64(perf_table, "gt"),
        lt: get_f64(perf_table, "lt"),
        eq: get_f64(perf_table, "eq"),
        and_: get_f64(perf_table, "and"),
        or_: get_f64(perf_table, "or"),
        not_: get_f64(perf_table, "not"),
        join: get_f64(perf_table, "join"),
        letter_of: get_f64(perf_table, "letter-of"),
        length_of_str: get_f64(perf_table, "length-of-str"),
        contains_str: get_f64(perf_table, "contains-str"),
        r#mod: get_f64(perf_table, "mod"),
        round: get_f64(perf_table, "round"),
        abs: get_f64(perf_table, "abs"),
        floor: get_f64(perf_table, "floor"),
        ceil: get_f64(perf_table, "ceil"),
        sqrt: get_f64(perf_table, "sqrt"),
        sin: get_f64(perf_table, "sin"),
        cos: get_f64(perf_table, "cos"),
        tan: get_f64(perf_table, "tan"),
        asin: get_f64(perf_table, "asin"),
        acos: get_f64(perf_table, "acos"),
        atan: get_f64(perf_table, "atan"),
        ln: get_f64(perf_table, "ln"),
        log: get_f64(perf_table, "log"),
        exp: get_f64(perf_table, "exp"),
        pow10: get_f64(perf_table, "pow10"),
        get_var: get_f64(perf_table, "get-var"),
        set_var: get_f64(perf_table, "set-var"),
        get_list: get_f64(perf_table, "get-list"),
        at_index: get_f64(perf_table, "at-index"),
        index_of: get_f64(perf_table, "index-of"),
        length_of_list: get_f64(perf_table, "length-of-list"),
        param: get_f64(perf_table, "param"),
    })
}

fn get_string(table: &toml::Table, key: &str) -> String {
    table
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn get_f64(table: &toml::Table, key: &str) -> f64 {
    table
        .get(key)
        .map(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|i| i as f64))
                .unwrap_or(0.0)
        })
        .unwrap_or(0.0)
}

fn get_bool(table: &toml::Table, key: &str) -> bool {
    table.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn get_u32(table: &toml::Table, key: &str) -> u32 {
    table
        .get(key)
        .and_then(|v| v.as_integer())
        .unwrap_or(0) as u32
}

fn get_string_array(table: &toml::Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_scratch3_toml() {
        let raw = include_str!("../../data/targets/scratch3.toml");
        let target = parse_target_toml("scratch3", raw).unwrap();
        assert_eq!(target.id, "scratch3");
        assert_eq!(target.info.name, "Scratch 3.0");
        assert_eq!(target.exec.preferred_branch_method, BranchMethod::ProcCall);
        assert!(!target.exec.compiler_type_hints);
        assert_eq!(target.exec.max_branch_recursion, 3_000_000);
        assert_eq!(target.exec.preferred_branch_recursion, 300_000);
        assert!((target.perf.add - 1.0).abs() < f64::EPSILON);
        assert!((target.perf.set_var - 3.4).abs() < f64::EPSILON);
        assert!((target.perf.r#mod - 1.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_turbowarp3_toml() {
        let raw = include_str!("../../data/targets/turbowarp3.toml");
        let target = parse_target_toml("turbowarp3", raw).unwrap();
        assert_eq!(target.id, "turbowarp3");
        assert_eq!(target.info.name, "TurboWarp");
        assert_eq!(
            target.exec.preferred_branch_method,
            BranchMethod::JumpTable
        );
        assert!(target.exec.compiler_type_hints);
        assert_eq!(target.exec.max_branch_recursion, 2000);
        assert!((target.perf.add - 46.4).abs() < 0.01);
        assert!((target.perf.get_list - 1203300.0).abs() < 1.0);
    }

    #[test]
    fn test_list_targets() {
        let targets = list_targets();
        assert!(targets.contains(&"scratch3".to_string()));
        assert!(targets.contains(&"turbowarp3".to_string()));
    }
}