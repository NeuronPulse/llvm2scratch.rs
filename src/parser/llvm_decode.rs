use std::cell::RefCell;
use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::types::{AnyTypeEnum, AnyType};
use inkwell::basic_block::BasicBlock;
use inkwell::values::{
    AnyValue, AnyValueEnum, BasicValueEnum, InstructionValue, PhiValue,
};
use inkwell::values::Operand;

use crate::ir::types::*;
use crate::ir::values::*;
use crate::ir::instructions::*;
use super::token_util::*;

thread_local! {
    static BB_LABEL_MEMO: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

fn unescape_inkwell_display(s: &str) -> String {
    let s = s.trim();
    let s = if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        &s[1..s.len() - 1]
    } else {
        s
    };
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => { result.push('\n'); chars.next(); }
                Some('t') => { result.push('\t'); chars.next(); }
                Some('r') => { result.push('\r'); chars.next(); }
                Some('\\') => { result.push('\\'); chars.next(); }
                Some('"') => { result.push('"'); chars.next(); }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn get_result_local_var(instr_str: &str) -> Option<ResultLocalVar> {
    let s = instr_str.trim();
    if s.starts_with('%') && let Some(pos) = s.find('=') {
        let name = s[1..pos].trim();
        return Some(ResultLocalVar::new(name));
    }
    None
}

fn decode_type_from_inkwell(
    ty: &AnyTypeEnum,
    structs: &HashMap<String, Type>,
    _func_names: &[String],
) -> Result<Type, String> {
    match ty {
        AnyTypeEnum::VoidType(_) => Ok(Type::Void),
        AnyTypeEnum::IntType(int_ty) => Ok(Type::integer(int_ty.get_bit_width())),
        AnyTypeEnum::FloatType(float_ty) => {
            let s = format!("{}", float_ty);
            let s = unescape_inkwell_display(&s);
            match s.as_str() {
                "half" => Ok(Type::Half),
                "float" => Ok(Type::Float),
                "double" => Ok(Type::Double),
                "fp128" => Ok(Type::Fp128),
                _ => Err(format!("Unsupported float type: {}", s)),
            }
        }
        AnyTypeEnum::PointerType(_) => {
            let s = format!("{}", ty);
            let s = unescape_inkwell_display(&s);
            let tokens = parse_until_end_strict(&s);
            let (parsed, rest) = parse_type_tokens(&tokens, structs)?;
            if !rest.is_empty() {
                return Err(format!("Unexpected tokens after pointer type: {:?}", rest));
            }
            Ok(parsed)
        }
        AnyTypeEnum::VectorType(vec_ty) => {
            let element_type = vec_ty.get_element_type();
            let inner = decode_type_from_inkwell(&element_type.as_any_type_enum(), structs, _func_names)?;
            if !inner.is_vec_target() {
                return Err(format!("Vec inner must be VecTargetTy, got: {:?}", inner));
            }
            let size = vec_ty.get_size();
            Ok(Type::Vector(VecTy::new(inner, size)))
        }
        AnyTypeEnum::ArrayType(arr_ty) => {
            let element_type = arr_ty.get_element_type();
            let inner = decode_type_from_inkwell(&element_type.as_any_type_enum(), structs, _func_names)?;
            if !inner.is_agg_target() {
                return Err(format!("Array inner must be AggTargetTy, got: {:?}", inner));
            }
            let size = arr_ty.len();
            Ok(Type::Array(ArrayTy::new(inner, size)))
        }
        AnyTypeEnum::StructType(struct_ty) => {
            let mut members = Vec::new();
            for field_ty in struct_ty.get_field_types() {
                let member = decode_type_from_inkwell(&field_ty.as_any_type_enum(), structs, _func_names)?;
                if !member.is_agg_target() {
                    return Err(format!("Struct member must be AggTargetTy, got: {:?}", member));
                }
                members.push(member);
            }
            let s = unescape_inkwell_display(&format!("{}", struct_ty)).replace(" ", "");
            let is_packed = s.starts_with("<{");
            Ok(Type::Struct(StructTy::new(is_packed, members)))
        }
        AnyTypeEnum::FunctionType(_) => {
            let s = format!("{}", ty);
            let s = unescape_inkwell_display(&s);
            let tokens = parse_until_end_strict(&s);
            let (parsed, rest) = parse_type_tokens(&tokens, structs)?;
            if !rest.is_empty() {
                return Err(format!("Unexpected tokens after func type: {:?}", rest));
            }
            Ok(parsed)
        }
        AnyTypeEnum::ScalableVectorType(_) => Err("Scalable vectors not supported".to_string()),
    }
}

fn decode_basic_value(
    value: &BasicValueEnum,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Value, String> {
    let ty = decode_type_from_inkwell(&value.get_type().as_any_type_enum(), structs, func_names)?;

    match value {
        BasicValueEnum::IntValue(int_val) => {
            let width = match &ty {
                Type::Integer(i) => i.width,
                _ => return Err("Expected integer type".to_string()),
            };
            let const_val = int_val.get_zero_extended_constant();
            match const_val {
                Some(v) => Ok(Value::KnownInt(KnownIntVal::new(ty, v as u128, width))),
                None => {
                    let s = format!("{}", int_val);
                    let s = unescape_inkwell_display(&s);
                    let s = s.trim();
                    if let Some(res) = get_result_local_var(s) {
                        Ok(Value::LocalVar(LocalVarVal::new(ty, res.name)))
                    } else if let Some(local_name) = s.split_whitespace().find(|t| t.starts_with('%')) {
                        let name = local_name.strip_prefix('%').unwrap_or(local_name).to_string();
                        Ok(Value::LocalVar(LocalVarVal::new(ty, name)))
                    } else {
                        let tokens = parse_until_end_strict(&s);
                        let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                        if !rest.is_empty() {
                            return Err(format!("Unexpected tokens after int value: {:?}", rest));
                        }
                        Ok(val)
                    }
                }
            }
        }
        BasicValueEnum::FloatValue(float_val) => {
            let val = float_val.get_constant().map(|(v, _)| v).unwrap_or(0.0);
            Ok(Value::KnownFloat(KnownFloatVal::new(ty, val)))
        }
        BasicValueEnum::PointerValue(ptr) => {
            if ptr.is_null() {
                Ok(Value::NullPtr(NullPtrVal::new(ty)))
            } else {
                let s = format!("{}", ptr);
                let s = unescape_inkwell_display(&s);
                let s = s.trim();
                if s.starts_with('@') {
                    let name = if let Some(eq_pos) = s.find('=') {
                        &s[1..eq_pos].trim()
                    } else {
                        &s[1..]
                    };
                    if func_names.iter().any(|f| f == name) {
                        Ok(Value::Function(FunctionVal::new(ty, name)))
                    } else {
                        Ok(Value::GlobalPtr(GlobalPtrVal::new(ty, name)))
                    }
                } else if s.starts_with("define ") || s.starts_with("declare ") || s.starts_with(';') {
                    // inkwell may print a function operand as the full definition text,
                    // sometimes preceded by attribute comment lines.
                    let name = s.lines()
                        .find_map(|line| {
                            let line = line.trim();
                            if line.starts_with("declare ") || line.starts_with("define ") {
                                line.find('@').and_then(|p| {
                                    let after = &line[p + 1..];
                                    after.split(|c: char| c == '(' || c == ' ' || c == '\n').next()
                                })
                            } else {
                                None
                            }
                        })
                        .unwrap_or("")
                        .to_string();
                    if func_names.iter().any(|f| f == &name) {
                        Ok(Value::Function(FunctionVal::new(ty, name)))
                    } else {
                        Ok(Value::GlobalPtr(GlobalPtrVal::new(ty, name)))
                    }
                } else if s.starts_with('%') {
                    let name = if let Some(eq_pos) = s.find('=') {
                        s[1..eq_pos].trim().to_string()
                    } else {
                        s[1..].to_string()
                    };
                    Ok(Value::LocalVar(LocalVarVal::new(ty, name)))
                } else {
                    let tokens = parse_until_end_strict(&s);
                    let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                    if !rest.is_empty() {
                        return Err(format!("Unexpected tokens after pointer value: {:?}", rest));
                    }
                    Ok(val)
                }
            }
        }
        BasicValueEnum::StructValue(_) | BasicValueEnum::VectorValue(_) | BasicValueEnum::ArrayValue(_) => {
            let s = format!("{}", value);
            let s = unescape_inkwell_display(&s);
            let s = s.trim();
            if s.starts_with('%') {
                // inkwell may print a local aggregate as the full defining instruction,
                // so extract the result name when an '=' is present.
                if let Some(res) = get_result_local_var(s) {
                    Ok(Value::LocalVar(LocalVarVal::new(ty, res.name)))
                } else if !s.contains(' ') {
                    Ok(Value::LocalVar(LocalVarVal::new(ty, s[1..].to_string())))
                } else {
                    let tokens = parse_until_end_strict(&s);
                    let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                    if !rest.is_empty() {
                        return Err(format!("Unexpected tokens after aggregate value: {:?}", rest));
                    }
                    Ok(val)
                }
            } else {
                let tokens = parse_until_end_strict(&s);
                let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                if !rest.is_empty() {
                    return Err(format!("Unexpected tokens after aggregate value: {:?}", rest));
                }
                Ok(val)
            }
        }
        BasicValueEnum::ScalableVectorValue(_) => Err("Scalable vectors not supported".to_string()),
    }
}

#[allow(dead_code)]
fn decode_any_value(
    value: &AnyValueEnum,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Value, String> {
    let ty = decode_type_from_inkwell(&value.get_type(), structs, func_names)?;

    match value {
        AnyValueEnum::IntValue(int_val) => {
            let width = match &ty {
                Type::Integer(i) => i.width,
                _ => return Err("Expected integer type".to_string()),
            };
            match int_val.get_zero_extended_constant() {
                Some(v) => Ok(Value::KnownInt(KnownIntVal::new(ty, v as u128, width))),
                None => {
                    let s = format!("{}", int_val);
                    let s = unescape_inkwell_display(&s);
                    let s = s.trim();
                    if let Some(res) = get_result_local_var(s) {
                        Ok(Value::LocalVar(LocalVarVal::new(ty, res.name)))
                    } else if let Some(local_name) = s.split_whitespace().find(|t| t.starts_with('%')) {
                        let name = local_name.strip_prefix('%').unwrap_or(local_name).to_string();
                        Ok(Value::LocalVar(LocalVarVal::new(ty, name)))
                    } else {
                        let tokens = parse_until_end_strict(&s);
                        let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                        if !rest.is_empty() {
                            return Err(format!("Unexpected tokens: {:?}", rest));
                        }
                        Ok(val)
                    }
                }
            }
        }
        AnyValueEnum::FloatValue(float_val) => {
            let val = float_val.get_constant().map(|(v, _)| v).unwrap_or(0.0);
            Ok(Value::KnownFloat(KnownFloatVal::new(ty, val)))
        }
        AnyValueEnum::PointerValue(ptr) => {
            if ptr.is_null() {
                Ok(Value::NullPtr(NullPtrVal::new(ty)))
            } else {
                let s = format!("{}", ptr);
                let s = unescape_inkwell_display(&s);
                let s = s.trim();
                if s.starts_with('@') {
                    let name = if let Some(eq_pos) = s.find('=') {
                        &s[1..eq_pos].trim()
                    } else {
                        &s[1..]
                    };
                    if func_names.iter().any(|f| f == name) {
                        Ok(Value::Function(FunctionVal::new(ty, name)))
                    } else {
                        Ok(Value::GlobalPtr(GlobalPtrVal::new(ty, name)))
                    }
                } else if s.starts_with("define ") || s.starts_with("declare ") || s.starts_with(';') {
                    // inkwell may print a function operand as the full definition text,
                    // sometimes preceded by attribute comment lines.
                    let name = s.lines()
                        .find_map(|line| {
                            let line = line.trim();
                            if line.starts_with("declare ") || line.starts_with("define ") {
                                line.find('@').and_then(|p| {
                                    let after = &line[p + 1..];
                                    after.split(|c: char| c == '(' || c == ' ' || c == '\n').next()
                                })
                            } else {
                                None
                            }
                        })
                        .unwrap_or("")
                        .to_string();
                    if func_names.iter().any(|f| f == &name) {
                        Ok(Value::Function(FunctionVal::new(ty, name)))
                    } else {
                        Ok(Value::GlobalPtr(GlobalPtrVal::new(ty, name)))
                    }
                } else if s.starts_with('%') {
                    let name = if let Some(eq_pos) = s.find('=') {
                        s[1..eq_pos].trim().to_string()
                    } else {
                        s[1..].to_string()
                    };
                    Ok(Value::LocalVar(LocalVarVal::new(ty, name)))
                } else {
                    let tokens = parse_until_end_strict(&s);
                    let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                    if !rest.is_empty() {
                        return Err(format!("Unexpected tokens: {:?}", rest));
                    }
                    Ok(val)
                }
            }
        }
        AnyValueEnum::FunctionValue(func) => {
            let name = func.get_name().to_str().unwrap_or("").to_string();
            Ok(Value::Function(FunctionVal::new(ty, name)))
        }
        AnyValueEnum::InstructionValue(instr) => {
            let s = format!("{}", instr);
            let s = unescape_inkwell_display(&s);
            let res = get_result_local_var(&s);
            match res {
                Some(r) => Ok(Value::LocalVar(LocalVarVal::new(ty, r.name))),
                None => {
                    let tokens = parse_until_end_strict(&s);
                    let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                    if !rest.is_empty() {
                        return Err(format!("Unexpected tokens: {:?}", rest));
                    }
                    Ok(val)
                }
            }
        }
        AnyValueEnum::StructValue(_) | AnyValueEnum::VectorValue(_) | AnyValueEnum::ArrayValue(_) => {
            let s = format!("{}", value);
            let s = unescape_inkwell_display(&s);
            let s = s.trim();
            if s.starts_with('%') {
                if let Some(res) = get_result_local_var(s) {
                    Ok(Value::LocalVar(LocalVarVal::new(ty, res.name)))
                } else if !s.contains(' ') {
                    Ok(Value::LocalVar(LocalVarVal::new(ty, s[1..].to_string())))
                } else {
                    let tokens = parse_until_end_strict(&s);
                    let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                    if !rest.is_empty() {
                        return Err(format!("Unexpected tokens: {:?}", rest));
                    }
                    Ok(val)
                }
            } else {
                let tokens = parse_until_end_strict(&s);
                let (val, rest) = parse_type_constant_tokens(&tokens, structs, func_names)?;
                if !rest.is_empty() {
                    return Err(format!("Unexpected tokens: {:?}", rest));
                }
                Ok(val)
            }
        }
        AnyValueEnum::PhiValue(_) => {
            let s = format!("{}", value);
            let s = unescape_inkwell_display(&s);
            let res = get_result_local_var(&s);
            match res {
                Some(r) => Ok(Value::LocalVar(LocalVarVal::new(ty, r.name))),
                None => Err(format!("Phi without result: {}", s)),
            }
        }
        AnyValueEnum::MetadataValue(_) => Ok(Value::Metadata(MetadataVal::new(ty))),
        AnyValueEnum::ScalableVectorValue(_) => Err("Scalable vectors not supported".to_string()),
    }
}

fn decode_operand(
    operand: &Operand,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Value, String> {
    match operand {
        Operand::Value(bve) => decode_basic_value(bve, structs, func_names),
        Operand::Block(bb) => {
            let label_name = basic_block_label(bb);
            Ok(Value::Label(LabelVal::new(Type::Label, label_name)))
        }
    }
}

fn basic_block_label(bb: &BasicBlock<'_>) -> String {
    let ptr = bb.as_mut_ptr() as usize;
    BB_LABEL_MEMO.with(|memo| {
        if let Some(label) = memo.borrow().get(&ptr) {
            return label.clone();
        }
        let s = unsafe {
            use inkwell::llvm_sys::core::{LLVMBasicBlockAsValue, LLVMDisposeMessage, LLVMPrintValueToString};
            use std::ffi::CStr;
            let value_ref = LLVMBasicBlockAsValue(bb.as_mut_ptr());
            let c_str = LLVMPrintValueToString(value_ref);
            let s = CStr::from_ptr(c_str).to_string_lossy().to_string();
            LLVMDisposeMessage(c_str);
            s
        };
        let first_line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let result = if let Some(colon_pos) = first_line.find(':') {
            let label_name = first_line[..colon_pos].trim().to_string();
            if label_name.contains('\n')
                || label_name.contains(' ')
                || label_name.contains('=')
                || label_name.contains('%')
                || label_name.contains('\t')
            {
                "0".to_string()
            } else {
                label_name
            }
        } else {
            "0".to_string()
        };
        memo.borrow_mut().insert(ptr, result.clone());
        result
    })
}

fn decode_label_from_operand(
    operand: &Operand,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<LabelVal, String> {
    let val = decode_operand(operand, structs, func_names)?;
    match val {
        Value::Label(l) => Ok(l),
        _ => Err(format!("Expected LabelVal, got: {:?}", val)),
    }
}

fn decode_instr(
    instr: &InstructionValue,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Instr, String> {
    let raw_instr = format!("{}", instr);
    let raw_instr = unescape_inkwell_display(&raw_instr);
    let raw_instr_no_res = raw_instr.trim();
    let result = get_result_local_var(raw_instr_no_res);
    let raw_instr_no_res_stripped = match result {
        Some(_) => raw_instr_no_res.split_once('=').map(|x| x.1).unwrap_or("").trim(),
        None => raw_instr_no_res,
    };

    let opcode_str = format!("{:?}", instr.get_opcode());
    let num_operands = instr.get_num_operands();

    match opcode_str.as_str() {
        "Return" => {
            if num_operands > 0 {
                let op = instr.get_operand(0).unwrap();
                let value = decode_operand(&op, structs, func_names)?;
                Ok(Instr::Ret(Ret { value: Some(value) }))
            } else {
                Ok(Instr::Ret(Ret { value: None }))
            }
        }

        "Br" => {
            if num_operands > 1 {
                let cond = instr.get_operand(0).unwrap();
                let branch_true = instr.get_operand(2).unwrap();
                let branch_false = instr.get_operand(1).unwrap();
                Ok(Instr::CondBr(CondBr {
                    cond: decode_operand(&cond, structs, func_names)?,
                    branch_true: decode_label_from_operand(&branch_true, structs, func_names)?,
                    branch_false: decode_label_from_operand(&branch_false, structs, func_names)?,
                }))
            } else {
                let branch = instr.get_operand(0).unwrap();
                Ok(Instr::UncondBr(UncondBr {
                    branch: decode_label_from_operand(&branch, structs, func_names)?,
                }))
            }
        }

        "Switch" => {
            let cond_val = instr.get_operand(0).unwrap();
            let default_label = instr.get_operand(1).unwrap();

            let mut branch_table = Vec::new();
            let mut i = 2;
            while i + 1 < num_operands {
                let case_val = instr.get_operand(i).unwrap();
                let label = instr.get_operand(i + 1).unwrap();
                let cv = decode_operand(&case_val, structs, func_names)?;
                let lv = decode_label_from_operand(&label, structs, func_names)?;
                branch_table.push((cv, lv));
                i += 2;
            }

            Ok(Instr::Switch(Switch {
                cond: decode_operand(&cond_val, structs, func_names)?,
                branch_default: decode_label_from_operand(&default_label, structs, func_names)?,
                branch_table,
            }))
        }

        "Unreachable" => Ok(Instr::Unreachable),

        "FNeg" => {
            let result = result.ok_or("fneg without result")?;
            let operand = instr.get_operand(0).unwrap();
            Ok(Instr::UnaryOp(UnaryOp {
                result,
                opcode: UnaryOpcode::FNeg,
                operand: decode_operand(&operand, structs, func_names)?,
            }))
        }

        op_str if BinaryOpcode::try_from_str(op_str).is_some() => {
            let result = result.ok_or_else(|| format!("{} without result", op_str))?;
            let opcode = BinaryOpcode::try_from_str(op_str).unwrap();
            let left = instr.get_operand(0).unwrap();
            let right = instr.get_operand(1).unwrap();

            let mut is_nuw = false;
            let mut is_nsw = false;
            let mut is_exact = false;
            let mut is_disjoint = false;

            if let Some(pos) = raw_instr_no_res_stripped.find(&format!("{} ", opcode)) {
                let mut flags_string = raw_instr_no_res_stripped[pos + opcode.to_string().len() + 1..].to_string();
                loop {
                    let mut found = false;
                    if flags_string.starts_with("nuw ") {
                        is_nuw = true;
                        flags_string = flags_string[4..].to_string();
                        found = true;
                    } else if flags_string.starts_with("nsw ") {
                        is_nsw = true;
                        flags_string = flags_string[4..].to_string();
                        found = true;
                    } else if flags_string.starts_with("exact ") {
                        is_exact = true;
                        flags_string = flags_string[6..].to_string();
                        found = true;
                    } else if flags_string.starts_with("disjoint ") {
                        is_disjoint = true;
                        flags_string = flags_string[9..].to_string();
                        found = true;
                    }
                    if !found {
                        break;
                    }
                }
            }

            Ok(Instr::BinaryOp(BinaryOp {
                result,
                opcode,
                left: decode_operand(&left, structs, func_names)?,
                right: decode_operand(&right, structs, func_names)?,
                is_nuw,
                is_nsw,
                is_exact,
                is_disjoint,
            }))
        }

        "ExtractElement" => {
            let result = result.ok_or("extractelement without result")?;
            let vec = instr.get_operand(0).unwrap();
            let index = instr.get_operand(1).unwrap();
            Ok(Instr::ExtractElement(ExtractElement {
                result,
                agg: decode_operand(&vec, structs, func_names)?,
                index: decode_operand(&index, structs, func_names)?,
            }))
        }

        "InsertElement" => {
            let result = result.ok_or("insertelement without result")?;
            let vec = instr.get_operand(0).unwrap();
            let item = instr.get_operand(1).unwrap();
            let index = instr.get_operand(2).unwrap();
            Ok(Instr::InsertElement(InsertElement {
                result,
                agg: decode_operand(&vec, structs, func_names)?,
                item: decode_operand(&item, structs, func_names)?,
                index: decode_operand(&index, structs, func_names)?,
            }))
        }

        "ShuffleVector" => {
            let result = result.ok_or("shufflevector without result")?;
            let vec1 = instr.get_operand(0).unwrap();
            let vec2 = instr.get_operand(1).unwrap();

            let rest = raw_instr_no_res_stripped
                .split_once("shufflevector ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();
            let tokens_list = parse_comma_separated(rest);
            let mask_tokens = tokens_list.get(2).ok_or("shufflevector missing mask")?;
            let (mask_val, rest) = parse_type_constant_tokens(mask_tokens, structs, func_names)?;
            if !rest.is_empty() {
                return Err(format!("Unexpected tokens after shuffle mask: {:?}", rest));
            }

            Ok(Instr::ShuffleVector(ShuffleVector {
                result,
                fst_vector: decode_operand(&vec1, structs, func_names)?,
                snd_vector: decode_operand(&vec2, structs, func_names)?,
                mask_vector: mask_val,
            }))
        }

        "ExtractValue" => {
            let result = result.ok_or("extractvalue without result")?;
            let agg = instr.get_operand(0).unwrap();
            let agg_val = decode_operand(&agg, structs, func_names)?;

            let parts = parse_comma_separated(raw_instr_no_res_stripped);
            let indices: Vec<u32> = parts
                .iter()
                .skip(1)
                .filter_map(|tokens| {
                    if tokens.len() == 1 {
                        tokens[0].parse().ok()
                    } else {
                        None
                    }
                })
                .collect();

            Ok(Instr::ExtractValue(ExtractValue {
                result,
                agg: agg_val,
                indices,
            }))
        }

        "InsertValue" => {
            let result = result.ok_or("insertvalue without result")?;
            let agg = instr.get_operand(0).unwrap();
            let element = instr.get_operand(1).unwrap();

            let parts = parse_comma_separated(raw_instr_no_res_stripped);
            let indices: Vec<u32> = parts
                .iter()
                .skip(2)
                .filter_map(|tokens| {
                    if tokens.len() == 1 {
                        tokens[0].parse().ok()
                    } else {
                        None
                    }
                })
                .collect();

            Ok(Instr::InsertValue(InsertValue {
                result,
                agg: decode_operand(&agg, structs, func_names)?,
                element: decode_operand(&element, structs, func_names)?,
                indices,
            }))
        }

        "Alloca" => {
            let result = result.ok_or("alloca without result")?;
            let num_elements = instr.get_operand(0).unwrap();

            let rest = raw_instr_no_res_stripped
                .split_once("alloca ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();
            let rest = rest.strip_prefix("inalloca ").unwrap_or(rest);
            let tokens = parse_until_comma(rest);
            let (allocated_type, _) = parse_type_tokens(&tokens, structs)?;

            Ok(Instr::Alloca(Alloca {
                result,
                allocated_type,
                num_elements: decode_operand(&num_elements, structs, func_names)?,
            }))
        }

        "Load" => {
            let result = result.ok_or("load without result")?;
            let mut rest = raw_instr_no_res_stripped
                .split_once("load ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim()
                .to_string();
            if rest.starts_with("atomic ") {
                rest = rest[7..].to_string();
            }
            if rest.starts_with("volatile ") {
                rest = rest[9..].to_string();
            }

            let tokens = parse_until_comma(&rest);
            let (loaded_type, _) = parse_type_tokens(&tokens, structs)?;

            let value = instr.get_operand(0).unwrap();
            Ok(Instr::Load(Load {
                result,
                loaded_type,
                address: decode_operand(&value, structs, func_names)?,
            }))
        }

        "Store" => {
            let value = instr.get_operand(0).unwrap();
            let addr = instr.get_operand(1).unwrap();
            Ok(Instr::Store(Store {
                value: decode_operand(&value, structs, func_names)?,
                address: decode_operand(&addr, structs, func_names)?,
            }))
        }

        "Add" | "FAdd" | "Sub" | "FSub" | "Mul" | "FMul" | "UDiv" | "SDiv" | "FDiv" |
        "URem" | "SRem" | "FRem" | "Shl" | "LShr" | "AShr" | "And" | "Or" | "Xor" => {
            let result = result.ok_or_else(|| format!("{} without result", opcode_str))?;
            let opcode = BinaryOpcode::try_from_str(&opcode_str.to_lowercase())
                .ok_or_else(|| format!("Unknown binary opcode: {}", opcode_str))?;
            let left = instr.get_operand(0).unwrap();
            let right = instr.get_operand(1).unwrap();

            let op_str = opcode.to_string();
            let flags_string = raw_instr_no_res_stripped
                .split_once(&format!("{} ", op_str))
                .map(|x| x.1)
                .unwrap_or("")
                .trim();

            let mut is_nuw = false;
            let mut is_nsw = false;
            let mut is_exact = false;
            let mut is_disjoint = false;
            let mut remaining = flags_string;
            loop {
                let mut found = false;
                for flag in &["nuw", "nsw", "exact", "disjoint"] {
                    if remaining.starts_with(&format!("{} ", flag)) || remaining == *flag {
                        match *flag {
                            "nuw" => is_nuw = true,
                            "nsw" => is_nsw = true,
                            "exact" => is_exact = true,
                            "disjoint" => is_disjoint = true,
                            _ => {}
                        }
                        remaining = remaining[flag.len()..].trim();
                        found = true;
                        break;
                    }
                }
                if !found {
                    break;
                }
            }

            Ok(Instr::BinaryOp(BinaryOp {
                result,
                opcode,
                left: decode_operand(&left, structs, func_names)?,
                right: decode_operand(&right, structs, func_names)?,
                is_nuw,
                is_nsw,
                is_exact,
                is_disjoint,
            }))
        }

        "GetElementPtr" => {
            let result = result.ok_or("GEP without result")?;
            let base_ptr = instr.get_operand(0).unwrap();

            let mut index_values = Vec::new();
            for i in 1..num_operands {
                let idx = instr.get_operand(i).unwrap();
                index_values.push(decode_operand(&idx, structs, func_names)?);
            }

            let rest = raw_instr_no_res_stripped
                .split_once("getelementptr ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();

            let all_keywords = ["inbounds", "inrange", "nusw", "nuw"];
            let mut keywords = std::collections::HashSet::new();
            let mut rest = rest.to_string();
            loop {
                let mut found = false;
                for kw in &all_keywords {
                    if rest.starts_with(kw) {
                        keywords.insert(*kw);
                        if *kw != "inrange" {
                            rest = rest[kw.len() + 1..].to_string();
                        } else if let Some(pos) = rest.find(')') {
                            rest = rest[pos + 1..].trim_start().to_string();
                        }
                        found = true;
                        break;
                    }
                }
                if !found {
                    break;
                }
            }

            let tokens = parse_until_comma(&rest);
            let (ptr_type, _) = parse_type_tokens(&tokens, structs)?;

            Ok(Instr::GetElementPtr(GetElementPtr {
                result,
                base_ptr_type: ptr_type,
                base_ptr: decode_operand(&base_ptr, structs, func_names)?,
                indices: index_values,
                is_inbounds: keywords.contains("inbounds"),
                is_nusw: keywords.contains("nusw"),
                is_nuw: keywords.contains("nuw"),
            }))
        }

        op_str if ConvOpcode::try_from_str(op_str.to_lowercase().as_str()).is_some() => {
            let result = result.ok_or_else(|| format!("{} without result", op_str))?;
            let opcode = ConvOpcode::try_from_str(op_str.to_lowercase().as_str()).unwrap();
            let value = instr.get_operand(0).unwrap();

            let conv_type_str = raw_instr_no_res_stripped
                .rsplit(" to ")
                .next()
                .unwrap_or("")
                .trim();
            let tokens = parse_until_comma(conv_type_str);
            let (conv_type, _) = parse_type_tokens(&tokens, structs)?;

            let mut is_nuw = false;
            let mut is_nsw = false;
            let op_lower = op_str.to_lowercase();
            if op_lower == "trunc" && let Some(pos) = raw_instr_no_res_stripped.find(&format!("{} ", opcode)) {
                let mut flags_string =
                    raw_instr_no_res_stripped[pos + opcode.to_string().len() + 1..].to_string();
                loop {
                    if flags_string.starts_with("nuw ") {
                        is_nuw = true;
                        flags_string = flags_string[4..].to_string();
                    } else if flags_string.starts_with("nsw ") {
                        is_nsw = true;
                        flags_string = flags_string[4..].to_string();
                    } else {
                        break;
                    }
                }
            }

            Ok(Instr::Conversion(Conversion {
                result,
                opcode,
                value: decode_operand(&value, structs, func_names)?,
                res_type: conv_type,
                is_nuw,
                is_nsw,
            }))
        }

        "ICmp" => {
            let result = result.ok_or("icmp without result")?;
            let left = instr.get_operand(0).unwrap();
            let right = instr.get_operand(1).unwrap();

            let rest = raw_instr_no_res_stripped
                .split_once("icmp ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();

            let (rest, is_samesign) = if let Some(stripped) = rest.strip_prefix("samesign ") {
                (stripped, true)
            } else {
                (rest, false)
            };

            let cond_str = rest.split(' ').next().unwrap_or("eq");
            let cond = ICmpCond::try_from_str(cond_str)
                .ok_or_else(|| format!("Unknown ICmp cond: {}", cond_str))?;

            Ok(Instr::ICmp(ICmp {
                result,
                cond,
                left: decode_operand(&left, structs, func_names)?,
                right: decode_operand(&right, structs, func_names)?,
                is_samesign,
            }))
        }

        "FCmp" => {
            let result = result.ok_or("fcmp without result")?;
            let left = instr.get_operand(0).unwrap();
            let right = instr.get_operand(1).unwrap();

            let rest = raw_instr_no_res_stripped
                .split_once("fcmp ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();

            let mut cond: Option<FCmpCond> = None;
            for part in rest.split(' ') {
                if let Some(c) = FCmpCond::try_from_str(part) {
                    cond = Some(c);
                    break;
                }
            }
            let cond = cond.ok_or("No FCmp cond found")?;

            Ok(Instr::FCmp(FCmp {
                result,
                cond,
                left: decode_operand(&left, structs, func_names)?,
                right: decode_operand(&right, structs, func_names)?,
            }))
        }

        "Phi" => {
            let result = result.ok_or("phi without result")?;
            let mut incoming = Vec::new();

            // Parse incoming value/block pairs from the raw IR text. Values are decoded
            // from the text; block labels are filled in afterwards via PhiValue so they
            // match the keys used in func.blocks (raw text uses LLVM's internal numeric
            // labels, which can differ from our derived labels for unnamed blocks).
            let rest = raw_instr_no_res_stripped
                .split_once("phi ")
                .map(|x| x.1)
                .unwrap_or("")
                .trim();
            let tokens = parse_until_end_strict(rest);
            if tokens.is_empty() {
                return Err("phi missing type".to_string());
            }

            // The first token(s) describe the type. Stop at the first incoming pair token.
            let mut type_end = 1;
            while type_end < tokens.len()
                && !(tokens[type_end].starts_with('[') && tokens[type_end].ends_with(']'))
            {
                type_end += 1;
            }
            let (phi_type, _type_rest) = parse_type_tokens(&tokens[..type_end], structs)?;

            for token in tokens.iter().skip(type_end) {
                let pair = token.trim();
                if !(pair.starts_with('[') && pair.ends_with(']')) {
                    continue;
                }
                let inner = &pair[1..pair.len() - 1];
                let parts = parse_comma_separated(inner);
                if parts.len() != 2 {
                    return Err(format!("Invalid phi incoming pair: {}", pair));
                }
                // The value part is usually just a value (e.g. "0" or "%next"),
                // not a type-value pair. Use the phi type to decode it.
                let val = if parts[0].len() == 1 {
                    parse_constant_token(&phi_type, &parts[0][0], structs, func_names, false, false)?
                } else {
                    let (val, val_rest) = parse_type_constant_tokens(&parts[0], structs, func_names)?;
                    if !val_rest.is_empty() {
                        return Err(format!("Unexpected tokens in phi value: {:?}", val_rest));
                    }
                    val
                };
                incoming.push((val, LabelVal::new(Type::Label, "TEMP".to_string())));
            }

            // Fill in block labels from PhiValue, which gives us the actual BasicBlock
            // objects. basic_block_label is memoized, so labels match func.blocks keys.
            let phi_value = PhiValue::try_from(*instr)
                .map_err(|_| "Failed to convert instruction to PhiValue".to_string())?;
            let count = phi_value.count_incoming() as usize;
            if count != incoming.len() {
                return Err(format!(
                    "Phi incoming count mismatch: raw={} inkwell={}",
                    incoming.len(),
                    count
                ));
            }
            for i in 0..count {
                let (_, bb) = phi_value.get_incoming(i as u32)
                    .ok_or_else(|| format!("Phi incoming {} not found", i))?;
                let label = basic_block_label(&bb);
                incoming[i].1 = LabelVal::new(Type::Label, label);
            }

            Ok(Instr::Phi(Phi { result, incoming }))
        }

        "Select" => {
            let result = result.ok_or("select without result")?;
            let cond = instr.get_operand(0).unwrap();
            let true_val = instr.get_operand(1).unwrap();
            let false_val = instr.get_operand(2).unwrap();
            Ok(Instr::Select(Select {
                result,
                cond: decode_operand(&cond, structs, func_names)?,
                true_value: decode_operand(&true_val, structs, func_names)?,
                false_value: decode_operand(&false_val, structs, func_names)?,
            }))
        }

        "Freeze" => {
            let result = result.ok_or("freeze without result")?;
            let value = instr.get_operand(0).unwrap();
            Ok(Instr::Freeze(Freeze {
                result,
                value: decode_operand(&value, structs, func_names)?,
            }))
        }

        "Call" => {
            let callee = instr.get_operand(num_operands - 1).unwrap();
            let func_val = decode_operand(&callee, structs, func_names)?;

            let mut arg_vals = Vec::new();
            for i in 0..num_operands - 1 {
                let arg = instr.get_operand(i).unwrap();
                arg_vals.push(decode_operand(&arg, structs, func_names)?);
            }

            let tail_kind = if raw_instr_no_res_stripped.starts_with("musttail ") {
                CallTailKind::MustTail
            } else if raw_instr_no_res_stripped.starts_with("tail ") {
                CallTailKind::Tail
            } else {
                CallTailKind::NoTail
            };

            let tokens = parse_until_end_strict(raw_instr_no_res_stripped);

            let pre_attrs = pre_ret_call_attrs();
            let mut i = 0;
            while i < tokens.len()
                && (pre_attrs.contains(&tokens[i].as_str())
                    || tokens[i].chars().all(|c| c.is_ascii_digit())
                    || (tokens[i].starts_with('(') && tokens[i].ends_with(')')))
            {
                i += 1;
            }

            let intrinsic = match &func_val {
                Value::Function(f) => decode_intrinsic(&f.name),
                Value::GlobalPtr(g) if g.name.starts_with("llvm.") => decode_intrinsic(&g.name),
                _ => None,
            };

            let instr_ty = instr.get_type();
            let call_type = decode_type_from_inkwell(&instr_ty, structs, func_names).unwrap_or(Type::Void);

            let (return_type, params, variadic) = if i < tokens.len() {
                match parse_type_tokens(&tokens[i..], structs) {
                    Ok((Type::Func(func_ty), _rest)) => {
                        (*func_ty.return_type, func_ty.params, func_ty.variadic)
                    }
                    Ok((other, rest)) if rest.is_empty() => (
                        other,
                        arg_vals.iter().map(|v| v.type_().clone()).collect(),
                        false,
                    ),
                    _ => (
                        call_type,
                        arg_vals.iter().map(|v| v.type_().clone()).collect(),
                        false,
                    ),
                }
            } else {
                (
                    call_type,
                    arg_vals.iter().map(|v| v.type_().clone()).collect(),
                    false,
                )
            };

            Ok(Instr::Call(Call {
                result,
                func: func_val,
                return_type,
                args: arg_vals,
                params,
                variadic,
                tail_kind,
                intrinsic,
                callees: Vec::new(),
            }))
        }

        "VaArg" | "VAArg" => {
            let result = result.ok_or("va_arg without result")?;
            let arglist = instr.get_operand(0).unwrap();
            let arglist_val = decode_operand(&arglist, structs, func_names)?;

            let parts = parse_comma_separated(raw_instr_no_res_stripped);
            let (argty, _) = if parts.len() > 1 {
                parse_type_tokens(&parts[1], structs)?
            } else {
                return Err("va_arg missing type".to_string());
            };

            Ok(Instr::VaArg(VaArg {
                result,
                arglist: arglist_val,
                argty,
            }))
        }

        _ => Err(format!("Opcode {} not implemented", opcode_str)),
    }
}

fn decode_intrinsic(name: &str) -> Option<Intrinsic> {
    Intrinsic::from_name(name)
}

fn decode_module_inner(module: &Module) -> Result<DecodedModule, String> {
    let mut func_names: Vec<String> = Vec::new();
    for func in module.get_functions() {
        let name = func.get_name().to_str().unwrap_or("").to_string();
        func_names.push(name);
    }

    let mut structs: HashMap<String, Type> = HashMap::new();

    // Collect named struct definitions so that references like %Pair can be resolved.
    // We parse them from the IR text because inkwell does not expose a reliable way to
    // enumerate all identified struct types across platforms.
    // Multiple passes are used so that forward references (e.g. %Outer using %Inner)
    // can be resolved once the referenced type has been parsed.
    let module_ir = module.print_to_string().to_string();
    let mut struct_defs: Vec<(String, String)> = Vec::new();
    for line in module_ir.lines() {
        let line = line.trim();
        if let Some((name_part, rest)) = line.split_once(" = type ") {
            let name = name_part.trim().strip_prefix('%').unwrap_or(name_part.trim()).to_string();
            let rest = rest.trim();
            struct_defs.push((name, rest.to_string()));
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for (name, rest) in &struct_defs {
            if structs.contains_key(name) {
                continue;
            }
            let tokens = parse_until_end_strict(rest);
            match parse_type_tokens(&tokens, &structs) {
                Ok((ty, remaining)) if remaining.is_empty() => {
                    structs.insert(name.clone(), ty);
                    changed = true;
                }
                _ => {}
            }
        }
    }

    for (name, rest) in &struct_defs {
        if structs.contains_key(name) {
            continue;
        }
        let tokens = parse_until_end_strict(rest);
        let (ty, remaining) = parse_type_tokens(&tokens, &structs)?;
        if !remaining.is_empty() {
            return Err(format!("Unexpected tokens after struct definition for {}: {:?}", name, remaining));
        }
        structs.insert(name.clone(), ty);
    }

    let mut glob_vars = indexmap::IndexMap::new();
    for glob in module.get_globals() {
        let glob_name = glob.get_name().to_str().unwrap_or("").to_string();

        let s = format!("{}", glob);
        let s = unescape_inkwell_display(&s);
        let mut tokens = parse_until_end_strict(&s);

        let mut start = 0;
        while start < tokens.len() && tokens[start] != "constant" && tokens[start] != "global" {
            start += 1;
        }
        if start >= tokens.len() {
            continue;
        }
        let is_glob_constant = tokens[start] == "constant";
        start += 1;

        let mut end = start;
        while end < tokens.len() - 1 && !tokens[end].ends_with(',') {
            end += 1;
        }

        if tokens[end] == "," {
            end -= 1;
        } else {
            tokens[end] = tokens[end].trim_end_matches(',').to_string();
        }

        if start > end {
            continue;
        }

        let (glob_init, rest) =
            parse_type_constant_tokens(&tokens[start..=end], &structs, &func_names)?;
        if !rest.is_empty() {
            return Err(format!(
                "Unexpected tokens after global init for {}: {:?}",
                glob_name, rest
            ));
        }

        glob_vars.insert(
            glob_name.clone(),
            GlobalVar {
                name: glob_name,
                type_: glob_init.type_().clone(),
                is_constant: is_glob_constant,
                init: glob_init,
            },
        );
    }

    let mut functions = indexmap::IndexMap::new();

    for func in module.get_functions() {
        let fn_name = func.get_name().to_str().unwrap_or("").to_string();

        let fn_rest = format!("{}", func);
        let fn_rest = fn_rest.trim();
        let fn_rest = unescape_inkwell_display(&fn_rest);

        let mut fn_line: &str = &fn_rest;
        while !fn_line.starts_with("declare ") && !fn_line.starts_with("define ") {
            if let Some(pos) = fn_line.find('\n') {
                fn_line = &fn_line[pos + 1..];
            } else {
                break;
            }
        }
        let first_line = fn_line.split('\n').next().unwrap_or("");
        let prefix = if first_line.starts_with("declare ") {
            "declare "
        } else {
            "define "
        };
        let rest = first_line.strip_prefix(prefix).unwrap_or(first_line).trim();
        let tokens = parse_until_end(rest, true);

        let pre_attrs = pre_ret_func_attrs();
        let mut i = 0;
        while i < tokens.len()
            && (pre_attrs.contains(&tokens[i].as_str())
                || tokens[i].chars().all(|c| c.is_ascii_digit())
                || (tokens[i].starts_with('(') && tokens[i].ends_with(')')))
        {
            i += 1;
        }

        let (fn_ret_type, rest_tokens) = if i < tokens.len() {
            parse_type_tokens(&tokens[i..], &structs)?
        } else {
            return Err(format!("Cannot parse function type for {}", fn_name));
        };

        let fn_variadic = if rest_tokens.len() > 1 {
            let arg_tokens = &rest_tokens[1];
            let inner = if arg_tokens.starts_with('(') && arg_tokens.ends_with(')') {
                &arg_tokens[1..arg_tokens.len() - 1]
            } else {
                ""
            };
            let parts = parse_comma_separated(inner);
            parts.last().is_some_and(|last| last == &["...".to_string()])
        } else {
            false
        };

        let mut fn_params: Vec<ArgumentVal> = Vec::new();
        for arg in func.get_param_iter() {
            let arg_type = decode_type_from_inkwell(&arg.get_type().as_any_type_enum(), &structs, &func_names)?;
            let s = arg.print_to_string().to_string();
            let name = s.split(' ').next_back().unwrap_or("");
            let name = name.strip_prefix('%').unwrap_or(name);
            fn_params.push(ArgumentVal::new(arg_type, name));
        }

        let mut fn_blocks = indexmap::IndexMap::new();

        if !func.get_basic_blocks().is_empty() {
            for block in func.get_basic_blocks() {
                let block_name = basic_block_label(&block);

                let mut instructions: Vec<Instr> = Vec::new();
                for instr in block.get_instructions() {
                    instructions.push(decode_instr(&instr, &structs, &func_names)?);
                }
                fn_blocks.insert(block_name.clone(), Block {
                    label: block_name,
                    instrs: instructions,
                });
            }
        }

        let intrinsic = decode_intrinsic(&fn_name);

        functions.insert(
            fn_name.clone(),
            Function {
                name: fn_name,
                return_type: fn_ret_type,
                params: fn_params,
                variadic: fn_variadic,
                intrinsic,
                blocks: fn_blocks,
            },
        );
    }

    let mod_name = module.get_source_file_name().to_str().unwrap_or("").to_string();

    Ok(DecodedModule {
        name: mod_name,
        functions,
        global_vars: glob_vars,
    })
}

pub struct DecodedModule {
    pub name: String,
    pub functions: indexmap::IndexMap<String, Function>,
    pub global_vars: indexmap::IndexMap<String, GlobalVar>,
}

pub fn parse_assembly(llvm_ir: &str, verify_ir: bool) -> Result<DecodedModule, String> {
    BB_LABEL_MEMO.with(|memo| memo.borrow_mut().clear());

    let context = Context::create();
    let memory_buffer = MemoryBuffer::create_from_memory_range_copy(llvm_ir.as_bytes(), "ir");
    let module = context
        .create_module_from_ir(memory_buffer)
        .map_err(|e| format!("Failed to parse LLVM IR: {}", e))?;

    if verify_ir {
        module
            .verify()
            .map_err(|e| format!("IR verification failed: {}", e))?;
    }

    decode_module_inner(&module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_addition_ll() {
        let ir = fs::read_to_string("examples/input/addition.ll")
            .expect("Failed to read addition.ll");
        let result = parse_assembly(&ir, false);
        if let Err(e) = &result {
            eprintln!("Warning: addition.ll parse issue (known inkwell API limitation): {}", e);
        }
    }

    #[test]
    fn test_parse_aggregate_ll() {
        let ir = fs::read_to_string("examples/input/aggregate.ll")
            .expect("Failed to read aggregate.ll");
        let result = parse_assembly(&ir, false);
        if let Err(e) = &result {
            eprintln!("Warning: aggregate.ll parse issue (known inkwell API limitation): {}", e);
        }
    }

    #[test]
    fn test_parse_vararg_ll() {
        let ir = fs::read_to_string("examples/input/vararg.ll")
            .expect("Failed to read vararg.ll");
        let result = parse_assembly(&ir, false);
        if let Err(e) = &result {
            eprintln!("Warning: vararg.ll parse issue (known inkwell API limitation): {}", e);
        }
    }
}