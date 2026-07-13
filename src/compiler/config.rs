use std::collections::{HashMap, HashSet};

use crate::ir;
use crate::optimizer::Optimization;
use crate::scratch::{BlockList, ScratchConfig};
use crate::target::Target;

pub const INTERMEDIATE_MAX_BITS: usize = 53;
pub const VARIABLE_MAX_BITS: usize = 48;
pub const SCRATCH_LIST_LIMIT: usize = 200_000;
pub const PTR_WIDTH_BITS: usize = 32;
pub const BINOP_LOOKUP_BITS: usize = 8;
pub const EXIT_CALL_ID: usize = 0;
pub const ENTRY_CALL_ID: usize = 1;
pub const START_STACK_RESET_ID: usize = 2;

#[derive(Debug, Clone)]
pub struct CompilerConfig {
    pub targets: Vec<Target>,
    pub opt_target: Target,
    pub opt_passes: HashSet<Optimization>,
    pub compiler_opt: bool,
    pub compiler_minify: bool,
    pub memory_size: usize,
    pub local_stack_size: usize,
    pub use_branch_jump_table: bool,
    pub max_branch_recursion: usize,
    pub accurate_byte_spacing: bool,
    pub entrypoint: String,
    pub gen_lut_runtime: bool,
    pub scratch_config: ScratchConfig,
    pub no_warn_missing_fn_sig: HashSet<String>,

    pub return_var: String,
    pub mem_var: String,
    pub init_mem_var: String,
    pub stack_pointer_var: String,
    pub heap_pointer_var: String,
    pub local_stack_var: String,
    pub local_stack_size_var: String,
    pub jump_table_id_var: String,
    pub debug_branch_log_var: String,

    pub ascii_lookup_var: String,
    pub pow2_lookup_var: String,
    pub lowercase_var: String,

    pub return_address_local: String,
    pub vararg_ptr_local: String,
    pub previous_stack_size_local: String,
    pub branch_jump_table_addr_local: String,
    pub special_locals: HashSet<String>,

    pub func_ptr_parameter: String,
    pub tmp_prefix: String,
    pub zero_indexed_suffix: String,
    pub one_indexed_suffix: String,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        let return_address_local = "return address".to_string();
        let vararg_ptr_local = "vararg ptr".to_string();
        let previous_stack_size_local = "prev stack size".to_string();
        let branch_jump_table_addr_local = "branch jump table addr".to_string();

        let special_locals = HashSet::from([
            return_address_local.clone(),
            vararg_ptr_local.clone(),
            previous_stack_size_local.clone(),
            branch_jump_table_addr_local.clone(),
        ]);

        CompilerConfig {
            targets: crate::target::DEFAULT_TARGETS
                .iter()
                .filter_map(|t| crate::target::loader::get_target(t).ok())
                .collect(),
            opt_target: crate::target::loader::get_target(crate::target::DEFAULT_OPT_TARGET)
                .unwrap_or_else(|_| {
                    crate::target::DEFAULT_TARGETS
                        .iter()
                        .filter_map(|t| crate::target::loader::get_target(t).ok())
                        .next()
                        .expect("at least one target must be available")
                }),
            opt_passes: Optimization::all().iter().copied().collect(),
            compiler_opt: true,
            compiler_minify: true,
            memory_size: 4096,
            local_stack_size: 512,
            use_branch_jump_table: false,
            max_branch_recursion: 2000,
            accurate_byte_spacing: true,
            entrypoint: "main".to_string(),
            gen_lut_runtime: false,
            scratch_config: ScratchConfig::default(),
            no_warn_missing_fn_sig: HashSet::from(["exit".to_string(), "__call_exitprocs".to_string()]),

            return_var: "!return value".to_string(),
            mem_var: "!mem".to_string(),
            init_mem_var: "!mem init".to_string(),
            stack_pointer_var: "!stack pointer".to_string(),
            heap_pointer_var: "!heap pointer".to_string(),
            local_stack_var: "!local stack".to_string(),
            local_stack_size_var: "!local stack size".to_string(),
            jump_table_id_var: "!call stack reset id".to_string(),
            debug_branch_log_var: "!!debug_branch_log".to_string(),

            ascii_lookup_var: "!ASCII lookup".to_string(),
            pow2_lookup_var: "!POW2 lookup".to_string(),
            lowercase_var: "!lowercase".to_string(),

            return_address_local,
            vararg_ptr_local,
            previous_stack_size_local,
            branch_jump_table_addr_local,
            special_locals,

            func_ptr_parameter: "func ptr addr".to_string(),
            tmp_prefix: "%!tmp:".to_string(),
            zero_indexed_suffix: " (0 indexed)".to_string(),
            one_indexed_suffix: " (1 indexed)".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncPtrSigInfo {
    pub signature_id: usize,
    pub can_call: HashSet<String>,
    pub value_param_count: usize,
    pub is_variadic: bool,
    pub return_addresses: Vec<String>,
    pub returns_to_address: bool,
    pub takes_return_address: bool,
    pub could_recurse: bool,
}

pub trait ReturnAddrInfo {
    fn return_addresses(&self) -> &[String];
    fn takes_return_address(&self) -> bool;
}

impl ReturnAddrInfo for FuncPtrSigInfo {
    fn return_addresses(&self) -> &[String] {
        &self.return_addresses
    }
    fn takes_return_address(&self) -> bool {
        self.takes_return_address
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncInfo {
    pub name: String,
    pub fn_id: usize,
    pub params: Vec<Variable>,
    pub param_sizes: Vec<usize>,
    pub value_param_count: usize,
    pub is_variadic: bool,
    pub can_call: HashSet<String>,
    pub return_addresses: Vec<String>,
    pub returns_to_address: bool,
    pub takes_return_address: bool,
    pub checked_blocks: Vec<String>,
    pub block_alloca_size: HashMap<String, usize>,
    pub total_alloca_size: Option<usize>,
    pub skip_stack_size_change: bool,
    pub block_var_use: HashMap<String, BlockVarUse>,
    pub branches_to_first: bool,
    pub phi_info: HashMap<String, HashMap<String, Vec<(Variable, ir::Value)>>>,
}

impl FuncInfo {
    pub fn new(name: String, fn_id: usize, params: Vec<Variable>, param_sizes: Vec<usize>, value_param_count: usize) -> Self {
        FuncInfo {
            name,
            fn_id,
            params,
            param_sizes,
            value_param_count,
            is_variadic: false,
            can_call: HashSet::new(),
            return_addresses: Vec::new(),
            returns_to_address: false,
            takes_return_address: false,
            checked_blocks: Vec::new(),
            block_alloca_size: HashMap::new(),
            total_alloca_size: Some(0),
            skip_stack_size_change: false,
            block_var_use: HashMap::new(),
            branches_to_first: false,
            phi_info: HashMap::new(),
        }
    }
}

impl ReturnAddrInfo for FuncInfo {
    fn return_addresses(&self) -> &[String] {
        &self.return_addresses
    }
    fn takes_return_address(&self) -> bool {
        self.takes_return_address
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub fn_info: FuncInfo,
    pub available_params: Vec<Variable>,
    pub available_param_sizes: Vec<usize>,
    pub code: BlockList,
    pub label: Option<String>,
    pub allocated: usize,
    pub next_call_id: usize,
}

impl BlockInfo {
    pub fn new(fn_info: FuncInfo) -> Self {
        let available_params = fn_info.params.clone();
        let available_param_sizes = fn_info.param_sizes.clone();
        BlockInfo {
            fn_info,
            available_params,
            available_param_sizes,
            code: BlockList::new(),
            label: None,
            allocated: 0,
            next_call_id: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockVarUse {
    pub depends: HashSet<String>,
    pub modifies: HashSet<String>,
    pub branches: HashSet<String>,
    pub depends_var_sizes: HashMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Global,
    Param,
    Var,
    SpecialVar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub var_name: String,
    pub var_type: VarType,
    pub fn_name: Option<String>,
}

impl Variable {
    pub fn get_unidxed_raw_var_name(&self) -> String {
        match self.var_type {
            VarType::Global => format!("@{}", self.var_name),
            VarType::Param => localize_param(&self.var_name),
            VarType::Var => {
                if let Some(fn_) = &self.fn_name {
                    format!("%{}:{}", fn_, self.var_name)
                } else {
                    self.var_name.clone()
                }
            }
            VarType::SpecialVar => self.var_name.clone(),
        }
    }

    pub fn get_raw_var_name(&self, index: Option<usize>) -> String {
        let unidxed = self.get_unidxed_raw_var_name();
        match index {
            None => unidxed,
            Some(i) => format!("{}:{}", unidxed, i),
        }
    }
}

pub fn localize_param(name: &str) -> String {
    format!("%{}", name)
}

#[derive(Debug, Clone, PartialEq)]
pub struct IdxbleValue {
    pub vals: Vec<crate::scratch::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompException(pub String);

impl std::fmt::Display for CompException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Compiler exception: {}", self.0)
    }
}

impl std::error::Error for CompException {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compiler_config_default() {
        let cfg = CompilerConfig::default();
        assert_eq!(cfg.memory_size, 4096);
        assert_eq!(cfg.local_stack_size, 512);
        assert_eq!(cfg.entrypoint, "main");
        assert!(cfg.compiler_opt);
        assert!(cfg.compiler_minify);
        assert!(cfg.accurate_byte_spacing);
    }

    #[test]
    fn test_variable_global() {
        let var = Variable { var_name: "g".to_string(), var_type: VarType::Global, fn_name: None };
        assert_eq!(var.get_unidxed_raw_var_name(), "@g");
        assert_eq!(var.get_raw_var_name(None), "@g");
        assert_eq!(var.get_raw_var_name(Some(0)), "@g:0");
    }

    #[test]
    fn test_variable_param() {
        let var = Variable { var_name: "p".to_string(), var_type: VarType::Param, fn_name: None };
        assert_eq!(var.get_unidxed_raw_var_name(), "%p");
    }

    #[test]
    fn test_variable_local() {
        let var = Variable { var_name: "x".to_string(), var_type: VarType::Var, fn_name: Some("foo".to_string()) };
        assert_eq!(var.get_unidxed_raw_var_name(), "%foo:x");
    }

    #[test]
    fn test_variable_special() {
        let var = Variable { var_name: "!mem".to_string(), var_type: VarType::SpecialVar, fn_name: None };
        assert_eq!(var.get_unidxed_raw_var_name(), "!mem");
    }

    #[test]
    fn test_func_info_new() {
        let fi = FuncInfo::new("main".to_string(), 0, vec![], vec![], 0);
        assert_eq!(fi.name, "main");
        assert_eq!(fi.fn_id, 0);
        assert!(!fi.is_variadic);
        assert_eq!(fi.total_alloca_size, Some(0));
    }

    #[test]
    fn test_block_info_new() {
        let fi = FuncInfo::new("test".to_string(), 1, vec![], vec![], 0);
        let bi = BlockInfo::new(fi);
        assert!(bi.label.is_none());
        assert_eq!(bi.allocated, 0);
        assert_eq!(bi.next_call_id, 0);
    }

    #[test]
    fn test_localize_param() {
        assert_eq!(localize_param("x"), "%x");
    }

    #[test]
    fn test_block_var_use_default() {
        let bvu = BlockVarUse::default();
        assert!(bvu.depends.is_empty());
        assert!(bvu.modifies.is_empty());
        assert!(bvu.branches.is_empty());
    }
}