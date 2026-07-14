use super::llvm_in_rust;

pub type DecodedModule = crate::ir::instructions::Module;

pub fn parse_assembly(llvm_ir: &str, verify_ir: bool) -> Result<crate::ir::instructions::Module, String> {
    let (ctx, module) = llvm_ir_parser::parser::parse(llvm_ir)
        .map_err(|e| format!("Parse error at line {} col {}: {}", e.line, e.col, e.message))?;

    let _ = verify_ir;

    llvm_in_rust::convert_module(&module, &ctx)
}
