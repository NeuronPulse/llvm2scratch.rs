use std::collections::{BTreeMap, HashMap, HashSet};

use indexmap::IndexMap;
use num_bigint::BigUint;
use num_traits::ToPrimitive;

use crate::ir;
use crate::ir::instructions::BinaryOpcode;
use crate::ir::values::LocalVarVal;
use crate::ir::types::{FuncTy, IntegerTy, Type};
use crate::parser::llvm_decode::DecodedModule;
use crate::scratch::{self, Block, BlockList, Project, Value};
use crate::scratch::ast::{BoolOp, ControlFlow, ControlOp, CounterOp, CostumeInfoOp, KnownVal, ListEditOp, ListOp, Op, PenOp, VarOp, VolumeOp, StopOption};
use crate::scratch::ast::{GetOfList, EditListData, EditVarData, ProcedureDefData, ProcedureCallData};

use super::config::{
    CompException, CompilerConfig, FuncInfo, FuncPtrSigInfo, BlockInfo,
    Variable, VarType, IdxbleValue, ReturnAddrInfo, BlockVarUse, PTR_WIDTH_BITS,
};
use super::variable::{get_value_cost, InferredValue};
use super::binop::{self, BinopKind};
use super::memory;
use super::twos_complement;
use crate::optimizer::known_value_prop::simplify_value;

#[derive(Debug, Clone)]
pub struct SetjmpSite {
    pub fn_name: String,
    pub block_label: String,
    pub call_idx: usize,
    pub dispatch_id: usize,
    pub continuation_name: String,
    pub result_var: String,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub proj: Project,
    pub cfg: CompilerConfig,
    pub fn_info: HashMap<String, FuncInfo>,
    pub fn_ptr_sig_info: Vec<FuncPtrSigInfo>,
    pub fn_ptr_sigs: Vec<(FuncTy, Vec<String>)>,
    pub globvar_to_ptr: HashMap<String, usize>,
    pub highest_return_size: Option<usize>,
    pub next_fn_id: usize,
    pub min_func_ptr_addr: usize,
    pub all_check_locations: Vec<(String, String)>,
    pub setjmp_sites: Vec<SetjmpSite>,
    pub needs_and_lut: bool,
    pub needs_or_lut: bool,
    pub needs_xor_lut: bool,
    pub needs_str2color: bool,
    pub functions: HashMap<String, ir::Function>,
    uid_counter: usize,
}

impl Context {
    pub fn new(proj: Project, cfg: CompilerConfig) -> Self {
        Context {
            proj,
            cfg,
            fn_info: HashMap::new(),
            fn_ptr_sig_info: Vec::new(),
            fn_ptr_sigs: Vec::new(),
            globvar_to_ptr: HashMap::new(),
            highest_return_size: None,
            next_fn_id: 0,
            min_func_ptr_addr: 0,
            all_check_locations: Vec::new(),
            setjmp_sites: Vec::new(),
            needs_and_lut: false,
            needs_or_lut: false,
            needs_xor_lut: false,
            needs_str2color: false,
            functions: HashMap::new(),
            uid_counter: 0,
        }
    }
}

pub fn trans_value(
    val: &ir::Value,
    ctx: &mut Context,
    bctx: Option<&BlockInfo>,
) -> Result<InferredValue, CompException> {
    match val {
        ir::Value::KnownInt(ki) => {
            if ki.width as usize <= super::config::VARIABLE_MAX_BITS {
                // Keep the raw unsigned value to match Python's KnownIntVal handling.
                Ok(InferredValue::Single(Value::Known(KnownVal::Num(ki.value.to_u64().unwrap_or(0) as f64))))
            } else {
                let mut num = ki.value.clone();
                let mut width = ki.width as usize;
                let mut values = Vec::new();
                let part_mask = num_bigint::BigUint::from(1u32) << super::config::VARIABLE_MAX_BITS;
                while width > 0 {
                    let part = &num % &part_mask;
                    values.push(Value::Known(KnownVal::Num(part.to_u64().unwrap_or(0) as f64)));
                    num >>= super::config::VARIABLE_MAX_BITS;
                    width = width.saturating_sub(super::config::VARIABLE_MAX_BITS);
                }
                Ok(InferredValue::Indexed(IdxbleValue { vals: values }))
            }
        }
        ir::Value::KnownFloat(kf) => Ok(InferredValue::Single(Value::Known(KnownVal::Num(kf.value)))),
        ir::Value::LocalVar(lv) => {
            let is_param = bctx.map_or(false, |bc| {
                bc.available_params.iter().any(|p| p.var_name == lv.name)
            });
            let var = if is_param {
                let fn_name = bctx.map(|bc| bc.fn_info.name.clone()).unwrap_or_default();
                Variable {
                    var_name: lv.name.clone(),
                    var_type: VarType::Param,
                    fn_name: Some(fn_name),
                }
            } else {
                let fn_name = bctx.map(|bc| bc.fn_info.name.as_str());
                trans_local_var(lv, fn_name)
            };
            let size = memory::get_size_of(&lv.type_, false)?;
            if size > 1 {
                Ok(InferredValue::Indexed(var.get_all_values(size)))
            } else {
                Ok(InferredValue::Single(var.get_value(None)))
            }
        }
        ir::Value::Argument(arg) => {
            let size = memory::get_size_of(&arg.type_, false)?;
            let is_param = bctx.map_or(false, |bc| {
                bc.available_params.iter().any(|p| p.var_name == arg.name)
            });
            let fn_name = bctx.map(|bc| bc.fn_info.name.clone());
            let var = if is_param {
                Variable {
                    var_name: arg.name.clone(),
                    var_type: VarType::Param,
                    fn_name,
                }
            } else {
                Variable {
                    var_name: arg.name.clone(),
                    var_type: VarType::Var,
                    fn_name,
                }
            };
            if size > 1 {
                Ok(InferredValue::Indexed(var.get_all_values(size)))
            } else {
                Ok(InferredValue::Single(var.get_value(None)))
            }
        }
        ir::Value::GlobalPtr(gp) => {
            if let Some(&ptr) = ctx.globvar_to_ptr.get(&gp.name) {
                if ctx.cfg.compiler_opt {
                    Ok(InferredValue::Single(Value::Known(KnownVal::Num(ptr as f64))))
                } else {
                    let var = Variable {
                        var_name: gp.name.clone(),
                        var_type: VarType::Global,
                        fn_name: None,
                    };
                    Ok(InferredValue::Single(Value::GetVar {
                        name: var.get_unidxed_raw_var_name(),
                    }))
                }
            } else {
                Err(CompException(format!("Global variable not found: {}", gp.name)))
            }
        }
        ir::Value::Function(fv) => {
            if ctx.fn_ptr_sigs.iter().any(|(_, ptrs)| ptrs.contains(&fv.name)) {
                let ptr_addr = get_func_ptr_addr(&fv.name, ctx)?;
                Ok(InferredValue::Single(Value::Known(KnownVal::Num(ptr_addr as f64))))
            } else if let Some(info) = ctx.fn_info.get(&fv.name) {
                Ok(InferredValue::Single(Value::Known(KnownVal::Num(info.fn_id as f64))))
            } else {
                Err(CompException(format!("Function not found: {}", fv.name)))
            }
        }
        ir::Value::NullPtr(_) => Ok(InferredValue::Single(Value::Known(KnownVal::Num(0.0)))),
        ir::Value::Undef(_) => {
            let size = memory::get_size_of(val.type_(), false)?;
            if size > 1 {
                let vals: Vec<Value> = (0..size)
                    .map(|_| Value::Known(KnownVal::Num(0.0)))
                    .collect();
                Ok(InferredValue::Indexed(IdxbleValue { vals }))
            } else {
                Ok(InferredValue::Single(Value::Known(KnownVal::Num(0.0))))
            }
        }
        ir::Value::KnownVec(kv) => {
            let mut vals = Vec::new();
            for elem in &kv.values {
                let elem_iv = trans_value(elem, ctx, bctx)?;
                vals.extend(inferred_to_values(elem_iv));
            }
            if vals.len() == 1 {
                Ok(InferredValue::Single(vals.into_iter().next().unwrap()))
            } else {
                Ok(InferredValue::Indexed(IdxbleValue { vals }))
            }
        }
        ir::Value::ConstExpr(ce) => match &ce.expr {
            ir::values::ConstExpr::Conversion(c) => trans_value(&c.value, ctx, bctx),
            ir::values::ConstExpr::GetElementPtr(gep) => {
                let val = trans_gep_value(
                    &gep.base_ptr,
                    &gep.base_ptr_type,
                    &gep.indices,
                    gep.is_nuw,
                    ctx,
                    bctx,
                    true,
                )?;
                Ok(InferredValue::Single(val))
            }
            ir::values::ConstExpr::ExtractElement(e) => {
                let vec_iv = trans_value(&e.agg, ctx, bctx)?;
                let idx_iv = trans_value(&e.index, ctx, bctx)?;
                let idx_val = idx_iv.into_single()?;
                let idx_known = match &idx_val {
                    Value::Known(KnownVal::Num(n)) => {
                        if n.fract() == 0.0 && *n >= 0.0 {
                            Some(*n as usize)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let idx = idx_known.ok_or_else(|| {
                    CompException("ConstExpr extractelement requires constant index".to_string())
                })?;
                let vec_ty = e.agg.type_();
                let elem_size = memory::get_size_of(
                    match vec_ty {
                        ir::Type::Vector(vt) => &vt.inner,
                        _ => return Err(CompException("ConstExpr extractelement requires vector type".to_string())),
                    },
                    false,
                )?;
                let vals = inferred_to_values(vec_iv);
                let offset = idx * elem_size;
                let res_vals: Vec<Value> = vals[offset..offset + elem_size].to_vec();
                if res_vals.len() == 1 {
                    Ok(InferredValue::Single(res_vals.into_iter().next().unwrap()))
                } else {
                    Ok(InferredValue::Indexed(IdxbleValue { vals: res_vals }))
                }
            }
            ir::values::ConstExpr::InsertElement(e) => {
                let vec_iv = trans_value(&e.agg, ctx, bctx)?;
                let item_iv = trans_value(&e.item, ctx, bctx)?;
                let idx_iv = trans_value(&e.index, ctx, bctx)?;
                let idx_val = idx_iv.into_single()?;
                let idx_known = match &idx_val {
                    Value::Known(KnownVal::Num(n)) => {
                        if n.fract() == 0.0 && *n >= 0.0 {
                            Some(*n as usize)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let idx = idx_known.ok_or_else(|| {
                    CompException("ConstExpr insertelement requires constant index".to_string())
                })?;
                let vec_ty = e.agg.type_();
                let elem_size = memory::get_size_of(
                    match vec_ty {
                        ir::Type::Vector(vt) => &vt.inner,
                        _ => return Err(CompException("ConstExpr insertelement requires vector type".to_string())),
                    },
                    false,
                )?;
                let mut vals = inferred_to_values(vec_iv);
                let item_vals = inferred_to_values(item_iv);
                let offset = idx * elem_size;
                for (i, v) in item_vals.iter().enumerate() {
                    vals[offset + i] = v.clone();
                }
                if vals.len() == 1 {
                    Ok(InferredValue::Single(vals.into_iter().next().unwrap()))
                } else {
                    Ok(InferredValue::Indexed(IdxbleValue { vals }))
                }
            }
            ir::values::ConstExpr::ShuffleVector(s) => {
                let v1_iv = trans_value(&s.fst_vector, ctx, bctx)?;
                let v2_iv = trans_value(&s.snd_vector, ctx, bctx)?;
                let mask_iv = trans_value(&s.mask_vector, ctx, bctx)?;
                let vec_ty = s.fst_vector.type_();
                let elem_ty = match vec_ty {
                    ir::Type::Vector(vt) => &*vt.inner,
                    _ => return Err(CompException("shufflevector requires vector type".to_string())),
                };
                let elem_size = memory::get_size_of(elem_ty, false)?;
                shuffle_vector(v1_iv, v2_iv, mask_iv, elem_size)
            }
            ir::values::ConstExpr::BinaryOp(bop) => {
                let lft_iv = trans_value(&bop.left, ctx, bctx)?;
                let rgt_iv = trans_value(&bop.right, ctx, bctx)?;
                let width = get_value_width(&bop.left);
                let res = if width > super::config::VARIABLE_MAX_BITS {
                    return Err(CompException(format!(
                        "ConstExpr BinaryOp for wide integer ({}-bit) not supported", width
                    )));
                } else {
                    let lft = lft_iv.into_single()?;
                    let rgt = rgt_iv.into_single()?;
                    match bop.opcode {
                        o if o == ir::instructions::BinaryOpcode::Add => InferredValue::Single(Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::Sub => InferredValue::Single(Value::Op(Op::Sub(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::Mul => InferredValue::Single(Value::Op(Op::Mul(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::UDiv => {
                            let div = Value::Op(Op::Div(Box::new(lft), Box::new(rgt)));
                            InferredValue::Single(if bop.is_exact { div } else { Value::Op(Op::Floor(Box::new(div))) })
                        }
                        o if o == ir::instructions::BinaryOpcode::URem => InferredValue::Single(Value::Op(Op::Mod(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::SDiv || o == ir::instructions::BinaryOpcode::SRem => {
                            return Err(CompException("ConstExpr signed div/rem not supported".to_string()));
                        }
                        o if o == ir::instructions::BinaryOpcode::Shl => {
                            let modulus = Value::Known(KnownVal::Num(2f64.powi(width as i32)));
                            InferredValue::Single(Value::Op(Op::Mod(
                                Box::new(Value::Op(Op::Mul(Box::new(lft), Box::new(Value::Known(KnownVal::Num(2f64.powi(width as i32))))))),
                                Box::new(modulus),
                            )))
                        }
                        o if o == ir::instructions::BinaryOpcode::LShr => {
                            InferredValue::Single(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(Box::new(lft), Box::new(rgt)))))))
                        }
                        o if o == ir::instructions::BinaryOpcode::AShr => {
                            let half = 2f64.powi((width - 1) as i32);
                            let twos = 2f64.powi(width as i32);
                            let cond = Value::BoolOp(BoolOp::Lt(Box::new(lft.clone()), Box::new(Value::Known(KnownVal::Num(half)))));
                            let signed = Value::Op(Op::Sub(Box::new(lft), Box::new(Value::Op(Op::Mul(
                                Box::new(Value::Known(KnownVal::Num(twos))),
                                Box::new(Value::Op(Op::BoolToFloat(Box::new(cond)))),
                            )))));
                            InferredValue::Single(signed) // simplified: just the signed interpretation
                        }
                        o if o == ir::instructions::BinaryOpcode::And || o == ir::instructions::BinaryOpcode::Or || o == ir::instructions::BinaryOpcode::Xor => {
                            return Err(CompException(format!("ConstExpr bitwise {:?} not supported", bop.opcode)));
                        }
                        o if o == ir::instructions::BinaryOpcode::FAdd => InferredValue::Single(Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::FSub => InferredValue::Single(Value::Op(Op::Sub(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::FMul => InferredValue::Single(Value::Op(Op::Mul(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::FDiv => InferredValue::Single(Value::Op(Op::Div(Box::new(lft), Box::new(rgt)))),
                        o if o == ir::instructions::BinaryOpcode::FRem => {
                            let modded = Value::Op(Op::Mod(Box::new(lft.clone()), Box::new(rgt.clone())));
                            InferredValue::Single(Value::Op(Op::Sub(Box::new(modded), Box::new(rgt))))
                        }
                        _ => return Err(CompException(format!("ConstExpr BinaryOp opcode {:?} not supported", bop.opcode))),
                    }
                };
                Ok(res)
            }
            _ => Err(CompException(format!("Cannot translate const expr: {:?}", ce))),
        },
        _ => Err(CompException(format!("Cannot translate value: {:?}", val))),
    }
}

/// Perform an LLVM `shufflevector` on flattened vector values.
///
/// `v1`/`v2` are flattened to chunks of `elem_size`. Each element of the
/// constant `mask` selects one element from the concatenation of `v1` and `v2`
/// (negative / out-of-range indices produce zero, matching undef/poison).
fn shuffle_vector(
    v1: InferredValue,
    v2: InferredValue,
    mask: InferredValue,
    elem_size: usize,
) -> Result<InferredValue, CompException> {
    let v1_vals = inferred_to_values(v1);
    let v2_vals = inferred_to_values(v2);
    let mask_vals = inferred_to_values(mask);
    let v1_len = v1_vals.len() / elem_size;
    let v2_len = v2_vals.len() / elem_size;

    let mut result = Vec::new();
    for mask_val in mask_vals {
        let mask_idx = match &mask_val {
            Value::Known(KnownVal::Num(n)) => {
                if n.fract() == 0.0 {
                    Some(*n as i64)
                } else {
                    None
                }
            }
            _ => None,
        };
        let selected = match mask_idx {
            Some(idx) if idx >= 0 && (idx as usize) < v1_len + v2_len => {
                let idx = idx as usize;
                if idx < v1_len {
                    let offset = idx * elem_size;
                    v1_vals[offset..offset + elem_size].to_vec()
                } else {
                    let idx = idx - v1_len;
                    let offset = idx * elem_size;
                    v2_vals[offset..offset + elem_size].to_vec()
                }
            }
            _ => vec![Value::Known(KnownVal::Num(0.0)); elem_size],
        };
        result.extend(selected);
    }

    if result.len() == 1 {
        Ok(InferredValue::Single(result.into_iter().next().unwrap()))
    } else {
        Ok(InferredValue::Indexed(IdxbleValue { vals: result }))
    }
}

fn trans_gep_value(
    base_ptr: &ir::Value,
    base_ptr_type: &ir::Type,
    indices: &[ir::Value],
    is_nuw: bool,
    ctx: &mut Context,
    bctx: Option<&BlockInfo>,
    force_numeric_global: bool,
) -> Result<Value, CompException> {
    let base = if force_numeric_global {
        if let ir::Value::GlobalPtr(gp) = base_ptr {
            if let Some(&ptr) = ctx.globvar_to_ptr.get(&gp.name) {
                Value::Known(KnownVal::Num(ptr as f64))
            } else {
                return Err(CompException(format!(
                    "Global variable not found: {}",
                    gp.name
                )));
            }
        } else {
            trans_value(base_ptr, ctx, bctx)?.into_single()?
        }
    } else {
        trans_value(base_ptr, ctx, bctx)?.into_single()?
    };
    let index_vals: Vec<(Value, usize)> = indices
        .iter()
        .map(|idx| {
            let width = get_value_width(idx);
            // Constant GEP indices are common in constant expressions. Use the raw
            // integer value directly so wide integer types (e.g. i64) do not get
            // split into multiple variable parts by trans_value.
            let val = if let ir::Value::KnownInt(ki) = idx {
                Value::Known(KnownVal::Num(ki.value.to_u64().unwrap_or(0) as f64))
            } else {
                trans_value(idx, ctx, bctx)?.into_single()?
            };
            Ok((val, width))
        })
        .collect::<Result<Vec<_>, CompException>>()?;
    let gep_result = memory::get_gep_offsets(base_ptr_type, &index_vals, ctx.cfg.accurate_byte_spacing, ctx.cfg.i8_gep_div)?;
    let val = memory::apply_gep_offsets(
        base,
        gep_result.known_offset,
        &gep_result.unknown_offsets,
        is_nuw,
        ctx.cfg.memory_size,
        gep_result.i8_gep_div,
    );
    Ok(simplify_value(&val))
}

fn trans_bitwise_binop(
    opcode: ir::instructions::BinaryOpcode,
    lft_iv: InferredValue,
    rgt_iv: InferredValue,
    width: usize,
    ctx: &mut Context,
) -> Result<InferredValue, CompException> {
    let kind = match opcode {
        ir::instructions::BinaryOpcode::And => BinopKind::And,
        ir::instructions::BinaryOpcode::Or => BinopKind::Or,
        ir::instructions::BinaryOpcode::Xor => BinopKind::Xor,
        _ => return Err(CompException(format!("Unsupported bitwise opcode: {:?}", opcode))),
    };
    match (lft_iv, rgt_iv) {
        (InferredValue::Single(lft), InferredValue::Single(rgt)) => {
            Ok(InferredValue::Single(binop::binop(
                kind, lft, rgt, width, &ctx.cfg,
                &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
            )?))
        }
        (InferredValue::Indexed(lft_idx), InferredValue::Indexed(rgt_idx)) => {
            if lft_idx.vals.len() != rgt_idx.vals.len() {
                return Err(CompException(format!(
                    "Bitwise operands have different part counts: {} vs {}",
                    lft_idx.vals.len(), rgt_idx.vals.len()
                )));
            }
            let mut result_vals = Vec::new();
            // Match Python: pass the full integer width to binOp for each part,
            // even though each part only holds up to VARIABLE_MAX_BITS bits.
            for (l, r) in lft_idx.vals.iter().zip(rgt_idx.vals.iter()) {
                result_vals.push(binop::binop(
                    kind, l.clone(), r.clone(), width, &ctx.cfg,
                    &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
                )?);
            }
            Ok(InferredValue::Indexed(IdxbleValue { vals: result_vals }))
        }
        _ => Err(CompException(format!(
            "Bitwise operands must both be single or both indexed"
        ))),
    }
}

fn inferred_to_values(iv: InferredValue) -> Vec<Value> {
    match iv {
        InferredValue::Single(v) => vec![v],
        InferredValue::Indexed(idx) => idx.vals,
    }
}

fn trans_local_var(lv: &LocalVarVal, fn_name: Option<&str>) -> Variable {
    let name = localize_var(&lv.name, false, fn_name, false);
    Variable {
        var_name: name,
        var_type: VarType::Var,
        fn_name: None,
    }
}

pub fn localize_var(name: &str, is_global: bool, fn_name: Option<&str>, is_param: bool) -> String {
    if is_global {
        format!("@{}", name)
    } else if is_param {
        localize_param(name)
    } else if let Some(fn_) = fn_name {
        format!("%{}:{}", fn_, name)
    } else {
        format!("%{}", name)
    }
}

pub fn localize_label(label: &str, fn_name: &str) -> String {
    format!("{}:{}", fn_name, label)
}

pub fn localize_call_id(call_id: usize, label: &str, fn_name: &str, recursive: bool) -> String {
    if recursive {
        format!("{}:{}:recursive call {}", fn_name, label, call_id)
    } else {
        format!("{}:{}:return addr {}", fn_name, label, call_id)
    }
}

pub fn localize_setjmp_continuation(call_id: usize, label: &str, fn_name: &str) -> String {
    format!("{}:{}:setjmp continue {}", fn_name, label, call_id)
}

pub fn localize_param(name: &str) -> String {
    format!("%{}", name)
}

fn is_setjmp_name(name: &str) -> bool {
    matches!(name, "setjmp" | "_setjmp" | "__sigsetjmp" | "llvm.setjmp")
}

fn is_longjmp_name(name: &str) -> bool {
    matches!(name, "longjmp" | "_longjmp" | "__siglongjmp" | "llvm.longjmp")
}

/// Assign parameters to local variables when their names are in `depends`.
/// Matches Python's `assignParameters` behavior.
fn assign_parameters(
    params: &[Variable],
    param_sizes: &[usize],
    depends: &HashSet<String>,
    use_branch_jump_table: bool,
) -> Result<BlockList, CompException> {
    // With branch jump tables, all code lives in a single procedure so
    // parameters are already in scope and never need to be reassigned.
    if use_branch_jump_table {
        return Ok(BlockList::new());
    }
    let mut blocks = BlockList::new();
    for (param, size) in params.iter().zip(param_sizes.iter()) {
        let mut var = param.clone();
        var.var_type = VarType::Var;
        if depends.contains(&var.var_name) {
            if *size == 1 {
                blocks.add_block(var.set_value(param.get_value(None), VarOp::Set, None)?);
            } else {
                blocks.add(var.set_inferred_value(InferredValue::Indexed(param.get_all_values(*size)))?);
            }
        }
    }
    Ok(blocks)
}

fn localize_func_ptr_sig(signature_id: usize) -> String {
    format!("!fn pointer signature:{}", signature_id)
}

fn localize_func_ptr_sig_callback(signature_id: usize) -> String {
    format!("{}:callback", localize_func_ptr_sig(signature_id))
}

fn get_func_ptr_addr(ptr: &str, ctx: &Context) -> Result<usize, CompException> {
    let mut addr = ctx.min_func_ptr_addr;
    for (_, ptrs) in &ctx.fn_ptr_sigs {
        if let Some(pos) = ptrs.iter().position(|p| p == ptr) {
            return Ok(addr + pos);
        }
        addr += ptrs.len();
    }
    Err(CompException(format!("Could not find function pointer for {}", ptr)))
}

fn get_value_func_ptr_refs(value: &ir::Value) -> HashSet<String> {
    let mut refs = HashSet::new();
    match value {
        ir::Value::Function(fv) => {
            refs.insert(fv.name.clone());
        }
        ir::Value::KnownVec(kv) => {
            for v in &kv.values {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Value::KnownArr(kv) => {
            for v in &kv.values {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Value::KnownStruct(kv) => {
            for v in &kv.values {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Value::ConstExpr(ce) => {
            refs.extend(get_const_expr_func_ptr_refs(&ce.expr));
        }
        _ => {}
    }
    refs
}

fn get_const_expr_func_ptr_refs(expr: &crate::ir::values::ConstExpr) -> HashSet<String> {
    let mut refs = HashSet::new();
    match expr {
        crate::ir::values::ConstExpr::Conversion(c) => {
            refs.extend(get_value_func_ptr_refs(&c.value));
        }
        crate::ir::values::ConstExpr::GetElementPtr(g) => {
            refs.extend(get_value_func_ptr_refs(&g.base_ptr));
            for idx in &g.indices {
                refs.extend(get_value_func_ptr_refs(idx));
            }
        }
        crate::ir::values::ConstExpr::ExtractElement(e) => {
            refs.extend(get_value_func_ptr_refs(&e.agg));
            refs.extend(get_value_func_ptr_refs(&e.index));
        }
        crate::ir::values::ConstExpr::InsertElement(e) => {
            refs.extend(get_value_func_ptr_refs(&e.agg));
            refs.extend(get_value_func_ptr_refs(&e.index));
            refs.extend(get_value_func_ptr_refs(&e.item));
        }
        crate::ir::values::ConstExpr::ShuffleVector(s) => {
            refs.extend(get_value_func_ptr_refs(&s.fst_vector));
            refs.extend(get_value_func_ptr_refs(&s.snd_vector));
            refs.extend(get_value_func_ptr_refs(&s.mask_vector));
        }
        crate::ir::values::ConstExpr::BinaryOp(b) => {
            refs.extend(get_value_func_ptr_refs(&b.left));
            refs.extend(get_value_func_ptr_refs(&b.right));
        }
    }
    refs
}

fn get_instr_func_ptr_refs(instr: &ir::Instr) -> HashSet<String> {
    let mut refs = HashSet::new();
    match instr {
        ir::Instr::Call(c) => {
            // Only collect indirect calls as function pointer references;
            // direct calls should not create a dispatcher procedure.
            if !matches!(c.func, ir::Value::Function(_)) {
                refs.extend(get_value_func_ptr_refs(&c.func));
            }
            for arg in &c.args {
                refs.extend(get_value_func_ptr_refs(arg));
            }
        }
        ir::Instr::Ret(r) => {
            if let Some(v) = &r.value {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Instr::Store(s) => {
            refs.extend(get_value_func_ptr_refs(&s.value));
            refs.extend(get_value_func_ptr_refs(&s.address));
        }
        ir::Instr::UnaryOp(u) => refs.extend(get_value_func_ptr_refs(&u.operand)),
        ir::Instr::BinaryOp(b) => {
            refs.extend(get_value_func_ptr_refs(&b.left));
            refs.extend(get_value_func_ptr_refs(&b.right));
        }
        ir::Instr::Conversion(c) => refs.extend(get_value_func_ptr_refs(&c.value)),
        ir::Instr::ICmp(i) => {
            refs.extend(get_value_func_ptr_refs(&i.left));
            refs.extend(get_value_func_ptr_refs(&i.right));
        }
        ir::Instr::FCmp(i) => {
            refs.extend(get_value_func_ptr_refs(&i.left));
            refs.extend(get_value_func_ptr_refs(&i.right));
        }
        ir::Instr::Select(s) => {
            refs.extend(get_value_func_ptr_refs(&s.cond));
            refs.extend(get_value_func_ptr_refs(&s.true_value));
            refs.extend(get_value_func_ptr_refs(&s.false_value));
        }
        ir::Instr::GetElementPtr(g) => {
            refs.extend(get_value_func_ptr_refs(&g.base_ptr));
            for idx in &g.indices {
                refs.extend(get_value_func_ptr_refs(idx));
            }
        }
        ir::Instr::Load(l) => refs.extend(get_value_func_ptr_refs(&l.address)),
        ir::Instr::ExtractElement(e) => {
            refs.extend(get_value_func_ptr_refs(&e.agg));
            refs.extend(get_value_func_ptr_refs(&e.index));
        }
        ir::Instr::InsertElement(e) => {
            refs.extend(get_value_func_ptr_refs(&e.agg));
            refs.extend(get_value_func_ptr_refs(&e.index));
            refs.extend(get_value_func_ptr_refs(&e.item));
        }
        ir::Instr::ShuffleVector(s) => {
            refs.extend(get_value_func_ptr_refs(&s.fst_vector));
            refs.extend(get_value_func_ptr_refs(&s.snd_vector));
            refs.extend(get_value_func_ptr_refs(&s.mask_vector));
        }
        ir::Instr::ExtractValue(e) => {
            refs.extend(get_value_func_ptr_refs(&e.agg));
        }
        ir::Instr::InsertValue(i) => {
            refs.extend(get_value_func_ptr_refs(&i.agg));
            refs.extend(get_value_func_ptr_refs(&i.element));
        }
        ir::Instr::Phi(p) => {
            for (v, _) in &p.incoming {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Instr::CondBr(c) => refs.extend(get_value_func_ptr_refs(&c.cond)),
        ir::Instr::Switch(s) => {
            refs.extend(get_value_func_ptr_refs(&s.cond));
            for (v, _) in &s.branch_table {
                refs.extend(get_value_func_ptr_refs(v));
            }
        }
        ir::Instr::Alloca(_) | ir::Instr::UncondBr(_) | ir::Instr::Unreachable |
        ir::Instr::Freeze(_) | ir::Instr::VaArg(_) => {}
    }
    refs
}

fn collect_fn_ptr_sigs(mod_: &DecodedModule) -> Vec<(FuncTy, Vec<String>)> {
    let mut all_refs: HashSet<String> = HashSet::new();

    for gv in mod_.global_vars.values() {
        all_refs.extend(get_value_func_ptr_refs(&gv.init));
    }

    for (_, func) in &mod_.functions {
        for (_, block) in &func.blocks {
            for instr in &block.instrs {
                all_refs.extend(get_instr_func_ptr_refs(instr));
            }
        }
    }

    let mut sorted_refs: Vec<String> = all_refs.into_iter().collect();
    sorted_refs.sort();

    let mut func_ptrs: Vec<(FuncTy, Vec<String>)> = Vec::new();
    for fn_name in sorted_refs {
        // Intrinsics are handled directly by trans_intrinsic, not via function pointers.
        if fn_name.starts_with("llvm.") {
            continue;
        }
        if let Some(func) = mod_.functions.get(&fn_name) {
            let signature = FuncTy::new(
                func.return_type.clone(),
                func.params.iter().map(|p| p.type_.clone()).collect(),
                func.variadic,
            );
            if let Some(pos) = func_ptrs.iter().position(|(sig, _)| *sig == signature) {
                func_ptrs[pos].1.push(fn_name);
            } else {
                func_ptrs.push((signature, vec![fn_name]));
            }
        }
    }

    // Also register signatures used by indirect call sites that do not refer
    // to a specific function (e.g. calling a function-pointer argument).
    for (_, func) in &mod_.functions {
        for (_, block) in &func.blocks {
            for instr in &block.instrs {
                if let ir::Instr::Call(call) = instr {
                    if !matches!(call.func, ir::Value::Function(_)) {
                        let signature = FuncTy::new(
                            call.return_type.clone(),
                            call.params.clone(),
                            call.variadic,
                        );
                        if !func_ptrs.iter().any(|(sig, _)| *sig == signature) {
                            func_ptrs.push((signature, Vec::new()));
                        }
                    }
                }
            }
        }
    }

    func_ptrs
}

fn get_func_ptr_signature_info<'a>(signature: &FuncTy, ctx: &'a Context) -> Option<(usize, &'a Vec<String>)> {
    for (signature_id, (sig, ptrs)) in ctx.fn_ptr_sigs.iter().enumerate() {
        if sig == signature {
            return Some((signature_id, ptrs));
        }
    }
    None
}

pub fn gen_temp_var(ctx: &mut Context) -> String {
    let id = ctx.uid_counter;
    ctx.uid_counter += 1;
    format!("{}{}", ctx.cfg.tmp_prefix, id)
}

pub fn trans_load(
    result: &Variable,
    address: Value,
    loaded_type: &ir::Type,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    // TODO FIX: properly skip over padding bytes
    if ctx.cfg.accurate_byte_spacing && matches!(loaded_type, ir::Type::Array(_) | ir::Type::Struct(_)) {
        return Err(CompException(format!(
            "Loading aggregates with accurate padding not supported yet"
        )));
    }

    let size = memory::get_size_of(loaded_type, false)?;
    let mut blocks = BlockList::new();
    let mem_var = ctx.cfg.mem_var.clone();

    if size == 1 {
        let load_val = scratch::Value::GetOfList(scratch::ast::GetOfList {
            op: scratch::ast::ListOp::AtIndex,
            name: mem_var,
            value: Box::new(address),
        });
        blocks.add_block(result.set_value(load_val, VarOp::Set, None)?);
    } else {
        let vals: Vec<Value> = (0..size).map(|i| {
            let addr = Value::Op(Op::Add(
                Box::new(address.clone()),
                Box::new(Value::Known(KnownVal::Num(i as f64))),
            ));
            scratch::Value::GetOfList(scratch::ast::GetOfList {
                op: scratch::ast::ListOp::AtIndex,
                name: mem_var.clone(),
                value: Box::new(addr),
            })
        }).collect();
        blocks.add(result.set_inferred_value(InferredValue::Indexed(IdxbleValue { vals }))?);
    }

    Ok(blocks)
}

pub fn trans_store(
    value: InferredValue,
    address: Value,
    stored_type: &ir::Type,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    // TODO FIX: properly skip over padding bytes when storing
    if ctx.cfg.accurate_byte_spacing && matches!(stored_type, ir::Type::Array(_) | ir::Type::Struct(_)) {
        return Err(CompException(format!(
            "Storing aggregates with accurate padding not supported yet"
        )));
    }

    let mut blocks = BlockList::new();
    let mem_var = ctx.cfg.mem_var.clone();

    match value {
        InferredValue::Single(v) => {
            blocks.add_block(Block::EditList(scratch::ast::EditListData {
                op: scratch::ast::ListEditOp::ReplaceAt,
                name: mem_var,
                index: Some(address.clone()),
                value: Some(v),
            }));
        }
        InferredValue::Indexed(iv) => {
            for (offset, val) in iv.vals.iter().enumerate() {
                let addr = Value::Op(Op::Add(
                    Box::new(address.clone()),
                    Box::new(Value::Known(KnownVal::Num(offset as f64))),
                ));
                blocks.add_block(Block::EditList(scratch::ast::EditListData {
                    op: scratch::ast::ListEditOp::ReplaceAt,
                    name: mem_var.clone(),
                    index: Some(addr),
                    value: Some(val.clone()),
                }));
            }
        }
    }

    Ok(blocks)
}

pub fn store_on_stack(
    stack_var: &str,
    stack_size_var: &str,
    offset: i64,
    size: usize,
    value: &Value,
) -> BlockList {
    let mut blocks = BlockList::new();
    for i in 0..size {
        let item_offset = offset + i as i64;
        let addr = if item_offset == 0 {
            Value::GetVar { name: stack_size_var.to_string() }
        } else if item_offset > 0 {
            Value::Op(Op::Add(
                Box::new(Value::GetVar { name: stack_size_var.to_string() }),
                Box::new(Value::Known(KnownVal::Num(item_offset as f64))),
            ))
        } else {
            Value::Op(Op::Sub(
                Box::new(Value::GetVar { name: stack_size_var.to_string() }),
                Box::new(Value::Known(KnownVal::Num((-item_offset) as f64))),
            ))
        };
        blocks.add_block(Block::EditList(scratch::ast::EditListData {
            op: scratch::ast::ListEditOp::ReplaceAt,
            name: stack_var.to_string(),
            index: Some(addr),
            value: if i == 0 {
                Some(value.clone())
            } else {
                Some(Value::Known(KnownVal::Num(0.0)))
            },
        }));
    }
    blocks
}

pub fn load_from_stack(
    stack_var: &str,
    stack_size_var: &str,
    offset: i64,
) -> Value {
    let addr = if offset == 0 {
        Value::GetVar { name: stack_size_var.to_string() }
    } else if offset > 0 {
        Value::Op(Op::Add(
            Box::new(Value::GetVar { name: stack_size_var.to_string() }),
            Box::new(Value::Known(KnownVal::Num(offset as f64))),
        ))
    } else {
        Value::Op(Op::Sub(
            Box::new(Value::GetVar { name: stack_size_var.to_string() }),
            Box::new(Value::Known(KnownVal::Num((-offset) as f64))),
        ))
    };
    scratch::Value::GetOfList(scratch::ast::GetOfList {
        op: scratch::ast::ListOp::AtIndex,
        name: stack_var.to_string(),
        value: Box::new(addr),
    })
}

/// Serialize a compile-time constant value into the byte-oriented initial
/// memory list.  The result is padded with zeros to `expected_size` to match
/// Python's `padValue` behavior for global initializers.
fn value_to_init_mem(
    val: &ir::Value,
    expected_size: usize,
    ctx: &mut Context,
) -> Result<Vec<KnownVal>, CompException> {
    match val {
        ir::Value::KnownInt(ki) => {
            let mut vals = Vec::new();
            if ki.width as usize <= super::config::VARIABLE_MAX_BITS {
                vals.push(KnownVal::Num(ki.value.to_u64().unwrap_or(0) as f64));
            } else {
                let mut num = ki.value.clone();
                let mut width = ki.width as usize;
                let part_mask = BigUint::from(1u32) << super::config::VARIABLE_MAX_BITS;
                while width > 0 {
                    let part = &num % &part_mask;
                    vals.push(KnownVal::Num(part.to_u64().unwrap_or(0) as f64));
                    num >>= super::config::VARIABLE_MAX_BITS;
                    width = width.saturating_sub(super::config::VARIABLE_MAX_BITS);
                }
            }
            while vals.len() < expected_size {
                vals.push(KnownVal::Num(0.0));
            }
            Ok(vals)
        }
        ir::Value::KnownFloat(kf) => {
            let mut vals = vec![KnownVal::Num(kf.value)];
            while vals.len() < expected_size {
                vals.push(KnownVal::Num(0.0));
            }
            Ok(vals)
        }
        ir::Value::KnownArr(arr) => {
            let mut vals = Vec::new();
            for elem in &arr.values {
                let elem_size = memory::get_size_of(elem.type_(), true).unwrap_or(1);
                vals.extend(value_to_init_mem(elem, elem_size, ctx)?);
            }
            while vals.len() < expected_size {
                vals.push(KnownVal::Num(0.0));
            }
            Ok(vals)
        }
        ir::Value::KnownStruct(struc) => {
            let mut vals = Vec::new();
            for elem in &struc.values {
                let elem_size = memory::get_size_of(elem.type_(), true).unwrap_or(1);
                vals.extend(value_to_init_mem(elem, elem_size, ctx)?);
            }
            while vals.len() < expected_size {
                vals.push(KnownVal::Num(0.0));
            }
            Ok(vals)
        }
        ir::Value::KnownVec(kv) => {
            let mut vals = Vec::new();
            for elem in &kv.values {
                let elem_size = memory::get_size_of(elem.type_(), true).unwrap_or(1);
                vals.extend(value_to_init_mem(elem, elem_size, ctx)?);
            }
            while vals.len() < expected_size {
                vals.push(KnownVal::Num(0.0));
            }
            Ok(vals)
        }
        ir::Value::NullPtr(_) | ir::Value::Undef(_) => {
            Ok((0..expected_size).map(|_| KnownVal::Num(0.0)).collect())
        }
        ir::Value::GlobalPtr(gp) => {
            if let Some(&ptr) = ctx.globvar_to_ptr.get(&gp.name) {
                let mut vals = vec![KnownVal::Num(ptr as f64)];
                while vals.len() < expected_size {
                    vals.push(KnownVal::Num(0.0));
                }
                Ok(vals)
            } else {
                Err(CompException(format!(
                    "Global variable not found: {}",
                    gp.name
                )))
            }
        }
        ir::Value::Function(fv) => {
            if ctx.fn_ptr_sigs.iter().any(|(_, ptrs)| ptrs.contains(&fv.name)) {
                let ptr_addr = get_func_ptr_addr(&fv.name, ctx)?;
                let mut vals = vec![KnownVal::Num(ptr_addr as f64)];
                while vals.len() < expected_size {
                    vals.push(KnownVal::Num(0.0));
                }
                Ok(vals)
            } else if let Some(info) = ctx.fn_info.get(&fv.name) {
                let mut vals = vec![KnownVal::Num(info.fn_id as f64)];
                while vals.len() < expected_size {
                    vals.push(KnownVal::Num(0.0));
                }
                Ok(vals)
            } else {
                Err(CompException(format!("Function not found: {}", fv.name)))
            }
        }
        ir::Value::ConstExpr(ce) => match &ce.expr {
            ir::values::ConstExpr::Conversion(c) => {
                value_to_init_mem(&c.value, expected_size, ctx)
            }
            ir::values::ConstExpr::GetElementPtr(gep) => {
                let val = trans_gep_value(
                    &gep.base_ptr,
                    &gep.base_ptr_type,
                    &gep.indices,
                    gep.is_nuw,
                    ctx,
                    None,
                    true,
                )?;
                match val {
                    Value::Known(KnownVal::Num(n)) => {
                        let mut vals = vec![KnownVal::Num(n)];
                        while vals.len() < expected_size {
                            vals.push(KnownVal::Num(0.0));
                        }
                        Ok(vals)
                    }
                    _ => Ok((0..expected_size)
                        .map(|_| KnownVal::Num(0.0))
                        .collect()),
                }
            }
            _ => Ok((0..expected_size)
                .map(|_| KnownVal::Num(0.0))
                .collect()),
        },
        _ => Ok((0..expected_size)
            .map(|_| KnownVal::Num(0.0))
            .collect()),
    }
}

pub fn init_memory(
    mod_: &DecodedModule,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();

    let include_padding = ctx.cfg.accurate_byte_spacing;
    let sizes: Vec<usize> = mod_.global_vars
        .values()
        .map(|gv| memory::get_size_of(&gv.type_, include_padding))
        .collect::<Result<Vec<_>, _>>()?;
    let total_size: usize = sizes.iter().sum();

    let starting_global_addr: usize = 1;
    let starting_heap_ptr = starting_global_addr + total_size;
    let starting_stack_ptr = ctx.cfg.memory_size;
    let starting_fn_ptr_addr = ctx.cfg.memory_size + 1;

    let mut ptr = starting_global_addr;
    for (i, (_, gv)) in mod_.global_vars.iter().enumerate() {
        ctx.globvar_to_ptr.insert(gv.name.clone(), ptr);
        ptr += sizes[i];
    }

    ctx.min_func_ptr_addr = starting_fn_ptr_addr;

    // Preseed stack handling: skip clearing/filling memory and global
    // initializers; just set the stack pointer (and heap pointer) and exit.
    if ctx.cfg.preseed_stack {
        let init_ptr = if ctx.cfg.preseed_stack_ptr == 0 {
            starting_heap_ptr
        } else {
            ctx.cfg.preseed_stack_ptr
        };
        blocks.add_block(Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: ctx.cfg.stack_pointer_var.clone(),
            value: Value::Known(KnownVal::Num(init_ptr as f64)),
        }));
        blocks.add_block(Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: ctx.cfg.heap_pointer_var.clone(),
            value: Value::Known(KnownVal::Num(starting_heap_ptr as f64)),
        }));
        return Ok(blocks);
    }

    let mut init_mem: Vec<KnownVal> = Vec::new();
    ptr = starting_global_addr;
    for (i, (_, gv)) in mod_.global_vars.iter().enumerate() {
        if !ctx.cfg.compiler_opt {
            let globvar = Variable {
                var_name: gv.name.clone(),
                var_type: VarType::Global,
                fn_name: None,
            };
            blocks.add_block(globvar.set_value(
                Value::Known(KnownVal::Num(ptr as f64)),
                VarOp::Set,
                None,
            )?);
        }

        init_mem.extend(value_to_init_mem(&gv.init, sizes[i], ctx)?);

        ptr += sizes[i];
    }

    ctx.proj.lists.insert(ctx.cfg.init_mem_var.clone(), init_mem);

    let mem_var = ctx.cfg.mem_var.clone();
    let init_mem_var = ctx.cfg.init_mem_var.clone();
    let memory_size = ctx.cfg.memory_size;
    let effective_mem_size = memory_size.min(super::config::SCRATCH_LIST_LIMIT);

    let list_is_saturated = Value::BoolOp(BoolOp::Eq(
        Box::new(Value::GetListLength { name: mem_var.clone() }),
        Box::new(Value::Known(KnownVal::Num(effective_mem_size as f64))),
    ));

    let saturate_body = BlockList::from_blocks(vec![
        Block::EditList(EditListData {
            op: ListEditOp::DeleteAll,
            name: mem_var.clone(),
            value: None,
            index: None,
        }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::RepTimes,
            condition: Some(Value::Known(KnownVal::Num(memory_size as f64))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditList(EditListData {
                    op: ListEditOp::AddTo,
                    name: mem_var.clone(),
                    value: Some(Value::Known(KnownVal::Num(0.0))),
                    index: None,
                }),
            ])),
            else_body: None,
        }),
    ]);

    let for_each_body = BlockList::from_blocks(vec![
        Block::EditList(EditListData {
            op: ListEditOp::ReplaceAt,
            name: mem_var.clone(),
            index: Some(Value::Op(Op::Add(
                Box::new(Value::Known(KnownVal::Num((starting_global_addr - 1) as f64))),
                Box::new(Value::GetVar { name: "ptr".to_string() }),
            ))),
            value: Some(Value::GetOfList(GetOfList {
                op: ListOp::AtIndex,
                name: init_mem_var,
                value: Box::new(Value::GetVar { name: "ptr".to_string() }),
            })),
        }),
    ]);

    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.stack_pointer_var.clone(),
        value: Value::Known(KnownVal::Num(starting_stack_ptr as f64)),
    }));

    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.heap_pointer_var.clone(),
        value: Value::Known(KnownVal::Num(starting_heap_ptr as f64)),
    }));

    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::If,
        condition: Some(Value::BoolOp(BoolOp::Not(Box::new(list_is_saturated)))),
        var: None,
        body: Some(saturate_body),
        else_body: None,
    }));

    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::ForEach,
        condition: Some(Value::Known(KnownVal::Num(total_size as f64))),
        var: Some("ptr".to_string()),
        body: Some(for_each_body),
        else_body: None,
    }));

    Ok(blocks)
}

pub fn init_local_stack(ctx: &Context) -> BlockList {
    let mut blocks = BlockList::new();

    let local_stack_var = ctx.cfg.local_stack_var.clone();
    let local_stack_size = ctx.cfg.local_stack_size;

    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.local_stack_size_var.clone(),
        value: Value::Known(KnownVal::Num(0.0)),
    }));

    let list_is_saturated = Value::BoolOp(BoolOp::Eq(
        Box::new(Value::GetListLength { name: local_stack_var.clone() }),
        Box::new(Value::Known(KnownVal::Num(local_stack_size as f64))),
    ));

    let saturate_body = BlockList::from_blocks(vec![
        Block::EditList(EditListData {
            op: ListEditOp::DeleteAll,
            name: local_stack_var.clone(),
            value: None,
            index: None,
        }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::RepTimes,
            condition: Some(Value::Known(KnownVal::Num(local_stack_size as f64))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditList(EditListData {
                    op: ListEditOp::AddTo,
                    name: local_stack_var.clone(),
                    value: Some(Value::Known(KnownVal::Num(0.0))),
                    index: None,
                }),
            ])),
            else_body: None,
        }),
    ]);

    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::If,
        condition: Some(Value::BoolOp(BoolOp::Not(Box::new(list_is_saturated)))),
        var: None,
        body: Some(saturate_body),
        else_body: None,
    }));

    blocks
}

pub fn compile(
    llvm: &str,
    cfg: Option<CompilerConfig>,
) -> Result<(Project, Option<usize>, HashMap<String, crate::ir::instructions::Function>), CompException> {
    let cfg = cfg.unwrap_or_default();
    let proj = Project::new(cfg.scratch_config.clone());
    let mut ctx = Context::new(proj, cfg);

    let mod_ = crate::parser::parse_assembly(llvm, false)
        .map_err(|e| CompException(format!("Parse error: {:?}", e)))?;

    for (name, func) in &mod_.functions {
        ctx.functions.insert(name.clone(), func.clone());
    }

    ctx.fn_ptr_sigs = collect_fn_ptr_sigs(&mod_);

    ctx = add_foreign_functions(ctx);

    ctx.proj.code.push(BlockList::from_blocks(vec![
        Block::OnStartFlag,
        Block::ProcedureCall(scratch::ast::ProcedureCallData {
            name: "!init".to_string(),
            args: Vec::new(),
            run_without_refresh: false,
        }),
    ]));

    let mut init_blocks = BlockList::from_blocks(vec![
        Block::ProcedureDef(scratch::ast::ProcedureDefData {
            name: "!init".to_string(),
            params: Vec::new(),
            warp: true,
        }),
    ]);

    let init_mem = init_memory(&mod_, &mut ctx)?;
    init_blocks.add(init_mem);
    init_blocks.add(init_local_stack(&ctx));

    ctx = trans_funcs(&mod_, ctx)?;
    maybe_add_str2color_helper(&mut ctx);
    trans_func_ptr_sigs(&mut ctx)?;

    let init_lookups = init_lookup_tables(&mut ctx)?;
    init_blocks.add(init_lookups);

    let start_blocks = trans_entrypoint_call(&mut ctx)?;
    init_blocks.add(start_blocks);

    // Note: post_opt_transform is NOT called here. It is called by the caller
    // (main.rs) AFTER the first optimization pass, to match Python's pipeline:
    // transFuncs -> optimize -> postOptTransform -> optimize
    ctx.proj.code.push(init_blocks);

    Ok((ctx.proj, ctx.highest_return_size, ctx.functions))
}

fn add_foreign_functions(mut ctx: Context) -> Context {
    let return_var = ctx.cfg.return_var.clone();
    let mem_var = ctx.cfg.mem_var.clone();
    let heap_pointer_var = ctx.cfg.heap_pointer_var.clone();
    let ascii_lookup = format!("{}{}", ctx.cfg.ascii_lookup_var, ctx.cfg.zero_indexed_suffix);

    let uppercase_costume_name = "uppercase";
    let lowercase_str = "abcdefghijklmnopqrstuvwxyz";
    ctx.proj.lists.insert(ctx.cfg.lowercase_var.clone(), lowercase_str.chars().map(|c| KnownVal::Str(c.to_string())).collect());
    ctx.proj.add_costume(uppercase_costume_name.to_string());
    let lc_costume_num = ctx.proj.add_costume(lowercase_str.to_string());

    let mut ascii_lookup_vals: Vec<KnownVal> = Vec::new();
    for x in 1u32..256 {
        let c = char::from_u32(x).unwrap_or(char::REPLACEMENT_CHARACTER);
        let s = c.to_string();
        // Match Python's unicode_escape behavior: escape control characters
        // (1-31, 127) and high bytes (128-255), but keep backslash literal.
        if (x <= 31 || x >= 127) && x != 92 {
            ascii_lookup_vals.push(KnownVal::Str(format!("\\{:02X}", x)));
        } else {
            ascii_lookup_vals.push(KnownVal::Str(s));
        }
    }
    ctx.proj.lists.insert(ascii_lookup.clone(), ascii_lookup_vals);
    let add_func = |name: &str, params: Vec<&str>, contents: BlockList, ctx: &mut Context| {
        let localized_params: Vec<Variable> = params.iter().map(|p| Variable {
            var_name: p.to_string(),
            var_type: VarType::Param,
            fn_name: Some(name.to_string()),
        }).collect();
        let raw_param_names: Vec<String> = localized_params
            .iter()
            .map(|p| p.get_raw_var_name(None))
            .collect();
        let mut blocks = BlockList::from_blocks(vec![
            Block::ProcedureDef(ProcedureDefData {
                name: name.to_string(),
                params: raw_param_names,
                warp: true,
            }),
        ]);
        blocks.add(contents);
        ctx.proj.code.push(blocks);
        let param_sizes: Vec<usize> = vec![1; params.len()];
        ctx.fn_info.insert(name.to_string(), FuncInfo::new(
            name.to_string(), ctx.next_fn_id, localized_params, param_sizes, params.len(),
        ));
        ctx.next_fn_id += 1;
    };

    let rv = return_var.clone();
    add_func("!helper_exact_pow2i", vec!["exp", "exp_bits"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: "remaining".into(), value: Value::Op(Op::Abs(Box::new(Value::GetParam { name: "exp".into() }))) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "current_multiplier".into(), value: Value::Known(KnownVal::Num(2.0)) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Known(KnownVal::Num(1.0)) }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::RepTimes,
            condition: Some(Value::GetParam { name: "exp_bits".into() }),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(Value::BoolOp(BoolOp::Eq(
                        Box::new(Value::Op(Op::Mod(Box::new(Value::GetVar { name: "remaining".into() }), Box::new(Value::Known(KnownVal::Num(2.0)))))),
                        Box::new(Value::Known(KnownVal::Num(1.0))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::Mul(Box::new(Value::GetVar { name: rv.clone() }), Box::new(Value::GetVar { name: "current_multiplier".into() }))) }),
                    ])),
                    else_body: None,
                }),
                Block::EditVar(EditVarData { op: VarOp::Set, name: "remaining".into(), value: Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(Box::new(Value::GetVar { name: "remaining".into() }), Box::new(Value::Known(KnownVal::Num(2.0)))))))) }),
                Block::EditVar(EditVarData { op: VarOp::Set, name: "current_multiplier".into(), value: Value::Op(Op::Mul(Box::new(Value::GetVar { name: "current_multiplier".into() }), Box::new(Value::GetVar { name: "current_multiplier".into() }))) }),
            ])),
            else_body: None,
        }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::BoolOp(BoolOp::Lt(Box::new(Value::GetParam { name: "exp".into() }), Box::new(Value::Known(KnownVal::Num(0.0)))))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::Div(Box::new(Value::Known(KnownVal::Num(1.0))), Box::new(Value::GetVar { name: rv.clone() }))) }),
            ])),
            else_body: None,
        }),
    ]), &mut ctx);

    add_func("!helper_IEEE_754", vec!["float", "exp_bits", "max_exp", "2^mant_bits"], BlockList::from_blocks(vec![
        Block::ControlFlow(ControlFlow {
            op: ControlOp::IfElse,
            condition: Some(Value::BoolOp(BoolOp::Lt(
                Box::new(Value::Op(Op::Abs(Box::new(Value::GetParam { name: "float".into() })))),
                Box::new(Value::Known(KnownVal::Num(f64::INFINITY))),
            ))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: format!("{}:0", rv.clone()),
                    value: Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Lt(
                        Box::new(Value::Op(Op::Div(
                            Box::new(Value::Known(KnownVal::Num(1.0))),
                            Box::new(Value::GetParam { name: "float".into() }),
                        ))),
                        Box::new(Value::Known(KnownVal::Num(0.0))),
                    ))))),
                }),
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(Value::BoolOp(BoolOp::Eq(
                        Box::new(Value::GetParam { name: "float".into() }),
                        Box::new(Value::Known(KnownVal::Num(f64::NAN))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::EditVar(EditVarData {
                            op: VarOp::Set,
                            name: "exponent".into(),
                            value: Value::Op(Op::Add(
                                Box::new(Value::GetParam { name: "max_exp".into() }),
                                Box::new(Value::Known(KnownVal::Num(1.0))),
                            )),
                        }),
                        Block::EditVar(EditVarData {
                            op: VarOp::Set,
                            name: format!("{}:2", rv.clone()),
                            value: Value::Op(Op::Div(
                                Box::new(Value::GetParam { name: "2^mant_bits".into() }),
                                Box::new(Value::Known(KnownVal::Num(2.0))),
                            )),
                        }),
                    ])),
                    else_body: Some(BlockList::from_blocks(vec![
                        Block::ControlFlow(ControlFlow {
                            op: ControlOp::IfElse,
                            condition: Some(Value::BoolOp(BoolOp::Eq(
                                Box::new(Value::GetParam { name: "float".into() }),
                                Box::new(Value::Known(KnownVal::Num(0.0))),
                            ))),
                            var: None,
                            body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: "exponent".into(),
                                    value: Value::Op(Op::Sub(
                                        Box::new(Value::Known(KnownVal::Num(0.0))),
                                        Box::new(Value::GetParam { name: "max_exp".into() }),
                                    )),
                                }),
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: format!("{}:2", rv.clone()),
                                    value: Value::Known(KnownVal::Num(0.0)),
                                }),
                            ])),
                            else_body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: "exponent".into(),
                                    value: {
                                        let abs_float = Value::Op(Op::Abs(Box::new(Value::GetParam { name: "float".into() })));
                                        let ln_abs = Value::Op(Op::Ln(Box::new(abs_float)));
                                        let div_ln = Value::Op(Op::Div(
                                            Box::new(ln_abs),
                                            Box::new(Value::Known(KnownVal::Num(2f64.ln()))),
                                        ));
                                        let sub = Value::Op(Op::Sub(
                                            Box::new(div_ln),
                                            Box::new(Value::Known(KnownVal::Num(0.5))),
                                        ));
                                        Value::Op(Op::Floor(Box::new(sub)))
                                    },
                                }),
                                Block::ProcedureCall(ProcedureCallData {
                                    name: "!helper_exact_pow2i".into(),
                                    args: vec![
                                        Value::Op(Op::Add(
                                            Box::new(Value::GetVar { name: "exponent".into() }),
                                            Box::new(Value::Known(KnownVal::Num(1.0))),
                                        )),
                                        Value::GetParam { name: "exp_bits".into() },
                                    ],
                                    run_without_refresh: false,
                                }),
                                Block::ControlFlow(ControlFlow {
                                    op: ControlOp::IfElse,
                                    condition: Some(Value::BoolOp(BoolOp::Lt(
                                        Box::new(Value::Op(Op::Abs(Box::new(Value::GetParam { name: "float".into() })))),
                                        Box::new(Value::GetVar { name: rv.clone() }),
                                    ))),
                                    var: None,
                                    body: Some(BlockList::from_blocks(vec![
                                        Block::EditVar(EditVarData {
                                            op: VarOp::Set,
                                            name: "2^exponent".into(),
                                            value: Value::Op(Op::Div(
                                                Box::new(Value::GetVar { name: rv.clone() }),
                                                Box::new(Value::Known(KnownVal::Num(2.0))),
                                            )),
                                        }),
                                    ])),
                                    else_body: Some(BlockList::from_blocks(vec![
                                        Block::EditVar(EditVarData {
                                            op: VarOp::Change,
                                            name: "exponent".into(),
                                            value: Value::Known(KnownVal::Num(1.0)),
                                        }),
                                        Block::EditVar(EditVarData {
                                            op: VarOp::Set,
                                            name: "2^exponent".into(),
                                            value: Value::GetVar { name: rv.clone() },
                                        }),
                                    ])),
                                }),
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: format!("{}:2", rv.clone()),
                                    value: {
                                        let abs_float = Value::Op(Op::Abs(Box::new(Value::GetParam { name: "float".into() })));
                                        let div = Value::Op(Op::Div(
                                            Box::new(abs_float),
                                            Box::new(Value::GetVar { name: "2^exponent".into() }),
                                        ));
                                        let sub = Value::Op(Op::Sub(
                                            Box::new(div),
                                            Box::new(Value::Known(KnownVal::Num(1.0))),
                                        ));
                                        let mul = Value::Op(Op::Mul(
                                            Box::new(sub),
                                            Box::new(Value::GetParam { name: "2^mant_bits".into() }),
                                        ));
                                        Value::Op(Op::Round(Box::new(mul)))
                                    },
                                }),
                            ])),
                        }),
                    ])),
                }),
            ])),
            else_body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: format!("{}:0", rv.clone()),
                    value: Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Lt(
                        Box::new(Value::GetParam { name: "float".into() }),
                        Box::new(Value::Known(KnownVal::Num(0.0))),
                    ))))),
                }),
                Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: "exponent".into(),
                    value: Value::Op(Op::Add(
                        Box::new(Value::GetParam { name: "max_exp".into() }),
                        Box::new(Value::Known(KnownVal::Num(1.0))),
                    )),
                }),
                Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: format!("{}:2", rv.clone()),
                    value: Value::Known(KnownVal::Num(0.0)),
                }),
            ])),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: format!("{}:1", rv.clone()),
            value: Value::Op(Op::Add(
                Box::new(Value::GetVar { name: "exponent".into() }),
                Box::new(Value::GetParam { name: "max_exp".into() }),
            )),
        }),
    ]), &mut ctx);

    /*
    add_func("!helper_i64_to_double", vec!["low", "high"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "i64_sign".into(),
            value: Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                Box::new(Value::GetParam { name: "high".into() }),
                Box::new(Value::Known(KnownVal::Num(32768.0))),
            ))))),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "i64_exp".into(),
            value: Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                Box::new(Value::Op(Op::Mod(
                    Box::new(Value::GetParam { name: "high".into() }),
                    Box::new(Value::Known(KnownVal::Num(32768.0))),
                ))),
                Box::new(Value::Known(KnownVal::Num(16.0))),
            ))))),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "i64_mant_high".into(),
            value: Value::Op(Op::Mod(
                Box::new(Value::GetParam { name: "high".into() }),
                Box::new(Value::Known(KnownVal::Num(16.0))),
            )),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "i64_mant".into(),
            value: Value::Op(Op::Add(
                Box::new(Value::GetParam { name: "low".into() }),
                Box::new(Value::Op(Op::Mul(
                    Box::new(Value::GetVar { name: "i64_mant_high".into() }),
                    Box::new(Value::Known(KnownVal::Num(281474976710656.0))),
                ))),
            )),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: rv.clone(),
            value: Value::Known(KnownVal::Num(0.0)),
        }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::BoolOp(BoolOp::Or(
                Box::new(Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Eq(
                    Box::new(Value::GetVar { name: "i64_exp".into() }),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                )))))),
                Box::new(Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Eq(
                    Box::new(Value::GetVar { name: "i64_mant".into() }),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                )))))),
            ))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(Value::BoolOp(BoolOp::Eq(
                        Box::new(Value::GetVar { name: "i64_exp".into() }),
                        Box::new(Value::Known(KnownVal::Num(2047.0))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::ControlFlow(ControlFlow {
                            op: ControlOp::IfElse,
                            condition: Some(Value::BoolOp(BoolOp::Eq(
                                Box::new(Value::GetVar { name: "i64_mant".into() }),
                                Box::new(Value::Known(KnownVal::Num(0.0))),
                            ))),
                            var: None,
                            body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: rv.clone(),
                                    value: Value::Op(Op::Mul(
                                        Box::new(Value::Op(Op::Sub(
                                            Box::new(Value::Known(KnownVal::Num(1.0))),
                                            Box::new(Value::Op(Op::Mul(
                                                Box::new(Value::GetVar { name: "i64_sign".into() }),
                                                Box::new(Value::Known(KnownVal::Num(2.0))),
                                            ))),
                                        ))),
                                        Box::new(Value::Known(KnownVal::Num(f64::INFINITY))),
                                    )),
                                }),
                            ])),
                            else_body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: rv.clone(),
                                    value: Value::Known(KnownVal::Num(f64::NAN)),
                                }),
                            ])),
                        }),
                    ])),
                    else_body: Some(BlockList::from_blocks(vec![
                        Block::ControlFlow(ControlFlow {
                            op: ControlOp::IfElse,
                            condition: Some(Value::BoolOp(BoolOp::Eq(
                                Box::new(Value::GetVar { name: "i64_exp".into() }),
                                Box::new(Value::Known(KnownVal::Num(0.0))),
                            ))),
                            var: None,
                            body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: rv.clone(),
                                    value: Value::Op(Op::Mul(
                                        Box::new(Value::Op(Op::Sub(
                                            Box::new(Value::Known(KnownVal::Num(1.0))),
                                            Box::new(Value::Op(Op::Mul(
                                                Box::new(Value::GetVar { name: "i64_sign".into() }),
                                                Box::new(Value::Known(KnownVal::Num(2.0))),
                                            ))),
                                        ))),
                                        Box::new(Value::Op(Op::Mul(
                                            Box::new(Value::GetVar { name: "i64_mant".into() }),
                                            Box::new(Value::Known(KnownVal::Num(2f64.powi(-1074)))),
                                        ))),
                                    )),
                                }),
                            ])),
                            else_body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: "i64_pow".into(),
                                    value: Value::Op(Op::Sub(
                                        Box::new(Value::GetVar { name: "i64_exp".into() }),
                                        Box::new(Value::Known(KnownVal::Num(1023.0))),
                                    )),
                                }),
                                Block::ProcedureCall(ProcedureCallData {
                                    name: "!helper_exact_pow2i".into(),
                                    args: vec![
                                        Value::GetVar { name: "i64_pow".into() },
                                        Value::Known(KnownVal::Num(11.0)),
                                    ],
                                    run_without_refresh: false,
                                }),
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: "i64_pow2".into(),
                                    value: Value::GetVar { name: rv.clone() },
                                }),
                                Block::EditVar(EditVarData {
                                    op: VarOp::Set,
                                    name: rv.clone(),
                                    value: Value::Op(Op::Mul(
                                        Box::new(Value::Op(Op::Sub(
                                            Box::new(Value::Known(KnownVal::Num(1.0))),
                                            Box::new(Value::Op(Op::Mul(
                                                Box::new(Value::GetVar { name: "i64_sign".into() }),
                                                Box::new(Value::Known(KnownVal::Num(2.0))),
                                            ))),
                                        ))),
                                        Box::new(Value::Op(Op::Mul(
                                            Box::new(Value::Op(Op::Add(
                                                Box::new(Value::Known(KnownVal::Num(1.0))),
                                                Box::new(Value::Op(Op::Div(
                                                    Box::new(Value::GetVar { name: "i64_mant".into() }),
                                                    Box::new(Value::Known(KnownVal::Num(4503599627370496.0))),
                                                ))),
                                            ))),
                                            Box::new(Value::GetVar { name: "i64_pow2".into() }),
                                        ))),
                                    )),
                                }),
                            ])),
                        }),
                    ])),
                }),
            ])),
            else_body: None,
        }),
    ]), &mut ctx);
    */

    let lowercase_var = ctx.cfg.lowercase_var.clone();
    add_func("!helper_is_lowercase", vec!["char", "alphabet_pos"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "original".into(),
            value: Value::GetOfList(GetOfList {
                op: ListOp::AtIndex,
                name: lowercase_var.clone(),
                value: Box::new(Value::GetParam { name: "alphabet_pos".into() }),
            }),
        }),
        Block::EditList(EditListData {
            op: ListEditOp::ReplaceAt,
            name: lowercase_var.clone(),
            index: Some(Value::GetParam { name: "alphabet_pos".into() }),
            value: Some(Value::GetParam { name: "char".into() }),
        }),
        Block::SwitchCostume { value: Value::Known(KnownVal::Str(uppercase_costume_name.to_string())) },
        Block::SwitchCostume { value: Value::GetList { name: lowercase_var.clone() } },
        Block::EditList(EditListData {
            op: ListEditOp::ReplaceAt,
            name: lowercase_var.clone(),
            index: Some(Value::GetParam { name: "alphabet_pos".into() }),
            value: Some(Value::GetVar { name: "original".into() }),
        }),
        Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: rv.clone(),
            value: Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Eq(
                Box::new(Value::CostumeInfo { op: CostumeInfoOp::Number }),
                Box::new(Value::Known(KnownVal::Num(lc_costume_num as f64))),
            ))))),
        }),
    ]), &mut ctx);

    add_func("!helper_str2scratch", vec!["input"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Known(KnownVal::Str("".into())) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "ptr".into(), value: Value::GetParam { name: "input".into() } }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::GetOfList(GetOfList { op: ListOp::AtIndex, name: mem_var.clone(), value: Box::new(Value::GetVar { name: "ptr".into() }) }) }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::Until,
            condition: Some(Value::BoolOp(BoolOp::Eq(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(0.0)))))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::Join(Box::new(Value::GetVar { name: rv.clone() }), Box::new(Value::GetOfList(GetOfList { op: ListOp::AtIndex, name: ascii_lookup.clone(), value: Box::new(Value::GetVar { name: "char".into() }) })))) }),
                Block::EditVar(EditVarData { op: VarOp::Change, name: "ptr".into(), value: Value::Known(KnownVal::Num(1.0)) }),
                Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::GetOfList(GetOfList { op: ListOp::AtIndex, name: mem_var.clone(), value: Box::new(Value::GetVar { name: "ptr".into() }) }) }),
            ])),
            else_body: None,
        }),
    ]), &mut ctx);

    let enough_space = Value::BoolOp(BoolOp::Lt(
        Box::new(Value::Op(Op::LengthOf(Box::new(Value::GetParam { name: "input".into() })))),
        Box::new(Value::GetParam { name: "count".into() }),
    ));

    add_func("!helper_scratch2str", vec!["input", "str", "count"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: "ptr".into(), value: Value::Op(Op::Sub(Box::new(Value::GetParam { name: "str".into() }), Box::new(Value::Known(KnownVal::Num(1.0))))) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "i".into(), value: Value::Known(KnownVal::Num(1.0)) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "cost".into(), value: Value::CostumeInfo { op: CostumeInfoOp::Number } }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::IfElse,
            condition: Some(enough_space.clone()),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::Op(Op::LengthOf(Box::new(Value::GetParam { name: "input".into() }))) }),
            ])),
            else_body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::Op(Op::Sub(Box::new(Value::GetParam { name: "count".into() }), Box::new(Value::Known(KnownVal::Num(1.0))))) }),
            ])),
        }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::RepTimes,
            condition: Some(Value::GetVar { name: "char".into() }),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: "ascii".into(), value: Value::GetOfList(GetOfList { op: ListOp::IndexOf, name: ascii_lookup.clone(), value: Box::new(Value::Op(Op::LetterOf(Box::new(Value::GetVar { name: "i".into() }), Box::new(Value::GetParam { name: "input".into() })))) }) }),
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(Value::BoolOp(BoolOp::And(
                        Box::new(Value::BoolOp(BoolOp::Gt(Box::new(Value::GetVar { name: "ascii".into() }), Box::new(Value::Known(KnownVal::Num(64.0)))))),
                        Box::new(Value::BoolOp(BoolOp::Lt(Box::new(Value::GetVar { name: "ascii".into() }), Box::new(Value::Known(KnownVal::Num(91.0)))))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::ProcedureCall(ProcedureCallData { name: "!helper_is_lowercase".into(), args: vec![
                            Value::Op(Op::LetterOf(Box::new(Value::GetVar { name: "i".into() }), Box::new(Value::GetParam { name: "input".into() }))),
                            Value::Op(Op::Sub(Box::new(Value::GetVar { name: "ascii".into() }), Box::new(Value::Known(KnownVal::Num(64.0))))),
                        ], run_without_refresh: false }),
                        Block::ControlFlow(ControlFlow {
                            op: ControlOp::If,
                            condition: Some(Value::BoolOp(BoolOp::Eq(Box::new(Value::GetVar { name: rv.clone() }), Box::new(Value::Known(KnownVal::Num(1.0)))))),
                            var: None,
                            body: Some(BlockList::from_blocks(vec![
                                Block::EditVar(EditVarData { op: VarOp::Change, name: "ascii".into(), value: Value::Known(KnownVal::Num(32.0)) }),
                            ])),
                            else_body: None,
                        }),
                    ])),
                    else_body: None,
                }),
                Block::EditList(EditListData { op: ListEditOp::ReplaceAt, name: mem_var.clone(), index: Some(Value::Op(Op::Add(Box::new(Value::GetVar { name: "ptr".into() }), Box::new(Value::GetVar { name: "i".into() })))), value: Some(Value::GetVar { name: "ascii".into() }) }),
                Block::EditVar(EditVarData { op: VarOp::Change, name: "i".into(), value: Value::Known(KnownVal::Num(1.0)) }),
            ])),
            else_body: None,
        }),
        Block::EditList(EditListData { op: ListEditOp::ReplaceAt, name: mem_var.clone(), index: Some(Value::Op(Op::Add(Box::new(Value::GetVar { name: "ptr".into() }), Box::new(Value::GetVar { name: "i".into() })))), value: Some(Value::Known(KnownVal::Num(0.0))) }),
        Block::SwitchCostume { value: Value::GetVar { name: "cost".into() } },
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::BoolToFloat(Box::new(enough_space.clone()))) }),
    ]), &mut ctx);

    add_func("SB3_render", vec![], BlockList::from_blocks(vec![
        Block::EditVolume { op: VolumeOp::Change, value: Value::Known(KnownVal::Num(0.0)) },
    ]), &mut ctx);

    add_func("SB3_say_str", vec!["input"], BlockList::from_blocks(vec![
        Block::ProcedureCall(ProcedureCallData { name: "!helper_str2scratch".into(), args: vec![Value::GetParam { name: "input".into() }], run_without_refresh: false }),
        Block::Say { value: Value::GetVar { name: rv.clone() } },
    ]), &mut ctx);

    add_func("SB3_say_char", vec!["input"], BlockList::from_blocks(vec![
        Block::Say { value: Value::GetOfList(GetOfList { op: ListOp::AtIndex, name: ascii_lookup.clone(), value: Box::new(Value::GetParam { name: "input".into() }) }) },
    ]), &mut ctx);

    add_func("SB3_say_dbl", vec!["input"], BlockList::from_blocks(vec![
        Block::Say { value: Value::GetParam { name: "input".into() } },
    ]), &mut ctx);

    add_func("SB3_wait", vec!["duration"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: "end".into(), value: Value::Op(Op::Add(Box::new(Value::DaysSince2000), Box::new(Value::Op(Op::Div(Box::new(Value::GetParam { name: "duration".into() }), Box::new(Value::Known(KnownVal::Num(86400.0)))))))) }),
        Block::ProcedureCall(ProcedureCallData { name: "SB3_render".into(), args: vec![], run_without_refresh: false }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::Until,
            condition: Some(Value::BoolOp(BoolOp::Gt(Box::new(Value::DaysSince2000), Box::new(Value::GetVar { name: "end".into() })))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::ProcedureCall(ProcedureCallData { name: "SB3_render".into(), args: vec![], run_without_refresh: false }),
            ])),
            else_body: None,
        }),
    ]), &mut ctx);

    add_func("SB3_wait_no_render", vec!["duration"], BlockList::from_blocks(vec![
        Block::Wait { value: Value::GetParam { name: "duration".into() } },
    ]), &mut ctx);

    add_func("SB3_ask_str", vec!["output", "input", "count"], BlockList::from_blocks(vec![
        Block::ProcedureCall(ProcedureCallData { name: "!helper_str2scratch".into(), args: vec![Value::GetParam { name: "input".into() }], run_without_refresh: false }),
        Block::Ask { value: Value::GetVar { name: rv.clone() }, var_name: None },
        Block::ProcedureCall(ProcedureCallData { name: "!helper_scratch2str".into(), args: vec![Value::GetAnswer, Value::GetParam { name: "output".into() }, Value::GetParam { name: "count".into() }], run_without_refresh: false }),
    ]), &mut ctx);

    add_func("SB3_ask_str_unsafe", vec!["output", "input"], BlockList::from_blocks(vec![
        Block::ProcedureCall(ProcedureCallData { name: "!helper_str2scratch".into(), args: vec![Value::GetParam { name: "input".into() }], run_without_refresh: false }),
        Block::Ask { value: Value::GetVar { name: rv.clone() }, var_name: None },
        Block::ProcedureCall(ProcedureCallData { name: "!helper_scratch2str".into(), args: vec![Value::GetAnswer, Value::GetParam { name: "output".into() }, Value::Known(KnownVal::Num(f64::INFINITY))], run_without_refresh: false }),
    ]), &mut ctx);

    add_func("SB3_ask_dbl", vec!["output", "input"], BlockList::from_blocks(vec![
        Block::ProcedureCall(ProcedureCallData { name: "!helper_str2scratch".into(), args: vec![Value::GetParam { name: "input".into() }], run_without_refresh: false }),
        Block::Ask { value: Value::GetVar { name: rv.clone() }, var_name: None },
        Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::Op(Op::StrToFloat(Box::new(Value::GetAnswer))) }),
        Block::EditList(EditListData { op: ListEditOp::ReplaceAt, name: mem_var.clone(), index: Some(Value::GetParam { name: "output".into() }), value: Some(Value::GetVar { name: "char".into() }) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Eq(Box::new(Value::GetAnswer), Box::new(Value::GetVar { name: "char".into() })))))) }),
    ]), &mut ctx);

    add_func("SB3_days_since_2000", vec![], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::DaysSince2000 }),
    ]), &mut ctx);

    add_func("_exit", vec!["status"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Op(Op::Join(Box::new(Value::Known(KnownVal::Str("exit called with status ".into()))), Box::new(Value::GetParam { name: "status".into() }))), var_name: None },
        Block::StopScript(StopOption::All),
    ]), &mut ctx);

    add_func("close", vec!["a"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("close called".into())), var_name: None },
    ]), &mut ctx);

    add_func("fstat", vec!["a", "b"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("fstat called".into())), var_name: None },
    ]), &mut ctx);

    add_func("isatty", vec!["a"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("isatty called".into())), var_name: None },
    ]), &mut ctx);

    add_func("lseek", vec!["a", "b", "c"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("lseek called".into())), var_name: None },
    ]), &mut ctx);

    add_func("read", vec!["a", "b", "c"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("read called".into())), var_name: None },
    ]), &mut ctx);

    add_func("sbrk", vec!["incr"], BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::GetVar { name: heap_pointer_var.clone() } }),
        Block::EditVar(EditVarData { op: VarOp::Change, name: heap_pointer_var.clone(), value: Value::GetParam { name: "incr".into() } }),
    ]), &mut ctx);

    add_func("write", vec!["file", "buf", "len"], BlockList::from_blocks(vec![
        Block::Ask { value: Value::Known(KnownVal::Str("write called".into())), var_name: None },
        Block::ProcedureCall(ProcedureCallData { name: "!helper_str2scratch".into(), args: vec![Value::GetParam { name: "buf".into() }], run_without_refresh: false }),
        Block::Ask { value: Value::GetVar { name: rv.clone() }, var_name: None },
    ]), &mut ctx);

    ctx
}

fn maybe_add_str2color_helper(ctx: &mut Context) {
    if !ctx.needs_str2color {
        return;
    }
    let rv = ctx.cfg.return_var.clone();
    let mem_var = ctx.cfg.mem_var.clone();
    let name = "!helper_str2color";
    let params = vec!["input"];
    let localized_params: Vec<Variable> = params.iter().map(|p| Variable {
        var_name: p.to_string(),
        var_type: VarType::Param,
        fn_name: Some(name.to_string()),
    }).collect();
    let raw_param_names: Vec<String> = localized_params
        .iter()
        .map(|p| p.get_raw_var_name(None))
        .collect();
    let mut blocks = BlockList::from_blocks(vec![
        Block::ProcedureDef(ProcedureDefData {
            name: name.to_string(),
            params: raw_param_names,
            warp: true,
        }),
    ]);
    blocks.add(BlockList::from_blocks(vec![
        Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Known(KnownVal::Num(0.0)) }),
        // Skip the leading '#'.
        Block::EditVar(EditVarData { op: VarOp::Set, name: "ptr".into(), value: Value::Op(Op::Add(Box::new(Value::GetParam { name: "input".into() }), Box::new(Value::Known(KnownVal::Num(1.0))))) }),
        Block::EditVar(EditVarData { op: VarOp::Set, name: "i".into(), value: Value::Known(KnownVal::Num(0.0)) }),
        Block::ControlFlow(ControlFlow {
            op: ControlOp::RepTimes,
            condition: Some(Value::Known(KnownVal::Num(6.0))),
            var: None,
            body: Some(BlockList::from_blocks(vec![
                Block::EditVar(EditVarData { op: VarOp::Set, name: "char".into(), value: Value::GetOfList(GetOfList { op: ListOp::AtIndex, name: mem_var.clone(), value: Box::new(Value::GetVar { name: "ptr".into() }) }) }),
                // digit = char - 48 (ASCII '0')
                Block::EditVar(EditVarData { op: VarOp::Set, name: "digit".into(), value: Value::Op(Op::Sub(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(48.0))))) }),
                // If char is lowercase a-f, digit = char - 87.
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(Value::BoolOp(BoolOp::And(
                        Box::new(Value::BoolOp(BoolOp::Gt(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(96.0)))))),
                        Box::new(Value::BoolOp(BoolOp::Lt(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(103.0)))))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::EditVar(EditVarData { op: VarOp::Set, name: "digit".into(), value: Value::Op(Op::Sub(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(87.0))))) }),
                    ])),
                    else_body: None,
                }),
                // If char is uppercase A-F, digit = char - 55.
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(Value::BoolOp(BoolOp::And(
                        Box::new(Value::BoolOp(BoolOp::Gt(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(64.0)))))),
                        Box::new(Value::BoolOp(BoolOp::Lt(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(71.0)))))),
                    ))),
                    var: None,
                    body: Some(BlockList::from_blocks(vec![
                        Block::EditVar(EditVarData { op: VarOp::Set, name: "digit".into(), value: Value::Op(Op::Sub(Box::new(Value::GetVar { name: "char".into() }), Box::new(Value::Known(KnownVal::Num(55.0))))) }),
                    ])),
                    else_body: None,
                }),
                Block::EditVar(EditVarData { op: VarOp::Set, name: rv.clone(), value: Value::Op(Op::Add(Box::new(Value::Op(Op::Mul(Box::new(Value::GetVar { name: rv.clone() }), Box::new(Value::Known(KnownVal::Num(16.0)))))), Box::new(Value::GetVar { name: "digit".into() }))) }),
                Block::EditVar(EditVarData { op: VarOp::Change, name: "ptr".into(), value: Value::Known(KnownVal::Num(1.0)) }),
                Block::EditVar(EditVarData { op: VarOp::Change, name: "i".into(), value: Value::Known(KnownVal::Num(1.0)) }),
            ])),
            else_body: None,
        }),
    ]));
    ctx.proj.code.push(blocks);
    let param_sizes: Vec<usize> = vec![1; params.len()];
    ctx.fn_info.insert(name.to_string(), FuncInfo::new(
        name.to_string(), ctx.next_fn_id, localized_params, param_sizes, params.len(),
    ));
    ctx.next_fn_id += 1;
}

struct FnInfoIntermediate {
    name: String,
    fn_id: usize,
    params: Vec<Variable>,
    param_sizes: Vec<usize>,
    value_param_count: usize,
    is_variadic: bool,
    can_call: HashSet<String>,
    block_alloca_size: HashMap<String, usize>,
    block_var_use: HashMap<String, super::config::BlockVarUse>,
    phi_info: HashMap<String, HashMap<String, Vec<(Variable, ir::Value)>>>,
    branches_to_first: bool,
    first_label: String,
}

fn get_fn_info(mod_: &DecodedModule, mut ctx: Context) -> Result<Context, CompException> {
    let mut next_fn_id: usize = 2;
    let mut intermediates: Vec<FnInfoIntermediate> = Vec::new();

    let mut return_addresses: HashMap<String, Vec<String>> = HashMap::new();
    let mut fn_ptr_sig_return_addrs: Vec<Vec<String>> = vec![Vec::new(); ctx.fn_ptr_sigs.len()];
    let mut fn_ptr_sig_called_by: Vec<HashSet<String>> = vec![HashSet::new(); ctx.fn_ptr_sigs.len()];
    let mut makes_variadic_alloc: HashMap<String, bool> = HashMap::new();

    let defined_func_names: HashSet<String> = mod_.functions
        .values()
        .filter(|f| !f.blocks.is_empty())
        .map(|f| f.name.clone())
        .collect();

    for (_, func) in &mod_.functions {
        if func.blocks.is_empty() {
            continue;
        }

        let fn_name = func.name.clone();
        let fn_id = next_fn_id;
        next_fn_id += 1;

        let mut params = Vec::new();
        let mut param_sizes = Vec::new();
        for arg in &func.params {
            let param_var = Variable {
                var_name: arg.name.clone(),
                var_type: VarType::Param,
                fn_name: Some(fn_name.clone()),
            };
            // Parameters are passed as value chunks, not memory bytes.
            let size = memory::get_size_of(&arg.type_, false)?;
            params.push(param_var);
            param_sizes.push(size);
        }

        let value_param_count = params.len();

        // Variadic functions receive a hidden pointer to the vararg area.
        if func.variadic {
            params.push(Variable {
                var_name: ctx.cfg.vararg_ptr_local.clone(),
                var_type: VarType::Param,
                fn_name: Some(fn_name.clone()),
            });
            param_sizes.push(1);
        }

        let mut can_call = HashSet::new();
        let mut block_alloca_size = HashMap::new();
        let mut phi_info: HashMap<String, HashMap<String, Vec<(Variable, ir::Value)>>> = HashMap::new();
        let mut outgoing_phi_values: HashMap<String, HashMap<String, Vec<ir::Value>>> = HashMap::new();

        for (_, block) in &func.blocks {
            let mut alloca_size: usize = 0;
            let mut call_id: usize = 0;

            for instr in &block.instrs {
                match instr {
                    ir::Instr::Call(call) => {
                        let localized_call_id = localize_call_id(call_id, &block.label, &fn_name, false);

                        *makes_variadic_alloc.entry(fn_name.clone()).or_insert(false) |= call.variadic;

                        let mut is_direct_call = false;
                        let mut could_call: Vec<String> = Vec::new();

                        if let ir::Value::Function(fv) = &call.func {
                            could_call.push(fv.name.clone());
                            is_direct_call = true;
                        } else {
                            let signature = FuncTy::new(
                                call.return_type.clone(),
                                call.params.clone(),
                                call.variadic,
                            );
                            if let Some((signature_id, sig_could_call)) = get_func_ptr_signature_info(&signature, &ctx) {
                                could_call.extend(sig_could_call.iter().cloned());
                                if could_call.len() == 1 {
                                    is_direct_call = true;
                                } else {
                                    fn_ptr_sig_called_by[signature_id].insert(fn_name.clone());
                                    fn_ptr_sig_return_addrs[signature_id].push(localized_call_id.clone());
                                }
                            }
                        }

                        if is_direct_call && !could_call.is_empty() && defined_func_names.contains(&could_call[0]) {
                            return_addresses.entry(could_call[0].clone()).or_insert_with(Vec::new).push(localized_call_id);
                        }

                        for called_name in &could_call {
                            if defined_func_names.contains(called_name) {
                                can_call.insert(called_name.clone());
                            }
                        }

                        call_id += 1;
                    }
                    ir::Instr::Alloca(alloc) => {
                        alloca_size += memory::get_size_of(&alloc.allocated_type, ctx.cfg.accurate_byte_spacing)?;
                    }
                    ir::Instr::Phi(phi) => {
                        for (val, label) in &phi.incoming {
                            let from_label = label.label.clone();
                            let to_label = block.label.clone();
                            let res_var = Variable {
                                var_name: phi.result.name.clone(),
                                var_type: VarType::Var,
                                fn_name: Some(fn_name.clone()),
                            };
                            phi_info
                                .entry(from_label.clone())
                                .or_insert_with(HashMap::new)
                                .entry(to_label.clone())
                                .or_insert_with(Vec::new)
                                .push((res_var.clone(), val.clone()));
                            outgoing_phi_values
                                .entry(from_label)
                                .or_insert_with(HashMap::new)
                                .entry(to_label)
                                .or_insert_with(Vec::new)
                                .push(val.clone());
                        }
                    }
                    _ => {}
                }
            }

            block_alloca_size.insert(block.label.clone(), alloca_size);
        }

        let block_var_use = super::graph_util::analyze_function_block_var_use(func, &outgoing_phi_values);

        let first_label = if let Some(first_block) = func.blocks.values().next() {
            first_block.label.clone()
        } else {
            "".to_string()
        };

        let branches_to_first = func.blocks.values().any(|b| {
            if b.label == first_label {
                return false;
            }
            b.instrs.last().map_or(false, |instr| {
                match instr {
                    ir::Instr::UncondBr(br) => br.branch.label == first_label,
                    ir::Instr::CondBr(cbr) => {
                        cbr.branch_true.label == first_label || cbr.branch_false.label == first_label
                    }
                    _ => false,
                }
            })
        });

        intermediates.push(FnInfoIntermediate {
            name: fn_name,
            fn_id,
            params,
            param_sizes,
            value_param_count,
            is_variadic: func.variadic,
            can_call,
            block_alloca_size,
            block_var_use,
            phi_info,
            branches_to_first,
            first_label,
        });
    }

    // Build call graph and compute transitive closures
    let mut call_graph: HashMap<String, HashSet<String>> = HashMap::new();
    for inter in &intermediates {
        let mut reachable = inter.can_call.clone();
        let mut stack: Vec<String> = inter.can_call.iter().cloned().collect();
        let mut visited = HashSet::new();
        while let Some(callee) = stack.pop() {
            if !visited.insert(callee.clone()) {
                continue;
            }
            if let Some(other) = intermediates.iter().find(|i| i.name == callee) {
                for next in &other.can_call {
                    if reachable.insert(next.clone()) {
                        stack.push(next.clone());
                    }
                }
            }
        }
        call_graph.insert(inter.name.clone(), reachable);
    }

    // Compute returns_to_address and check_locations for each function
    let mut returns_to_address: HashMap<String, bool> = HashMap::new();
    let mut check_locations: HashMap<String, Vec<String>> = HashMap::new();
    let mut total_alloca_size: HashMap<String, Option<usize>> = HashMap::new();

    for inter in &intermediates {
        let mut branches: HashMap<String, Vec<String>> = HashMap::new();
        branches.insert("ret".to_string(), Vec::new());
        for block_label in inter.block_var_use.keys() {
            let block_branches: Vec<String> = inter.block_var_use[block_label]
                .branches
                .iter()
                .cloned()
                .filter(|b| b != "ret")
                .collect();
            branches.insert(block_label.clone(), block_branches);
        }

        let mut fn_check_locations = Vec::new();
        let mut could_recurse = false;

        if !ctx.cfg.use_branch_jump_table {
            fn_check_locations = super::graph_util::select_cycle_checks(&branches);
            could_recurse = !fn_check_locations.is_empty();
            for branch in &fn_check_locations {
                ctx.all_check_locations.push((inter.name.clone(), branch.clone()));
            }
        }

        returns_to_address.insert(inter.name.clone(), could_recurse);
        check_locations.insert(inter.name.clone(), fn_check_locations);

        // Compute unavoidable branches and total alloca size
        let repeating_branches = super::graph_util::find_nodes_with_cycle(&branches);
        let unavoidable = super::graph_util::unavoidable_nodes(&branches, &inter.first_label, "ret");
        let ran_once_branches: HashSet<String> = unavoidable.difference(&repeating_branches).cloned().collect();

        let mut known_alloc_size = true;
        let mut alloc_size = 0usize;
        for block_label in branches.keys() {
            if block_label == "ret" {
                continue;
            }
            if ran_once_branches.contains(block_label) {
                alloc_size += inter.block_alloca_size.get(block_label).copied().unwrap_or(0);
            } else if inter.block_alloca_size.get(block_label).copied().unwrap_or(0) != 0 {
                known_alloc_size = false;
                break;
            }
        }

        let fn_total_alloca_size = if inter.branches_to_first {
            None
        } else if known_alloc_size {
            Some(alloc_size)
        } else {
            None
        };
        total_alloca_size.insert(inter.name.clone(), fn_total_alloca_size);
    }

    // Propagate downstream
    for inter in &intermediates {
        let name = inter.name.clone();
        let downstream_reaches = call_graph.get(&name).cloned().unwrap_or_default();
        let downstream_returns = downstream_reaches.iter().any(|call| *returns_to_address.get(call).unwrap_or(&false));
        if let Some(entry) = returns_to_address.get_mut(&name) {
            *entry |= downstream_returns;
        }
        let downstream_variadic = downstream_reaches.iter().any(|call| *makes_variadic_alloc.get(call).unwrap_or(&false));
        if downstream_variadic {
            *makes_variadic_alloc.entry(name).or_insert(false) = true;
        }
    }

    // Build function pointer signature info
    let mut fn_ptr_sig_info: Vec<FuncPtrSigInfo> = Vec::new();
    for (signature_id, (signature, could_call)) in ctx.fn_ptr_sigs.iter().enumerate() {
        let sig_returns_to_address = could_call.iter()
            .any(|call| *returns_to_address.get(call).unwrap_or(&false));
        let sig_return_addrs = fn_ptr_sig_return_addrs[signature_id].clone();
        let sig_takes_ret_addr = sig_returns_to_address && sig_return_addrs.len() > 1;
        let mut sig_could_call_total: HashSet<String> = could_call.iter().cloned().collect();

        if could_call.len() != 1 {
            for call in could_call {
                if let Some(reachable) = call_graph.get(call) {
                    sig_could_call_total.extend(reachable.iter().cloned());
                }
                return_addresses.entry((*call).clone()).or_insert_with(Vec::new)
                    .push(localize_func_ptr_sig_callback(signature_id));
            }
        }

        let sig_called_by = fn_ptr_sig_called_by[signature_id].clone();
        let sig_could_recurse = !sig_could_call_total.is_disjoint(&sig_called_by);

        fn_ptr_sig_info.push(FuncPtrSigInfo {
            signature_id,
            can_call: sig_could_call_total,
            value_param_count: signature.params.len(),
            is_variadic: signature.variadic,
            return_addresses: sig_return_addrs,
            returns_to_address: sig_returns_to_address,
            takes_return_address: sig_takes_ret_addr,
            could_recurse: sig_could_recurse,
        });
    }
    ctx.fn_ptr_sig_info = fn_ptr_sig_info;

    // Build final FuncInfo entries
    for mut inter in intermediates {
        let fn_total_alloca_size = total_alloca_size.get(&inter.name).copied().flatten();
        let skip_stack_size_change = fn_total_alloca_size.is_some()
            && call_graph.get(&inter.name).map_or(true, |calls| {
                calls.iter().all(|call| total_alloca_size.get(call).copied().flatten().unwrap_or(0) == 0)
            })
            && !makes_variadic_alloc.get(&inter.name).copied().unwrap_or(false);

        let fn_ret_addresses = return_addresses.get(&inter.name).cloned().unwrap_or_default();
        let fn_returns_to_address = *returns_to_address.get(&inter.name).unwrap_or(&false);
        let fn_takes_ret_addr = fn_returns_to_address && fn_ret_addresses.len() > 1;

        if fn_takes_ret_addr {
            inter.params.push(Variable {
                var_name: ctx.cfg.return_address_local.clone(),
                var_type: VarType::Param,
                fn_name: Some(inter.name.clone()),
            });
            inter.param_sizes.push(1);
        }

        let info = FuncInfo {
            name: inter.name.clone(),
            fn_id: inter.fn_id,
            params: inter.params,
            param_sizes: inter.param_sizes,
            value_param_count: inter.value_param_count,
            is_variadic: inter.is_variadic,
            can_call: inter.can_call,
            return_addresses: fn_ret_addresses,
            returns_to_address: fn_returns_to_address,
            takes_return_address: fn_takes_ret_addr,
            checked_blocks: check_locations.get(&inter.name).cloned().unwrap_or_default(),
            block_alloca_size: inter.block_alloca_size,
            total_alloca_size: fn_total_alloca_size,
            skip_stack_size_change,
            block_var_use: inter.block_var_use,
            branches_to_first: inter.branches_to_first,
            phi_info: inter.phi_info,
        };

        ctx.fn_info.insert(inter.name, info);
    }

    ctx.next_fn_id = next_fn_id;
    Ok(ctx)
}

/// Collect all direct setjmp calls in the module and assign each a global
/// dispatch id for the longjmp jump table.  This must run after get_fn_info
/// so that all_check_locations (and therefore the id base) is stable.
fn collect_setjmp_sites(mod_: &DecodedModule, ctx: &mut Context) -> Result<(), CompException> {
    let base_id = super::config::START_STACK_RESET_ID + ctx.all_check_locations.len();
    let mut next_id = base_id;

    for (_, func) in &mod_.functions {
        if func.blocks.is_empty() {
            continue;
        }

        for (_, block) in &func.blocks {
            let mut call_id: usize = 0;
            for instr in &block.instrs {
                if let ir::Instr::Call(call) = instr {
                    if let ir::Value::Function(fv) = &call.func {
                        if is_setjmp_name(&fv.name) {
                            let result_name = call
                                .result
                                .as_ref()
                                .map(|r| r.name.clone())
                                .unwrap_or_default();
                            ctx.setjmp_sites.push(SetjmpSite {
                                fn_name: func.name.clone(),
                                block_label: block.label.clone(),
                                call_idx: call_id,
                                dispatch_id: next_id,
                                continuation_name: localize_setjmp_continuation(
                                    call_id,
                                    &block.label,
                                    &func.name,
                                ),
                                result_var: localize_var(
                                    &result_name,
                                    false,
                                    Some(&func.name),
                                    false,
                                ),
                            });
                            next_id += 1;
                        }
                    }
                    call_id += 1;
                }
            }
        }
    }

    Ok(())
}

fn calculate_sum_diff(
    op: &str,
    lft: InferredValue,
    rgt: InferredValue,
    width: usize,
    ctx: &mut Context,
    is_nuw: bool,
) -> Result<(InferredValue, BlockList), CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        let lft_idx = match lft {
            InferredValue::Indexed(iv) => iv,
            _ => return Err(CompException(
                "calculate_sum_diff: expected indexed value for width > VARIABLE_MAX_BITS".to_string()
            )),
        };
        let rgt_idx = match rgt {
            InferredValue::Indexed(iv) => iv,
            _ => return Err(CompException(
                "calculate_sum_diff: expected indexed value for width > VARIABLE_MAX_BITS".to_string()
            )),
        };
        return Ok(calculate_wide_sum_diff(op, lft_idx, rgt_idx, width, ctx));
    }

    let lft_val = lft.into_single()?;
    let rgt_val = rgt.into_single()?;

    // Cannot overflow - result is guaranteed range [0, 2^width) for unsigned.
    if is_nuw {
        let val = if op == "add" {
            Value::Op(Op::Add(Box::new(lft_val), Box::new(rgt_val)))
        } else {
            Value::Op(Op::Sub(Box::new(lft_val), Box::new(rgt_val)))
        };
        return Ok((InferredValue::Single(val), BlockList::new()));
    }

    let mod_base = 2f64.powi(width as i32);

    let perf = &ctx.cfg.opt_target.perf;
    let mod_cost = perf.r#mod;
    let mod_is_faster = mod_cost < perf.add + perf.mul + perf.gt + perf.add + 2.0;

    let mut op = op.to_string();
    let mut lft = lft_val;
    let mut rgt = rgt_val;

    if ctx.cfg.compiler_minify {
        let (known_val, lft_is_known) = match (&lft, &rgt) {
            (Value::Known(KnownVal::Num(n)), _) => (*n, true),
            (_, Value::Known(KnownVal::Num(n))) => (*n, false),
            _ => (0.0, false),
        };
        let has_known = matches!(&lft, Value::Known(_)) || matches!(&rgt, Value::Known(_));

        if has_known {
            let known_val_int = known_val as i64;
            let mod_base_int = mod_base as i64;

            let (alt_op, alt_known) = if op == "add" {
                ("sub".to_string(), mod_base_int - known_val_int)
            } else if !lft_is_known {
                ("add".to_string(), mod_base_int - known_val_int)
            } else if mod_is_faster {
                (op.clone(), known_val_int - mod_base_int)
            } else {
                (op.clone(), known_val_int)
            };

            if alt_known.to_string().len() < known_val_int.to_string().len() {
                let alt_known_val = Value::Known(KnownVal::Num(alt_known as f64));
                if lft_is_known {
                    lft = alt_known_val;
                } else {
                    rgt = alt_known_val;
                }
                op = alt_op;
            }
        }
    }

    let unwrapped = if op == "add" {
        Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))
    } else {
        Value::Op(Op::Sub(Box::new(lft), Box::new(rgt)))
    };

    let res_val = if mod_is_faster {
        Value::Op(Op::Mod(
            Box::new(unwrapped.clone()),
            Box::new(Value::Known(KnownVal::Num(mod_base))),
        ))
    } else {
        let (comp_op, k_comp_val, adjustment) = if op == "add" {
            (">", mod_base - 1.0, -mod_base)
        } else {
            ("<", 0.0, mod_base)
        };
        let did_overflow = if comp_op == ">" {
            Value::BoolOp(BoolOp::Gt(
                Box::new(unwrapped.clone()),
                Box::new(Value::Known(KnownVal::Num(k_comp_val))),
            ))
        } else {
            Value::BoolOp(BoolOp::Lt(
                Box::new(unwrapped.clone()),
                Box::new(Value::Known(KnownVal::Num(k_comp_val))),
            ))
        };
        Value::Op(Op::Add(
            Box::new(unwrapped),
            Box::new(Value::Op(Op::Mul(
                Box::new(did_overflow),
                Box::new(Value::Known(KnownVal::Num(adjustment))),
            ))),
        ))
    };

    Ok((InferredValue::Single(res_val), BlockList::new()))
}

fn calculate_wide_sum_diff(
    op: &str,
    lft: IdxbleValue,
    rgt: IdxbleValue,
    width: usize,
    ctx: &mut Context,
) -> (InferredValue, BlockList) {
    let steps = lft.vals.len();
    assert_eq!(steps, rgt.vals.len());
    assert_eq!(steps, (width + super::config::VARIABLE_MAX_BITS - 1) / super::config::VARIABLE_MAX_BITS);

    if steps == 0 {
        return (InferredValue::Indexed(IdxbleValue { vals: vec![Value::Known(KnownVal::Num(0.0))] }), BlockList::new());
    }

    let perf = ctx.cfg.opt_target.perf.clone();
    let max_spacing = std::cmp::min(steps, 10);

    let mut best_cost = f64::INFINITY;
    let mut best_blocks = BlockList::new();
    let mut best_sum_nodes: Vec<Value> = Vec::new();
    let mut best_temp_names: std::collections::HashMap<usize, String> = std::collections::HashMap::new();

    for spacing in 1..=max_spacing {
        let start_index = spacing;
        for omit_last in [false, true] {
            let mut checkpoint_indices: std::collections::HashSet<usize> = (start_index..steps).step_by(spacing).collect();
            if omit_last && checkpoint_indices.contains(&(steps - 1)) {
                checkpoint_indices.remove(&(steps - 1));
            }

            let mut cost = 0.0;
            let mut blocks = BlockList::new();
            let mut sum_nodes: Vec<Value> = Vec::new();
            let mut stored_temp_names: std::collections::HashMap<usize, String> = std::collections::HashMap::new();

            for i in 0..steps {
                let is_last_step = i == steps - 1;
                let modulus = if is_last_step {
                    let remainder = width % super::config::VARIABLE_MAX_BITS;
                    if remainder == 0 { 2f64.powi(super::config::VARIABLE_MAX_BITS as i32) } else { 2f64.powi(remainder as i32) }
                } else {
                    2f64.powi(super::config::VARIABLE_MAX_BITS as i32)
                };

                let raw = if i == 0 {
                    let v = if op == "add" {
                        Value::Op(Op::Add(Box::new(lft.vals[0].clone()), Box::new(rgt.vals[0].clone())))
                    } else {
                        Value::Op(Op::Sub(Box::new(lft.vals[0].clone()), Box::new(rgt.vals[0].clone())))
                    };
                    cost += get_value_cost(&v, &perf);
                    v
                } else {
                    let earlier_stored: Vec<usize> = stored_temp_names.keys().copied().filter(|&idx| idx < i).collect();
                    let prev_stored = earlier_stored.into_iter().max();

                    let (start, mut prev) = if let Some(ps) = prev_stored {
                        (ps + 1, Value::GetVar { name: stored_temp_names[&ps].clone() })
                    } else {
                        (1, if op == "add" {
                            Value::Op(Op::Add(Box::new(lft.vals[0].clone()), Box::new(rgt.vals[0].clone())))
                        } else {
                            Value::Op(Op::Sub(Box::new(lft.vals[0].clone()), Box::new(rgt.vals[0].clone())))
                        })
                    };

                    for j in start..=i {
                        prev = partial_sum_diff(op, &lft.vals[j], &rgt.vals[j], &prev);
                    }

                    prev
                };

                if checkpoint_indices.contains(&i) && i >= start_index {
                    let temp_name = format!("!tmp:{}", i);
                    stored_temp_names.insert(i, temp_name);
                    blocks.add_block(Block::EditVar(crate::scratch::ast::EditVarData {
                        op: VarOp::Set,
                        name: format!("!tmp:{}", i),
                        value: raw.clone(),
                    }));
                    cost += perf.set_var + get_value_cost(&raw, &perf);
                }

                let expr_for_mod = if stored_temp_names.contains_key(&i) {
                    Value::GetVar { name: stored_temp_names[&i].clone() }
                } else {
                    raw
                };

                let res_node = Value::Op(Op::Mod(
                    Box::new(expr_for_mod),
                    Box::new(Value::Known(KnownVal::Num(modulus))),
                ));
                cost += get_value_cost(&res_node, &perf);
                sum_nodes.push(res_node);
            }

            if cost < best_cost {
                best_cost = cost;
                best_blocks = blocks;
                best_sum_nodes = sum_nodes;
                best_temp_names = stored_temp_names;
            }
        }
    }

    for (i, placeholder) in &best_temp_names {
        let real_name = gen_temp_var(ctx);
        replace_var_name_in_blocks(&mut best_blocks, placeholder, &real_name);
        for node in &mut best_sum_nodes {
            replace_var_name_in_value(node, placeholder, &real_name);
        }
        let _ = i;
    }

    (InferredValue::Indexed(IdxbleValue { vals: best_sum_nodes }), best_blocks)
}

fn ensure_pow2_lookup(ctx: &mut Context) {
    let name = ctx.cfg.pow2_lookup_var.clone();
    if !ctx.proj.lists.contains_key(&name) {
        let mut values = Vec::new();
        let max = super::config::VARIABLE_MAX_BITS as i32;
        for exp in -max..=max {
            values.push(KnownVal::Num(2f64.powi(exp)));
        }
        ctx.proj.lists.insert(name, values);
    }
}

fn get_pow2_multiplier(
    ctx: &mut Context,
    val: &Value,
    manual_offset: i32,
) -> Result<Value, CompException> {
    if !matches!(val, Value::Known(_)) {
        ensure_pow2_lookup(ctx);
    }
    twos_complement::int_pow2(val, manual_offset)
}

fn bit_shift(
    direction: &str,
    width: usize,
    val: InferredValue,
    shift: InferredValue,
    ctx: &mut Context,
    can_shift_out: bool,
) -> Result<(InferredValue, BlockList), CompException> {
    if width > 128 {
        return Err(CompException(format!(
            "bit_shift: width {} exceeds maximum supported width", width
        )));
    }

    let shift_single = match shift {
        InferredValue::Indexed(idx) => idx.vals[0].clone(),
        InferredValue::Single(v) => v,
    };
    let multiplier = get_pow2_multiplier(ctx, &shift_single, 0)?;

    match val {
        InferredValue::Indexed(idx) => {
            let vals = idx.vals;
            if vals.len() != 2 {
                return Err(CompException(format!(
                    "bit_shift only supports two-chunk values, got {} chunks", vals.len()
                )));
            }

            let high_width = if width % super::config::VARIABLE_MAX_BITS == 0 {
                super::config::VARIABLE_MAX_BITS
            } else {
                width % super::config::VARIABLE_MAX_BITS
            };

            let (high_part, mut high_blocks) = bit_shift(
                direction,
                high_width,
                InferredValue::Single(vals[1].clone()),
                InferredValue::Single(shift_single.clone()),
                ctx,
                can_shift_out || direction == "right",
            )?;
            let (low_part, low_blocks) = bit_shift(
                direction,
                super::config::VARIABLE_MAX_BITS,
                InferredValue::Single(vals[0].clone()),
                InferredValue::Single(shift_single.clone()),
                ctx,
                can_shift_out || direction == "left",
            )?;

            let mut high_part = high_part.into_single()?;
            let mut low_part = low_part.into_single()?;

            let remainder_shift_val = Value::Op(Op::Sub(
                Box::new(Value::Known(KnownVal::Num(super::config::VARIABLE_MAX_BITS as f64))),
                Box::new(shift_single.clone()),
            ));
            let remainder_shift = InferredValue::Single(simplify_value(&remainder_shift_val));

            if direction == "left" {
                let (rem, rem_blocks) = bit_shift(
                    "right",
                    super::config::VARIABLE_MAX_BITS,
                    InferredValue::Single(vals[0].clone()),
                    remainder_shift,
                    ctx,
                    true,
                )?;
                let rem = rem.into_single()?;
                high_part = Value::Op(Op::Add(Box::new(high_part), Box::new(rem)));
                high_blocks.add(low_blocks);
                high_blocks.add(rem_blocks);
            } else {
                let (rem, rem_blocks) = bit_shift(
                    "left",
                    super::config::VARIABLE_MAX_BITS,
                    InferredValue::Single(vals[1].clone()),
                    remainder_shift,
                    ctx,
                    true,
                )?;
                let rem = rem.into_single()?;
                low_part = Value::Op(Op::Add(Box::new(low_part), Box::new(rem)));
                high_blocks.add(low_blocks);
                high_blocks.add(rem_blocks);
            }

            Ok((InferredValue::Indexed(IdxbleValue { vals: vec![low_part, high_part] }), high_blocks))
        }
        InferredValue::Single(v) => {
            if direction == "left" {
                let unwrapped = Value::Op(Op::Mul(Box::new(v), Box::new(multiplier)));
                let res = if can_shift_out {
                    Value::Op(Op::Mod(
                        Box::new(unwrapped),
                        Box::new(Value::Known(KnownVal::Num(2f64.powi(width as i32)))),
                    ))
                } else {
                    unwrapped
                };
                Ok((InferredValue::Single(res), BlockList::new()))
            } else {
                let unwrapped = Value::Op(Op::Div(Box::new(v), Box::new(multiplier)));
                let res = if can_shift_out {
                    Value::Op(Op::Floor(Box::new(unwrapped)))
                } else {
                    unwrapped
                };
                Ok((InferredValue::Single(res), BlockList::new()))
            }
        }
    }
}

fn set_bool_var(var: &Variable, bool_val: Value) -> Result<BlockList, CompException> {
    let cond_val = Value::Op(Op::BoolToFloat(Box::new(bool_val)));
    let block = var.set_value(cond_val, VarOp::Set, None)?;
    Ok(BlockList::from_blocks(vec![block]))
}

fn int_compare_single(lft: Value, rgt: Value, width: usize, cond: ir::instructions::ICmpCond) -> Value {
    match cond {
        ir::instructions::ICmpCond::Eq => {
            Value::BoolOp(BoolOp::Eq(Box::new(lft), Box::new(rgt)))
        }
        ir::instructions::ICmpCond::Ne => {
            Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Eq(
                Box::new(lft),
                Box::new(rgt),
            )))))
        }
        ir::instructions::ICmpCond::Sgt | ir::instructions::ICmpCond::Sge |
        ir::instructions::ICmpCond::Slt | ir::instructions::ICmpCond::Sle => {
            let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
            let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
            match cond {
                ir::instructions::ICmpCond::Sgt => Value::BoolOp(BoolOp::Gt(Box::new(sl), Box::new(sr))),
                ir::instructions::ICmpCond::Sge => Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Lt(Box::new(sl), Box::new(sr)))))),
                ir::instructions::ICmpCond::Slt => Value::BoolOp(BoolOp::Lt(Box::new(sl), Box::new(sr))),
                ir::instructions::ICmpCond::Sle => Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(Box::new(sl), Box::new(sr)))))),
                _ => unreachable!(),
            }
        }
        ir::instructions::ICmpCond::Ugt => Value::BoolOp(BoolOp::Gt(Box::new(lft), Box::new(rgt))),
        ir::instructions::ICmpCond::Uge => Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Lt(Box::new(lft), Box::new(rgt)))))),
        ir::instructions::ICmpCond::Ult => Value::BoolOp(BoolOp::Lt(Box::new(lft), Box::new(rgt))),
        ir::instructions::ICmpCond::Ule => Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(Box::new(lft), Box::new(rgt)))))),
    }
}

fn large_int_compare(
    lft: &IdxbleValue,
    rgt: &IdxbleValue,
    width: usize,
    cond: ir::instructions::ICmpCond,
    ctx: &mut Context,
    res_var: &Variable,
) -> Result<(Option<Value>, BlockList), CompException> {
    let chunks = lft.vals.len();
    if chunks != rgt.vals.len() {
        return Err(CompException(format!(
            "large_int_compare: operand chunk count mismatch: {} vs {}",
            chunks, rgt.vals.len()
        )));
    }

    match cond {
        ir::instructions::ICmpCond::Eq => {
            let mut current = Value::BoolOp(BoolOp::Eq(
                Box::new(lft.vals[chunks - 1].clone()),
                Box::new(rgt.vals[chunks - 1].clone()),
            ));
            for i in (0..chunks - 1).rev() {
                current = Value::BoolOp(BoolOp::And(
                    Box::new(Value::BoolOp(BoolOp::Eq(
                        Box::new(lft.vals[i].clone()),
                        Box::new(rgt.vals[i].clone()),
                    ))),
                    Box::new(current),
                ));
            }
            Ok((Some(current), BlockList::new()))
        }
        ir::instructions::ICmpCond::Ne => {
            let (eq, blocks) = large_int_compare(lft, rgt, width, ir::instructions::ICmpCond::Eq, ctx, res_var)?;
            let eq = eq.ok_or_else(|| CompException("large_int_compare Eq returned None".to_string()))?;
            Ok((Some(Value::BoolOp(BoolOp::Not(Box::new(eq)))), blocks))
        }
        ir::instructions::ICmpCond::Ugt |
        ir::instructions::ICmpCond::Ult |
        ir::instructions::ICmpCond::Uge |
        ir::instructions::ICmpCond::Ule => {
            // Matches Python's largeIntCompare unsigned branch.
            let comp_op = if matches!(cond, ir::instructions::ICmpCond::Ugt | ir::instructions::ICmpCond::Ule) {
                ">"
            } else {
                "<"
            };
            let lsb_width = std::cmp::min(width, super::config::VARIABLE_MAX_BITS);
            let mut if_branch = set_bool_var(res_var, int_compare_single(
                lft.vals[0].clone(), rgt.vals[0].clone(), lsb_width, cond,
            ))?;

            for i in 1..chunks {
                let eq = Value::BoolOp(BoolOp::Eq(
                    Box::new(lft.vals[i].clone()),
                    Box::new(rgt.vals[i].clone()),
                ));
                let else_branch = set_bool_var(res_var, if comp_op == ">" {
                    Value::BoolOp(BoolOp::Gt(Box::new(lft.vals[i].clone()), Box::new(rgt.vals[i].clone())))
                } else {
                    Value::BoolOp(BoolOp::Lt(Box::new(lft.vals[i].clone()), Box::new(rgt.vals[i].clone())))
                })?;
                if_branch = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(eq),
                    var: None,
                    body: Some(if_branch),
                    else_body: Some(else_branch),
                })]);
            }
            Ok((None, if_branch))
        }
        ir::instructions::ICmpCond::Sgt |
        ir::instructions::ICmpCond::Slt |
        ir::instructions::ICmpCond::Sge |
        ir::instructions::ICmpCond::Sle => {
            // Matches Python's largeIntCompare signed branch.
            if chunks <= 1 {
                return Err(CompException(format!(
                    "large_int_compare: signed comparison requires multi-chunk value, got {} chunk(s)", chunks
                )));
            }
            let signed_mode = if matches!(cond, ir::instructions::ICmpCond::Sgt | ir::instructions::ICmpCond::Sge) {
                ir::instructions::ICmpCond::Sgt
            } else {
                ir::instructions::ICmpCond::Slt
            };
            let unsigned_mode = match cond {
                ir::instructions::ICmpCond::Sgt => ir::instructions::ICmpCond::Ugt,
                ir::instructions::ICmpCond::Slt => ir::instructions::ICmpCond::Ult,
                ir::instructions::ICmpCond::Sge => ir::instructions::ICmpCond::Uge,
                ir::instructions::ICmpCond::Sle => ir::instructions::ICmpCond::Ule,
                _ => unreachable!(),
            };
            let msb_width = width % super::config::VARIABLE_MAX_BITS;
            let msb_width = if msb_width == 0 { super::config::VARIABLE_MAX_BITS } else { msb_width };
            let signed_comp = set_bool_var(res_var, int_compare_single(
                lft.vals[chunks - 1].clone(),
                rgt.vals[chunks - 1].clone(),
                msb_width,
                signed_mode,
            ))?;
            let lower_lft = IdxbleValue { vals: lft.vals[..chunks - 1].to_vec() };
            let lower_rgt = IdxbleValue { vals: rgt.vals[..chunks - 1].to_vec() };
            let lower_width = (chunks - 1) * super::config::VARIABLE_MAX_BITS;
            let (_, unsigned_comp) = large_int_compare(
                &lower_lft, &lower_rgt, lower_width, unsigned_mode, ctx, res_var,
            )?;

            let eq_msb = Value::BoolOp(BoolOp::Eq(
                Box::new(lft.vals[chunks - 1].clone()),
                Box::new(rgt.vals[chunks - 1].clone()),
            ));
            let blocks = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
                op: ControlOp::IfElse,
                condition: Some(eq_msb),
                var: None,
                body: Some(unsigned_comp),
                else_body: Some(signed_comp),
            })]);
            Ok((None, blocks))
        }
    }
}

fn should_optimise_value_use(value: &Value, times_used: f64, ctx: &Context) -> bool {
    if times_used <= 1.0 {
        return false;
    }
    let perf = &ctx.cfg.opt_target.perf;
    let cost = get_value_cost(value, perf);
    let elision_cost = cost * times_used;
    let no_elision_cost = perf.set_var + cost + perf.get_var * times_used;
    // Matches Python's shouldOptimiseValueUse = not shouldElide,
    // where shouldElide returns no_elision_cost > elision_cost.
    no_elision_cost <= elision_cost
}

fn optimise_value_use(value: Value, times_used: f64, ctx: &mut Context) -> (Value, BlockList) {
    if should_optimise_value_use(&value, times_used, ctx) {
        let tmp = gen_temp_var(ctx);
        let block = Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: tmp.clone(),
            value,
        });
        (Value::GetVar { name: tmp }, BlockList::from_blocks(vec![block]))
    } else {
        (value, BlockList::new())
    }
}

fn signed_div_blocks(
    res_var: &Variable,
    lft: Value,
    rgt: Value,
    width: usize,
    is_exact: bool,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        return Err(CompException(format!(
            "Signed division of {}-bit integers is not supported",
            width
        )));
    }

    let mut blocks = BlockList::new();
    if is_exact {
        let signed_lft = twos_complement::undo_twos_complement(lft, width);
        let signed_rgt = twos_complement::undo_twos_complement(rgt, width);
        let div = Value::Op(Op::Div(Box::new(signed_lft), Box::new(signed_rgt)));
        let res = twos_complement::apply_twos_complement(div, width);
        blocks.add_block(res_var.set_value(res, VarOp::Set, None)?);
        return Ok(blocks);
    }

    let (lft, lblocks) = optimise_value_use(lft, 2.0, ctx);
    let (rgt, rblocks) = optimise_value_use(rgt, 2.0, ctx);
    blocks.add(lblocks);
    blocks.add(rblocks);

    let point_of_neg = 2f64.powi(width as i32) / 2.0;
    let change = 2f64.powi(width as i32);
    let known_point = Value::Known(KnownVal::Num(point_of_neg));
    let known_change = Value::Known(KnownVal::Num(change));

    let rgt_minus_change = Value::Op(Op::Sub(
        Box::new(rgt.clone()),
        Box::new(known_change.clone()),
    ));
    let lft_minus_change = Value::Op(Op::Sub(
        Box::new(lft.clone()),
        Box::new(known_change.clone()),
    ));

    let set_pos_pos = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
            Box::new(lft.clone()),
            Box::new(rgt.clone()),
        ))))),
        VarOp::Set,
        None,
    )?]);

    let set_pos_neg = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Add(
            Box::new(Value::Op(Op::Ceiling(Box::new(Value::Op(Op::Div(
                Box::new(lft.clone()),
                Box::new(rgt_minus_change.clone()),
            )))))),
            Box::new(known_change.clone()),
        )),
        VarOp::Set,
        None,
    )?]);

    let cond_inner = Value::BoolOp(BoolOp::Lt(
        Box::new(rgt.clone()),
        Box::new(known_point.clone()),
    ));
    let inner_pos = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_inner.clone()),
        var: None,
        body: Some(set_pos_pos),
        else_body: Some(set_pos_neg),
    })]);

    let set_neg_pos = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Add(
            Box::new(Value::Op(Op::Ceiling(Box::new(Value::Op(Op::Div(
                Box::new(lft_minus_change.clone()),
                Box::new(rgt.clone()),
            )))))),
            Box::new(known_change.clone()),
        )),
        VarOp::Set,
        None,
    )?]);

    let set_neg_neg = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
            Box::new(lft_minus_change.clone()),
            Box::new(rgt_minus_change.clone()),
        ))))),
        VarOp::Set,
        None,
    )?]);

    let inner_neg = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_inner),
        var: None,
        body: Some(set_neg_pos),
        else_body: Some(set_neg_neg),
    })]);

    let cond_outer = Value::BoolOp(BoolOp::Lt(Box::new(lft), Box::new(known_point)));
    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_outer),
        var: None,
        body: Some(inner_pos),
        else_body: Some(inner_neg),
    }));

    Ok(blocks)
}

fn signed_rem_blocks(
    res_var: &Variable,
    lft: Value,
    rgt: Value,
    width: usize,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        return Err(CompException(format!(
            "Signed remainder of {}-bit integers is not supported",
            width
        )));
    }

    let mut blocks = BlockList::new();
    let (lft, lblocks) = optimise_value_use(lft, 2.0, ctx);
    let (rgt, rblocks) = optimise_value_use(rgt, 3.0, ctx);
    blocks.add(lblocks);
    blocks.add(rblocks);

    let point_of_neg = 2f64.powi(width as i32) / 2.0;
    let change = 2f64.powi(width as i32);
    let known_point = Value::Known(KnownVal::Num(point_of_neg));
    let known_change = Value::Known(KnownVal::Num(change));

    let (right_sub_change, mut pos_neg_opt_blocks) = optimise_value_use(rgt.clone(), 2.0, ctx);
    pos_neg_opt_blocks.add_block(res_var.set_value(
        Value::Op(Op::Sub(
            Box::new(Value::Op(Op::Mod(
                Box::new(lft.clone()),
                Box::new(right_sub_change.clone()),
            ))),
            Box::new(right_sub_change),
        )),
        VarOp::Set,
        None,
    )?);

    let set_pos_pos = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Mod(Box::new(lft.clone()), Box::new(rgt.clone()))),
        VarOp::Set,
        None,
    )?]);

    let cond_inner = Value::BoolOp(BoolOp::Lt(
        Box::new(rgt.clone()),
        Box::new(known_point.clone()),
    ));
    let inner_pos = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_inner.clone()),
        var: None,
        body: Some(set_pos_pos),
        else_body: Some(pos_neg_opt_blocks),
    })]);

    let lft_minus_change = Value::Op(Op::Sub(
        Box::new(lft.clone()),
        Box::new(known_change.clone()),
    ));
    let rgt_minus_change = Value::Op(Op::Sub(
        Box::new(rgt.clone()),
        Box::new(known_change.clone()),
    ));

    let set_neg_pos = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Add(
            Box::new(Value::Op(Op::Sub(
                Box::new(Value::Op(Op::Mod(
                    Box::new(lft_minus_change.clone()),
                    Box::new(rgt.clone()),
                ))),
                Box::new(rgt.clone()),
            ))),
            Box::new(known_change.clone()),
        )),
        VarOp::Set,
        None,
    )?]);

    let set_neg_neg = BlockList::from_blocks(vec![res_var.set_value(
        Value::Op(Op::Add(
            Box::new(Value::Op(Op::Mod(
                Box::new(lft_minus_change),
                Box::new(rgt_minus_change),
            ))),
            Box::new(known_change.clone()),
        )),
        VarOp::Set,
        None,
    )?]);

    let inner_neg = BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_inner),
        var: None,
        body: Some(set_neg_pos),
        else_body: Some(set_neg_neg),
    })]);

    let cond_outer = Value::BoolOp(BoolOp::Lt(Box::new(lft), Box::new(known_point)));
    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond_outer),
        var: None,
        body: Some(inner_pos),
        else_body: Some(inner_neg),
    }));

    Ok(blocks)
}

fn ashr_blocks(
    res_var: &Variable,
    lft: Value,
    rgt: Value,
    width: usize,
    is_exact: bool,
    ctx: &mut Context,
) -> Result<BlockList, CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        return Err(CompException(format!(
            "AShr of {}-bit integers is not supported",
            width
        )));
    }

    let mut blocks = BlockList::new();
    let point_of_neg = 2f64.powi(width as i32) / 2.0;
    let change = 2f64.powi(width as i32);
    let right_mul = get_pow2_multiplier(ctx, &rgt, 0)?;

    let unwrapped_pos = Value::Op(Op::Div(Box::new(lft.clone()), Box::new(right_mul.clone())));
    let val_pos = if is_exact {
        unwrapped_pos
    } else {
        Value::Op(Op::Floor(Box::new(unwrapped_pos)))
    };

    let lft_minus_change = Value::Op(Op::Sub(
        Box::new(lft.clone()),
        Box::new(Value::Known(KnownVal::Num(change))),
    ));
    let unwrapped_neg = Value::Op(Op::Div(
        Box::new(lft_minus_change),
        Box::new(right_mul),
    ));
    let val_neg_inner = if is_exact {
        unwrapped_neg
    } else {
        Value::Op(Op::Ceiling(Box::new(unwrapped_neg)))
    };
    let val_neg = Value::Op(Op::Add(
        Box::new(val_neg_inner),
        Box::new(Value::Known(KnownVal::Num(change))),
    ));

    let set_pos = BlockList::from_blocks(vec![res_var.set_value(val_pos, VarOp::Set, None)?]);
    let set_neg = BlockList::from_blocks(vec![res_var.set_value(val_neg, VarOp::Set, None)?]);
    let cond = Value::BoolOp(BoolOp::Lt(
        Box::new(lft),
        Box::new(Value::Known(KnownVal::Num(point_of_neg))),
    ));
    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(cond),
        var: None,
        body: Some(set_pos),
        else_body: Some(set_neg),
    }));

    Ok(blocks)
}

fn multiply_no_wrap(left: Value, right: Value, width: usize) -> Result<Value, CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        return Err(CompException(format!("Multipling {} bits is not supported", width)));
    }
    Ok(Value::Op(Op::Mul(Box::new(left), Box::new(right))))
}

/// Multiply two 64-bit unsigned integers represented as [low(48), high(16)]
/// chunks, which is the format llvm2scratch uses for 64-bit integers.
/// This specialised implementation assumes the high 32 bits of both operands
/// are zero (i.e. the inputs are zero-extended 32-bit values), which is the
/// pattern emitted by Eigenmath's multi-precision arithmetic.
fn multiply_wide_u64(lft: &IdxbleValue, rgt: &IdxbleValue) -> Result<IdxbleValue, CompException> {
    if lft.vals.len() != 2 || rgt.vals.len() != 2 {
        return Err(CompException(
            "Only 64-bit wide multiplication is currently supported".to_string(),
        ));
    }

    // Operands are [low(48), high(16)].  For Eigenmath the high chunk is always
    // zero, so we only need the low 32 bits of each operand.
    let a = lft.vals[0].clone();
    let b = rgt.vals[0].clone();

    let mask16 = Value::Known(KnownVal::Num(0xFFFF as f64));
    let shift16 = Value::Known(KnownVal::Num(65536.0));
    let base32 = Value::Known(KnownVal::Num(4294967296.0));

    let a_lo = Value::Op(Op::Mod(Box::new(a.clone()), Box::new(mask16.clone())));
    let a_hi = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(a.clone()),
        Box::new(mask16.clone()),
    )))));
    let b_lo = Value::Op(Op::Mod(Box::new(b.clone()), Box::new(mask16.clone())));
    let b_hi = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(b.clone()),
        Box::new(mask16.clone()),
    )))));

    let p00 = Value::Op(Op::Mul(Box::new(a_lo.clone()), Box::new(b_lo.clone())));
    let p10 = Value::Op(Op::Mul(Box::new(a_hi.clone()), Box::new(b_lo.clone())));
    let p01 = Value::Op(Op::Mul(Box::new(a_lo.clone()), Box::new(b_hi.clone())));
    let p11 = Value::Op(Op::Mul(Box::new(a_hi.clone()), Box::new(b_hi.clone())));

    let mid = Value::Op(Op::Add(Box::new(p10), Box::new(p01)));
    let mid_shifted = Value::Op(Op::Mul(Box::new(mid.clone()), Box::new(shift16)));
    let sum_low = Value::Op(Op::Add(Box::new(p00), Box::new(mid_shifted)));

    let low = Value::Op(Op::Mod(Box::new(sum_low.clone()), Box::new(base32.clone())));
    let carry = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(sum_low),
        Box::new(base32),
    )))));
    let high = Value::Op(Op::Add(Box::new(p11), Box::new(carry)));

    // Convert the intermediate [32, 32] chunk result back to [48, 16].
    let low_base = Value::Known(KnownVal::Num(281474976710656.0)); // 2^48
    let high_shift = Value::Known(KnownVal::Num(4294967296.0)); // 2^32
    let high_mask = Value::Known(KnownVal::Num(65536.0)); // 2^16

    let res_low = Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Add(
            Box::new(low.clone()),
            Box::new(Value::Op(Op::Mul(
                Box::new(Value::Op(Op::Mod(
                    Box::new(high.clone()),
                    Box::new(high_mask.clone()),
                ))),
                Box::new(high_shift.clone()),
            ))),
        ))),
        Box::new(low_base),
    ));
    let res_high = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(high),
        Box::new(high_mask),
    )))));

    Ok(IdxbleValue { vals: vec![res_low, res_high] })
}

fn multiply_wrap(
    left: Value,
    right: Value,
    width: usize,
    ctx: &mut Context,
) -> Result<(Value, BlockList), CompException> {
    if width > super::config::VARIABLE_MAX_BITS {
        return Err(CompException(format!("Multipling {} bits not supported", width)));
    }

    if width <= 26 {
        // Safe: (2**26) ** 2 < 2**53
        let value = Value::Op(Op::Mod(
            Box::new(Value::Op(Op::Mul(Box::new(left), Box::new(right)))),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(width as i32)))),
        ));
        return Ok((value, BlockList::new()));
    }

    if width <= 50 {
        // Safe (with extra mod step): 2**25 * 2**25 + 2**25 * 2**25 < 2**53
        let mut blocks = BlockList::new();

        let (left, lblocks) = optimise_value_use(left, 3.0, ctx);
        let (right, rblocks) = optimise_value_use(right, 3.0, ctx);
        blocks.add(lblocks);
        blocks.add(rblocks);

        let half_width = width / 2;
        let pow_half = 2f64.powi(half_width as i32);

        let a0 = Value::Op(Op::Mod(
            Box::new(left.clone()),
            Box::new(Value::Known(KnownVal::Num(pow_half))),
        ));
        let b0 = Value::Op(Op::Mod(
            Box::new(right.clone()),
            Box::new(Value::Known(KnownVal::Num(pow_half))),
        ));

        let (a0, a0blocks) = optimise_value_use(a0, 2.0, ctx);
        let (b0, b0blocks) = optimise_value_use(b0, 2.0, ctx);
        blocks.add(a0blocks);
        blocks.add(b0blocks);

        let a0b1_plus_b0a1 = Value::Op(Op::Add(
            Box::new(Value::Op(Op::Mul(
                Box::new(a0.clone()),
                Box::new(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                    Box::new(right.clone()),
                    Box::new(Value::Known(KnownVal::Num(pow_half))),
                )))))),
            ))),
            Box::new(Value::Op(Op::Mul(
                Box::new(b0.clone()),
                Box::new(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                    Box::new(left.clone()),
                    Box::new(Value::Known(KnownVal::Num(pow_half))),
                )))))),
            ))),
        ));

        let a0b1_plus_b0a1 = if width > 34 {
            Value::Op(Op::Mod(
                Box::new(a0b1_plus_b0a1),
                Box::new(Value::Known(KnownVal::Num(2f64.powi(((width as f64) / 2.0).ceil() as i32)))),
            ))
        } else {
            a0b1_plus_b0a1
        };

        let value = Value::Op(Op::Mod(
            Box::new(Value::Op(Op::Add(
                Box::new(Value::Op(Op::Mul(
                    Box::new(a0b1_plus_b0a1),
                    Box::new(Value::Known(KnownVal::Num(pow_half))),
                ))),
                Box::new(Value::Op(Op::Mul(Box::new(a0), Box::new(b0)))),
            ))),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(width as i32)))),
        ));

        return Ok((value, blocks));
    }

    Err(CompException(format!("Multipling {} bits not supported", width)))
}

/// Arithmetic right shift of a 64-bit signed integer by exactly 32 bits.
///
/// The wide-integer representation uses VARIABLE_MAX_BITS (48) as the low
/// chunk width, so a 64-bit value is stored as [low(48), high(16)].
/// ashr by 32 yields a signed 33-bit result that fits in the low chunk;
/// the high chunk is simply sign-extended.
fn ashr_i64_by_32(val: &IdxbleValue) -> Result<IdxbleValue, CompException> {
    if val.vals.len() != 2 {
        return Err(CompException(
            "ashr_i64_by_32 only supports two-chunk values".to_string(),
        ));
    }
    let low = val.vals[0].clone(); // 48 bits
    let high = val.vals[1].clone(); // 16 bits

    let half_high = Value::Known(KnownVal::Num(32768.0)); // 2^15
    let high_base = Value::Known(KnownVal::Num(65536.0)); // 2^16
    let low_shift = Value::Known(KnownVal::Num(4294967296.0)); // 2^32
    let low_base = Value::Known(KnownVal::Num(281474976710656.0)); // 2^48

    // Sign bit of the whole 64-bit value is bit 15 of the high chunk.
    let sign_bit = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(high.clone()),
        Box::new(half_high),
    )))));

    // Interpret the high chunk as a signed 16-bit value.
    let signed_high = Value::Op(Op::Sub(
        Box::new(high),
        Box::new(Value::Op(Op::Mul(
            Box::new(sign_bit.clone()),
            Box::new(high_base.clone()),
        ))),
    ));

    // Shift the low chunk right by 32.
    let low_shifted = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(low),
        Box::new(low_shift),
    )))));

    // Combine and normalise to an unsigned low chunk.
    let raw_result = Value::Op(Op::Add(
        Box::new(low_shifted),
        Box::new(Value::Op(Op::Mul(Box::new(signed_high), Box::new(high_base)))),
    ));
    let low_chunk = Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Add(Box::new(raw_result), Box::new(low_base.clone())))),
        Box::new(low_base),
    ));

    // Sign-extend the high chunk.
    let high_chunk = Value::Op(Op::Mul(
        Box::new(sign_bit),
        Box::new(Value::Known(KnownVal::Num(65535.0))),
    ));

    Ok(IdxbleValue {
        vals: vec![low_chunk, high_chunk],
    })
}

fn handle_arith_binop(
    opcode: ir::instructions::BinaryOpcode,
    lft: Value,
    rgt: Value,
    _res_var: &Variable,
    _blocks: &mut BlockList,
    _ctx: &mut Context,
) -> Result<Option<InferredValue>, CompException> {
    match opcode {
        ir::instructions::BinaryOpcode::FAdd => Ok(Some(InferredValue::Single(Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))))),
        ir::instructions::BinaryOpcode::FSub => Ok(Some(InferredValue::Single(Value::Op(Op::Sub(Box::new(lft), Box::new(rgt)))))),
        ir::instructions::BinaryOpcode::FMul => Ok(Some(InferredValue::Single(Value::Op(Op::Mul(Box::new(lft), Box::new(rgt)))))),
        ir::instructions::BinaryOpcode::FDiv => Ok(Some(InferredValue::Single(Value::Op(Op::Div(Box::new(lft), Box::new(rgt)))))),
        ir::instructions::BinaryOpcode::FRem => {
            let cond = match (&lft, &rgt) {
                (Value::Known(KnownVal::Num(lk)), Value::Known(KnownVal::Num(rk))) => {
                    if *lk > 0.0 {
                        Value::Known(KnownVal::Num(if *rk < 0.0 { 1.0 } else { 0.0 }))
                    } else {
                        Value::Known(KnownVal::Num(if *rk > 0.0 { 1.0 } else { 0.0 }))
                    }
                }
                (Value::Known(KnownVal::Num(lk)), _) => {
                    if *lk > 0.0 {
                        Value::BoolOp(BoolOp::Lt(Box::new(rgt.clone()), Box::new(Value::Known(KnownVal::Num(0.0)))))
                    } else {
                        Value::BoolOp(BoolOp::Gt(Box::new(rgt.clone()), Box::new(Value::Known(KnownVal::Num(0.0)))))
                    }
                }
                (_, Value::Known(KnownVal::Num(rk))) => {
                    if *rk > 0.0 {
                        Value::BoolOp(BoolOp::Lt(Box::new(lft.clone()), Box::new(Value::Known(KnownVal::Num(0.0)))))
                    } else {
                        Value::BoolOp(BoolOp::Gt(Box::new(lft.clone()), Box::new(Value::Known(KnownVal::Num(0.0)))))
                    }
                }
                _ => Value::BoolOp(BoolOp::Lt(
                    Box::new(Value::Op(Op::Mul(Box::new(lft.clone()), Box::new(rgt.clone())))),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                )),
            };
            let modded = Value::Op(Op::Mod(Box::new(lft.clone()), Box::new(rgt.clone())));
            let adjustment = Value::Op(Op::Mul(Box::new(rgt.clone()), Box::new(cond)));
            Ok(Some(InferredValue::Single(Value::Op(Op::Sub(
                Box::new(modded),
                Box::new(adjustment),
            )))))
        }
        _ => Err(CompException(format!("Unsupported binary opcode: {:?}", opcode))),
    }
}

fn is_comptime_known(val: &Value) -> bool {
    matches!(val, Value::Known(KnownVal::Num(_)))
}

fn try_comptime_wide_div(
    lft: &IdxbleValue,
    rgt: &IdxbleValue,
    _width: usize,
) -> Result<Option<IdxbleValue>, CompException> {
    let all_known = lft.vals.iter().all(is_comptime_known)
        && rgt.vals.iter().all(is_comptime_known);
    if !all_known {
        return Ok(None);
    }

    let mut dividend = 0u128;
    for (i, v) in lft.vals.iter().enumerate() {
        if let Value::Known(KnownVal::Num(n)) = v {
            let chunk_val = *n as u128;
            dividend |= chunk_val << (i * super::config::VARIABLE_MAX_BITS);
        }
    }

    let mut divisor = 0u128;
    for (i, v) in rgt.vals.iter().enumerate() {
        if let Value::Known(KnownVal::Num(n)) = v {
            let chunk_val = *n as u128;
            divisor |= chunk_val << (i * super::config::VARIABLE_MAX_BITS);
        }
    }

    if divisor == 0 {
        return Err(CompException("Division by zero".to_string()));
    }

    let quotient = dividend / divisor;
    let nchunks = lft.vals.len();
    let mut result = Vec::with_capacity(nchunks);
    for i in 0..nchunks {
        let chunk = (quotient >> (i * super::config::VARIABLE_MAX_BITS)) & ((1u128 << super::config::VARIABLE_MAX_BITS) - 1);
        result.push(Value::Known(KnownVal::Num(chunk as f64)));
    }
    Ok(Some(IdxbleValue { vals: result }))
}

fn try_comptime_wide_rem(
    lft: &IdxbleValue,
    rgt: &IdxbleValue,
    _width: usize,
) -> Result<Option<IdxbleValue>, CompException> {
    let all_known = lft.vals.iter().all(is_comptime_known)
        && rgt.vals.iter().all(is_comptime_known);
    if !all_known {
        return Ok(None);
    }

    let mut dividend = 0u128;
    for (i, v) in lft.vals.iter().enumerate() {
        if let Value::Known(KnownVal::Num(n)) = v {
            let chunk_val = *n as u128;
            dividend |= chunk_val << (i * super::config::VARIABLE_MAX_BITS);
        }
    }

    let mut divisor = 0u128;
    for (i, v) in rgt.vals.iter().enumerate() {
        if let Value::Known(KnownVal::Num(n)) = v {
            let chunk_val = *n as u128;
            divisor |= chunk_val << (i * super::config::VARIABLE_MAX_BITS);
        }
    }

    if divisor == 0 {
        return Err(CompException("Division by zero".to_string()));
    }

    let remainder = dividend % divisor;
    let nchunks = lft.vals.len();
    let mut result = Vec::with_capacity(nchunks);
    for i in 0..nchunks {
        let chunk = (remainder >> (i * super::config::VARIABLE_MAX_BITS)) & ((1u128 << super::config::VARIABLE_MAX_BITS) - 1);
        result.push(Value::Known(KnownVal::Num(chunk as f64)));
    }
    Ok(Some(IdxbleValue { vals: result }))
}

fn wide_udiv(lft: &IdxbleValue, rgt: &IdxbleValue, width: usize) -> Result<InferredValue, CompException> {
    if let Some(result) = try_comptime_wide_div(lft, rgt, width)? {
        return Ok(InferredValue::Indexed(result));
    }
    Err(CompException(format!(
        "Unsigned division of {}-bit wide integers is not supported for non-constant values", width
    )))
}

fn wide_udiv_with_rem(lft: &IdxbleValue, rgt: &IdxbleValue, width: usize) -> Result<(InferredValue, InferredValue), CompException> {
    let q = try_comptime_wide_div(lft, rgt, width)?;
    let r = try_comptime_wide_rem(lft, rgt, width)?;
    match (q, r) {
        (Some(q), Some(r)) => Ok((InferredValue::Indexed(q), InferredValue::Indexed(r))),
        _ => Err(CompException(format!(
            "Unsigned division/remainder of {}-bit wide integers is not supported for non-constant values", width
        ))),
    }
}

fn replace_var_name_in_value(value: &mut Value, old_name: &str, new_name: &str) {
    match value {
        Value::GetVar { name } => {
            if name == old_name {
                *name = new_name.to_string();
            }
        }
        Value::Op(op) => {
            replace_var_name_in_value(op.left_mut(), old_name, new_name);
            if let Some(r) = op.right_mut() {
                replace_var_name_in_value(r, old_name, new_name);
            }
        }
        Value::BoolOp(bop) => {
            replace_var_name_in_value(bop.left_mut(), old_name, new_name);
            if let Some(r) = bop.right_mut() {
                replace_var_name_in_value(r, old_name, new_name);
            }
        }
        Value::GetOfList(gol) => {
            replace_var_name_in_value(&mut gol.value, old_name, new_name);
        }
        _ => {}
    }
}

fn replace_var_name_in_blocks(blocks: &mut BlockList, old_name: &str, new_name: &str) {
    for block in &mut blocks.blocks {
        match block {
            Block::EditVar(data) => {
                if data.name == old_name {
                    data.name = new_name.to_string();
                }
                replace_var_name_in_value(&mut data.value, old_name, new_name);
            }
            Block::EditList(data) => {
                if let Some(ref mut idx) = data.index {
                    replace_var_name_in_value(idx, old_name, new_name);
                }
                if let Some(ref mut val) = data.value {
                    replace_var_name_in_value(val, old_name, new_name);
                }
            }
            Block::ControlFlow(cf) => {
                if let Some(ref mut cond) = cf.condition {
                    replace_var_name_in_value(cond, old_name, new_name);
                }
                if let Some(ref mut body) = cf.body {
                    replace_var_name_in_blocks(body, old_name, new_name);
                }
                if let Some(ref mut else_body) = cf.else_body {
                    replace_var_name_in_blocks(else_body, old_name, new_name);
                }
            }
            Block::ProcedureCall(data) => {
                for arg in &mut data.args {
                    replace_var_name_in_value(arg, old_name, new_name);
                }
            }
            _ => {}
        }
    }
}

fn should_carry(op: &str, prev_sum: &Value, width: usize) -> Value {
    if op == "add" {
        Value::BoolOp(BoolOp::Gt(
            Box::new(prev_sum.clone()),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(width as i32) - 1.0))),
        ))
    } else {
        Value::BoolOp(BoolOp::Lt(
            Box::new(prev_sum.clone()),
            Box::new(Value::Known(KnownVal::Num(0.0))),
        ))
    }
}

fn partial_sum_diff(op: &str, lft: &Value, rgt: &Value, prev_sum: &Value) -> Value {
    let raw_sum = if op == "add" {
        Value::Op(Op::Add(Box::new(lft.clone()), Box::new(rgt.clone())))
    } else {
        Value::Op(Op::Sub(Box::new(lft.clone()), Box::new(rgt.clone())))
    };
    let carry = should_carry(op, prev_sum, super::config::VARIABLE_MAX_BITS);
    if op == "add" {
        Value::Op(Op::Add(Box::new(raw_sum), Box::new(carry)))
    } else {
        Value::Op(Op::Sub(Box::new(raw_sum), Box::new(carry)))
    }
}

/// Build a blocklist that checks `!longjmp pending` and propagates a longjmp
/// by stopping the current custom block.  This is inserted after every call
/// when the program contains setjmp/longjmp.
fn longjmp_propagation_check(ctx: &Context) -> Option<BlockList> {
    if ctx.setjmp_sites.is_empty() {
        return None;
    }
    Some(BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::If,
        condition: Some(Value::BoolOp(BoolOp::Eq(
            Box::new(Value::GetVar {
                name: ctx.cfg.longjmp_pending_var.clone(),
            }),
            Box::new(Value::Known(KnownVal::Num(1.0))),
        ))),
        var: None,
        body: Some(BlockList::from_blocks(vec![Block::StopScript(StopOption::This)])),
        else_body: None,
    })]))
}

/// Helper to read a single word from `!mem` at `base + offset`.
fn mem_load_word(base: Value, offset: f64, mem_var: &str) -> Value {
    Value::GetOfList(GetOfList {
        op: ListOp::AtIndex,
        name: mem_var.to_string(),
        value: Box::new(Value::Op(Op::Add(
            Box::new(base),
            Box::new(Value::Known(KnownVal::Num(offset))),
        ))),
    })
}

/// Helper to build a `replace item of list` block for `!mem` at `base + offset`.
fn mem_store_word(base: Value, offset: f64, value: Value, mem_var: &str) -> Block {
    Block::EditList(EditListData {
        op: ListEditOp::ReplaceAt,
        name: mem_var.to_string(),
        index: Some(Value::Op(Op::Add(
            Box::new(base),
            Box::new(Value::Known(KnownVal::Num(offset))),
        ))),
        value: Some(value),
    })
}

/// Copy the first `!local stack size` elements of `!local stack` into the
/// global setjmp snapshot list. Used before a setjmp so longjmp can restore
/// the saved register frame.
fn save_setjmp_local_stack_snapshot(ctx: &mut Context) -> BlockList {
    let mut blocks = BlockList::new();
    let local_stack_var = ctx.cfg.local_stack_var.clone();
    let snapshot_var = ctx.cfg.setjmp_snapshot_list_var.clone();
    let size_var = ctx.cfg.local_stack_size_var.clone();
    let idx_var = gen_temp_var(ctx);

    blocks.add_block(Block::EditList(EditListData {
        op: ListEditOp::DeleteAll,
        name: snapshot_var.clone(),
        value: None,
        index: None,
    }));

    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: idx_var.clone(),
        value: Value::Known(KnownVal::Num(1.0)),
    }));

    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::While,
        condition: Some(Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(
            Box::new(Value::GetVar { name: idx_var.clone() }),
            Box::new(Value::GetVar { name: size_var.clone() }),
        )))))),
        var: None,
        body: Some(BlockList::from_blocks(vec![
            Block::EditList(EditListData {
                op: ListEditOp::AddTo,
                name: snapshot_var.clone(),
                value: Some(Value::GetOfList(GetOfList {
                    op: ListOp::AtIndex,
                    name: local_stack_var.clone(),
                    value: Box::new(Value::GetVar { name: idx_var.clone() }),
                })),
                index: None,
            }),
            Block::EditVar(EditVarData {
                op: VarOp::Change,
                name: idx_var.clone(),
                value: Value::Known(KnownVal::Num(1.0)),
            }),
        ])),
        else_body: None,
    }));

    blocks
}

/// Restore `!local stack` from the global setjmp snapshot list. Assumes
/// `!local stack size` has already been restored to the saved value.
fn restore_setjmp_local_stack_snapshot(ctx: &mut Context) -> BlockList {
    let mut blocks = BlockList::new();
    let local_stack_var = ctx.cfg.local_stack_var.clone();
    let snapshot_var = ctx.cfg.setjmp_snapshot_list_var.clone();
    let size_var = ctx.cfg.local_stack_size_var.clone();
    let idx_var = gen_temp_var(ctx);

    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: idx_var.clone(),
        value: Value::Known(KnownVal::Num(1.0)),
    }));

    blocks.add_block(Block::ControlFlow(ControlFlow {
        op: ControlOp::While,
        condition: Some(Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(
            Box::new(Value::GetVar { name: idx_var.clone() }),
            Box::new(Value::GetVar { name: size_var.clone() }),
        )))))),
        var: None,
        body: Some(BlockList::from_blocks(vec![
            Block::EditList(EditListData {
                op: ListEditOp::ReplaceAt,
                name: local_stack_var.clone(),
                value: Some(Value::GetOfList(GetOfList {
                    op: ListOp::AtIndex,
                    name: snapshot_var.clone(),
                    value: Box::new(Value::GetVar { name: idx_var.clone() }),
                })),
                index: Some(Value::GetVar { name: idx_var.clone() }),
            }),
            Block::EditVar(EditVarData {
                op: VarOp::Change,
                name: idx_var.clone(),
                value: Value::Known(KnownVal::Num(1.0)),
            }),
        ])),
        else_body: None,
    }));

    blocks
}

/// Translate a setjmp call.  Ends the current procedure after saving the
/// execution context and starts the continuation procedure for the code
/// after the setjmp call.
fn trans_setjmp_call(
    call: &ir::instructions::Call,
    site: &SetjmpSite,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<(), CompException> {
    let env_ptr = trans_value(&call.args[0], ctx, Some(bctx))?.into_single()?;
    let mem_var = ctx.cfg.mem_var.clone();

    // Save execution context into jmp_buf.
    bctx.code.add_block(mem_store_word(
        env_ptr.clone(),
        0.0,
        Value::Known(KnownVal::Num(0x5EDCAFE as f64)),
        &mem_var,
    ));
    bctx.code.add_block(mem_store_word(
        env_ptr.clone(),
        1.0,
        Value::Known(KnownVal::Num(site.dispatch_id as f64)),
        &mem_var,
    ));
    bctx.code.add_block(mem_store_word(
        env_ptr.clone(),
        2.0,
        Value::GetVar {
            name: ctx.cfg.stack_pointer_var.clone(),
        },
        &mem_var,
    ));
    bctx.code.add_block(mem_store_word(
        env_ptr.clone(),
        3.0,
        Value::GetVar {
            name: ctx.cfg.local_stack_size_var.clone(),
        },
        &mem_var,
    ));

    // Save a snapshot of the local variable stack so it can be restored on longjmp.
    bctx.code.add(save_setjmp_local_stack_snapshot(ctx));

    // Initialize longjmp control variables for the first-time path.
    bctx.code.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.longjmp_target_id_var.clone(),
        value: Value::Known(KnownVal::Num(site.dispatch_id as f64)),
    }));
    bctx.code.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.longjmp_return_value_var.clone(),
        value: Value::Known(KnownVal::Num(0.0)),
    }));
    bctx.code.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.longjmp_pending_var.clone(),
        value: Value::Known(KnownVal::Num(0.0)),
    }));

    // Call the continuation inline; this is the first-time-through path.
    bctx.code.add_block(Block::ProcedureCall(ProcedureCallData {
        name: site.continuation_name.clone(),
        args: Vec::new(),
        run_without_refresh: false,
    }));
    // The setup code must not fall through into the continuation's body, which
    // is emitted as a separate procedure below.
    bctx.code.add_block(Block::StopScript(StopOption::This));

    // End the current (setup) procedure.
    ctx.proj.code.push(bctx.code.clone());

    // Start the continuation procedure.  When entered via longjmp, the
    // dispatcher has already set !longjmp return value and !longjmp pending.
    let mut cont = BlockList::from_blocks(vec![Block::ProcedureDef(ProcedureDefData {
        name: site.continuation_name.clone(),
        params: Vec::new(),
        warp: true,
    })]);

    let res_var = Variable {
        var_name: site.result_var.clone(),
        var_type: VarType::Var,
        fn_name: None,
    };
    cont.add_block(res_var.set_value(
        Value::GetVar {
            name: ctx.cfg.longjmp_return_value_var.clone(),
        },
        VarOp::Set,
        None,
    )?);
    // In branch-jump-table mode the continuation is entered from the top-level
    // jump table after a longjmp, so reset the pending flag here. In procedure-
    // call mode longjmp calls the continuation directly and relies on the flag
    // staying set to propagate through callers.
    if ctx.cfg.use_branch_jump_table {
        cont.add_block(Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: ctx.cfg.longjmp_pending_var.clone(),
            value: Value::Known(KnownVal::Num(0.0)),
        }));
    }

    bctx.code = cont;
    bctx.available_params = Vec::new();
    bctx.available_param_sizes = Vec::new();

    Ok(())
}

/// Translate a longjmp call.  Restores the saved context and signals the
/// dispatcher to resume at the corresponding setjmp continuation.
fn trans_longjmp_call(
    call: &ir::instructions::Call,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<BlockList, CompException> {
    let env_ptr = trans_value(&call.args[0], ctx, Some(bctx))?.into_single()?;
    let val = trans_value(&call.args[1], ctx, Some(bctx))?.into_single()?;
    let mem_var = ctx.cfg.mem_var.clone();

    let mut blocks = BlockList::new();

    // Restore stack pointer.
    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.stack_pointer_var.clone(),
        value: mem_load_word(env_ptr.clone(), 2.0, &mem_var),
    }));

    // Restore local stack size.
    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.local_stack_size_var.clone(),
        value: mem_load_word(env_ptr.clone(), 3.0, &mem_var),
    }));

    // Restore the local variable stack from the snapshot taken at setjmp.
    blocks.add(restore_setjmp_local_stack_snapshot(ctx));

    // Set the longjmp target to the dispatch id stored in jmp_buf.
    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.longjmp_target_id_var.clone(),
        value: mem_load_word(env_ptr.clone(), 1.0, &mem_var),
    }));

    // C semantics: if val is 0, setjmp returns 1.
    let ret_val = Value::Op(Op::Add(
        Box::new(val.clone()),
        Box::new(Value::Op(Op::Mul(
            Box::new(Value::BoolOp(BoolOp::Eq(
                Box::new(val.clone()),
                Box::new(Value::Known(KnownVal::Num(0.0))),
            ))),
            Box::new(Value::Known(KnownVal::Num(1.0))),
        ))),
    ));
    blocks.add_block(Block::EditVar(EditVarData {
        op: VarOp::Set,
        name: ctx.cfg.longjmp_return_value_var.clone(),
        value: ret_val,
    }));

    if ctx.cfg.use_branch_jump_table {
        // Branch-jump-table mode: propagate via the pending flag to the top-level
        // jump table. This path is guarded in trans_funcs in Phase 1.
        blocks.add_block(Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: ctx.cfg.longjmp_pending_var.clone(),
            value: Value::Known(KnownVal::Num(1.0)),
        }));
        blocks.add_block(Block::StopScript(StopOption::This));
    } else {
        // Procedure-call mode: longjmp directly resumes the matching setjmp
        // continuation. This avoids relying on a top-level jump table.
        let dispatch_value = Value::GetVar {
            name: ctx.cfg.longjmp_target_id_var.clone(),
        };
        blocks.add(build_longjmp_dispatch(&ctx.setjmp_sites, &dispatch_value)?);
        blocks.add_block(Block::StopScript(StopOption::This));
    }

    Ok(blocks)
}

/// Build a nested if-else chain that calls the continuation whose dispatch id
/// matches `dispatch_value`. Used for longjmp in procedure-call mode.
fn build_longjmp_dispatch(
    sites: &[SetjmpSite],
    dispatch_value: &Value,
) -> Result<BlockList, CompException> {
    if sites.is_empty() {
        return Err(CompException("longjmp encountered without any setjmp site".to_string()));
    }

    fn build_chain(
        sites: &[SetjmpSite],
        dispatch_value: &Value,
        idx: usize,
    ) -> BlockList {
        let site = &sites[idx];
        let call_cont = BlockList::from_blocks(vec![Block::ProcedureCall(ProcedureCallData {
            name: site.continuation_name.clone(),
            args: Vec::new(),
            run_without_refresh: false,
        })]);
        if idx == sites.len() - 1 {
            return call_cont;
        }
        let else_body = build_chain(sites, dispatch_value, idx + 1);
        BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
            op: ControlOp::IfElse,
            condition: Some(Value::BoolOp(BoolOp::Eq(
                Box::new(dispatch_value.clone()),
                Box::new(Value::Known(KnownVal::Num(site.dispatch_id as f64))),
            ))),
            var: None,
            body: Some(call_cont),
            else_body: Some(else_body),
        })])
    }

    Ok(build_chain(sites, dispatch_value, 0))
}

/// Names treated as built-in pen/motion helpers. Calls to these functions are
/// lowered directly to Scratch pen/motion blocks instead of procedure calls.
/// This is the *mechanism* layer: only the minimal atomic pen/motion
/// primitives are recognized. All geometry policy lives in `pen_gfx.h`.
const PEN_MOTION_BUILTINS: &[&str] = &[
    "pen_goto",
    "pen_down",
    "pen_up",
    "pen_clear",
    "pen_color",
    "pen_color_num",
    "pen_size",
];

fn is_pen_motion_builtin(name: &str) -> bool {
    PEN_MOTION_BUILTINS.contains(&name)
}

fn trans_pen_motion_call(
    name: &str,
    args: &[ir::Value],
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<Option<BlockList>, CompException> {
    let mut values = Vec::new();
    for arg in args {
        values.push(trans_value(arg, ctx, Some(bctx))?.into_single()?);
    }

    let blocks = match name {
        "pen_down" => Some(BlockList::from_block(Block::Pen(PenOp::Down))),
        "pen_up" => Some(BlockList::from_block(Block::Pen(PenOp::Up))),
        "pen_clear" => Some(BlockList::from_block(Block::Pen(PenOp::Clear))),
        "pen_color" => {
            ctx.needs_str2color = true;
            let color_arg = values.first().cloned().unwrap_or(Value::Known(KnownVal::Num(0.0)));
            let mut bl = BlockList::new();
            bl.add_block(Block::ProcedureCall(ProcedureCallData {
                name: "!helper_str2color".into(),
                args: vec![color_arg],
                run_without_refresh: false,
            }));
            bl.add_block(Block::Pen(PenOp::SetColor {
                color: Value::GetVar { name: ctx.cfg.return_var.clone() },
            }));
            Some(bl)
        }
        "pen_color_num" => {
            let color = values.first().cloned().unwrap_or(Value::Known(KnownVal::Num(0.0)));
            Some(BlockList::from_block(Block::Pen(PenOp::SetColor { color })))
        }
        "pen_size" => {
            let size = values.first().cloned().unwrap_or(Value::Known(KnownVal::Num(0.0)));
            Some(BlockList::from_block(Block::Pen(PenOp::SetSize { size })))
        }
        "pen_goto" => {
            let x = values.get(0).cloned().unwrap_or(Value::Known(KnownVal::Num(0.0)));
            let y = values.get(1).cloned().unwrap_or(Value::Known(KnownVal::Num(0.0)));
            Some(BlockList::from_block(Block::MotionGoto { x, y }))
        }
        _ => None,
    };

    Ok(blocks)
}

/// Translate a call instruction. Returns the blocks to run before the call,
/// the blocks to run after the call (result assignment and vararg cleanup),
/// and whether the callee returns to an address.
fn trans_call_instr(
    call: &ir::instructions::Call,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
    return_addr_id: Option<usize>,
) -> Result<(BlockList, BlockList, bool), CompException> {
    let mut pre_call = BlockList::new();
    let mut post_call = BlockList::new();

    if let ir::Value::Function(fv) = &call.func {
        if is_pen_motion_builtin(&fv.name) {
            if let Some(blocks) = trans_pen_motion_call(&fv.name, &call.args, ctx, bctx)? {
                return Ok((blocks, BlockList::new(), false));
            }
        }
    }

    let (callee_name, callee_is_variadic, callee_value_param_count, prefix_arg, callee_returns_to_address) = match &call.func {
        ir::Value::Function(fv) => {
            let info = ctx.fn_info.get(&fv.name).cloned().ok_or_else(|| {
                CompException(format!("Could not find function {}", fv.name))
            })?;
            (fv.name.clone(), info.is_variadic, info.value_param_count, None, info.returns_to_address)
        }
        _ => {
            let signature = FuncTy::new(
                call.return_type.clone(),
                call.params.clone(),
                call.variadic,
            );
            let sig_info = get_func_ptr_signature_info(&signature, ctx)
                .ok_or_else(|| CompException(format!("Could not find function signature for {:?}", signature)))?;
            let (signature_id, could_call) = sig_info;
            if could_call.len() == 1 {
                let name = could_call[0].clone();
                let info = ctx.fn_info.get(&name).cloned().ok_or_else(|| {
                    CompException(format!("Could not find function {}", name))
                })?;
                (name, info.is_variadic, info.value_param_count, None, info.returns_to_address)
            } else {
                let info = ctx.fn_ptr_sig_info.get(signature_id).cloned().ok_or_else(|| {
                    CompException(format!("Could not find function pointer signature info {}", signature_id))
                })?;
                let func_ptr_val = trans_value(&call.func, ctx, Some(bctx))?;
                let func_ptr_addr = match func_ptr_val {
                    InferredValue::Single(v) => v,
                    InferredValue::Indexed(_) => return Err(CompException(
                        "Function pointer must be single value".to_string()
                    )),
                };
                (localize_func_ptr_sig(signature_id), info.is_variadic, info.value_param_count, Some(func_ptr_addr), info.returns_to_address)
            }
        }
    };

    // Split explicit value arguments from trailing varargs for variadic callees.
    let (value_args, varargs) = if callee_is_variadic {
        let n = callee_value_param_count;
        call.args.split_at(n)
    } else {
        (&call.args[..], &[][..])
    };

    let mut args: Vec<Value> = Vec::new();
    if let Some(prefix) = prefix_arg {
        args.push(prefix);
    }
    for arg in value_args {
        let arg_val = trans_value(arg, ctx, Some(bctx))?;
        match arg_val {
            InferredValue::Single(v) => args.push(v),
            InferredValue::Indexed(iv) => args.extend(iv.vals),
        }
    }

    // Allocate stack space for varargs and pass the pointer to the callee.
    let mut vararg_alloc_size: usize = 0;
    let vararg_ptr: Value = if callee_is_variadic {
        vararg_alloc_size = varargs
            .iter()
            .map(|arg| memory::get_size_of(arg.type_(), ctx.cfg.accurate_byte_spacing))
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .sum();

        let ptr = if vararg_alloc_size != 0 {
            let sp = Value::GetVar {
                name: ctx.cfg.stack_pointer_var.clone(),
            };
            pre_call.add_block(Block::EditVar(EditVarData {
                op: VarOp::Change,
                name: ctx.cfg.stack_pointer_var.clone(),
                value: Value::Known(KnownVal::Num(-(vararg_alloc_size as f64))),
            }));
            sp
        } else {
            Value::Known(KnownVal::Num(0.0))
        };

        let mut offset = 0usize;
        for arg in varargs {
            let arg_val = trans_value(arg, ctx, Some(bctx))?;
            let arg_addr = Value::Op(Op::Add(
                Box::new(ptr.clone()),
                Box::new(Value::Known(KnownVal::Num(offset as f64))),
            ));
            pre_call.add(trans_store(arg_val, arg_addr, arg.type_(), ctx)?);
            offset += memory::get_size_of(arg.type_(), ctx.cfg.accurate_byte_spacing)?;
        }

        ptr
    } else {
        Value::Known(KnownVal::Num(0.0))
    };

    if callee_is_variadic {
        args.push(vararg_ptr);
    }

    if let Some(id) = return_addr_id {
        args.push(Value::Known(KnownVal::Num(id as f64)));
    }

    pre_call.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
        name: callee_name.clone(),
        args,
        run_without_refresh: false,
    }));

    if let Some(result) = &call.result {
        let res_var = Variable {
            var_name: localize_var(&result.name, false, Some(&bctx.fn_info.name), false),
            var_type: VarType::Var,
            fn_name: None,
        };
        let result_size = memory::get_size_of(&call.return_type, false)?;
        let return_var = Variable {
            var_name: ctx.cfg.return_var.clone(),
            var_type: VarType::SpecialVar,
            fn_name: None,
        };
        let ret_val = if result_size == 1 {
            InferredValue::Single(return_var.get_value(None))
        } else {
            InferredValue::Indexed(return_var.get_all_values(result_size))
        };
        post_call.add(res_var.set_inferred_value(ret_val)?);
    }

    // Deallocate vararg memory after reading the return value so that the
    // scratchblocks output matches the Python reference order.
    if callee_is_variadic && vararg_alloc_size != 0 {
        post_call.add_block(Block::EditVar(EditVarData {
            op: VarOp::Change,
            name: ctx.cfg.stack_pointer_var.clone(),
            value: Value::Known(KnownVal::Num(vararg_alloc_size as f64)),
        }));
    }

    Ok((pre_call, post_call, callee_returns_to_address))
}

/// Get the callee name for a direct call, or the single possible target for a
/// function pointer call with exactly one candidate. Returns None otherwise.
fn get_call_callee_name(call: &ir::instructions::Call, ctx: &Context) -> Option<String> {
    match &call.func {
        ir::Value::Function(fv) => Some(fv.name.clone()),
        _ => {
            let signature = FuncTy::new(
                call.return_type.clone(),
                call.params.clone(),
                call.variadic,
            );
            get_func_ptr_signature_info(&signature, ctx).and_then(|(_, could_call)| {
                if could_call.len() == 1 {
                    Some(could_call[0].clone())
                } else {
                    None
                }
            })
        }
    }
}

/// Compute the set of live variables that must be saved around a recursive call.
/// Matches Python's logic in `transComplexCall`.
fn compute_must_store(
    call: &ir::instructions::Call,
    bctx: &BlockInfo,
    following_instrs: &[ir::Instr],
    ctx: &Context,
) -> Result<(Vec<Variable>, Vec<usize>), CompException> {
    let result_name = call.result.as_ref().map(|r| r.name.as_str());
    let next_var_use = compute_next_var_use(
        following_instrs,
        bctx.label.as_deref().unwrap_or(""),
        &bctx.fn_info.phi_info,
        &bctx.fn_info.block_var_use,
        result_name,
    )?;

    let mut must_store: Vec<Variable> = Vec::new();
    let mut must_store_sizes: Vec<usize> = Vec::new();

    for var_name in &next_var_use.depends {
        if ctx.cfg.use_branch_jump_table && bctx.available_params.iter().any(|p| p.var_name == *var_name) {
            continue;
        }

        let is_param = bctx.available_params.iter().any(|p| p.var_name == *var_name);
        let var = Variable {
            var_name: var_name.clone(),
            var_type: if is_param { VarType::Param } else { VarType::Var },
            fn_name: Some(bctx.fn_info.name.clone()),
        };
        let size = next_var_use.depends_var_sizes.get(var_name).copied().unwrap_or(1);
        must_store.push(var);
        must_store_sizes.push(size);
    }

    // Sort numeric names by numeric value first, then alphabetically, to match Python.
    let mut indexed: Vec<(Variable, usize)> = must_store.into_iter().zip(must_store_sizes.into_iter()).collect();
    indexed.sort_by(|(a, _), (b, _)| {
        let a_is_digits = a.var_name.chars().all(|c| c.is_ascii_digit());
        let b_is_digits = b.var_name.chars().all(|c| c.is_ascii_digit());
        match (a_is_digits, b_is_digits) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => {
                let a_num: i64 = a.var_name.parse().unwrap_or(0);
                let b_num: i64 = b.var_name.parse().unwrap_or(0);
                a_num.cmp(&b_num)
            }
            (false, false) => a.var_name.cmp(&b.var_name),
        }
    });
    let mut sorted_must_store: Vec<Variable> = Vec::with_capacity(indexed.len());
    let mut sorted_sizes: Vec<usize> = Vec::with_capacity(indexed.len());
    for (var, size) in indexed {
        sorted_must_store.push(var);
        sorted_sizes.push(size);
    }

    // Add special locals that may be clobbered by recursion.
    if bctx.fn_info.total_alloca_size.is_none() && !bctx.fn_info.skip_stack_size_change {
        sorted_must_store.push(Variable {
            var_name: ctx.cfg.previous_stack_size_local.clone(),
            var_type: VarType::Var,
            fn_name: Some(bctx.fn_info.name.clone()),
        });
        sorted_sizes.push(1);
    }
    if bctx.fn_info.takes_return_address {
        sorted_must_store.push(Variable {
            var_name: ctx.cfg.return_address_local.clone(),
            var_type: VarType::Var,
            fn_name: Some(bctx.fn_info.name.clone()),
        });
        sorted_sizes.push(1);
    }
    if bctx.fn_info.is_variadic {
        sorted_must_store.push(Variable {
            var_name: ctx.cfg.vararg_ptr_local.clone(),
            var_type: VarType::Var,
            fn_name: Some(bctx.fn_info.name.clone()),
        });
        sorted_sizes.push(1);
    }
    if ctx.cfg.use_branch_jump_table {
        sorted_must_store.push(Variable {
            var_name: ctx.cfg.branch_jump_table_addr_local.clone(),
            var_type: VarType::Var,
            fn_name: Some(bctx.fn_info.name.clone()),
        });
        sorted_sizes.push(1);
    }

    Ok((sorted_must_store, sorted_sizes))
}

/// Resolve return-address-related info for a call. Returns None for intrinsics.
fn get_call_return_addr_info(
    call: &ir::instructions::Call,
    ctx: &Context,
) -> Option<(bool, bool, Vec<String>)> {
    if call.intrinsic.is_some() {
        return None;
    }
    match &call.func {
        ir::Value::Function(fv) => {
            ctx.fn_info.get(&fv.name).map(|info| (
                info.returns_to_address,
                info.takes_return_address,
                info.return_addresses.clone(),
            ))
        }
        _ => {
            let signature = FuncTy::new(
                call.return_type.clone(),
                call.params.clone(),
                call.variadic,
            );
            get_func_ptr_signature_info(&signature, ctx).and_then(|(signature_id, could_call)| {
                if could_call.len() == 1 {
                    ctx.fn_info.get(&could_call[0]).map(|info| (
                        info.returns_to_address,
                        info.takes_return_address,
                        info.return_addresses.clone(),
                    ))
                } else {
                    ctx.fn_ptr_sig_info.get(signature_id).map(|info| (
                        info.returns_to_address,
                        info.takes_return_address,
                        info.return_addresses.clone(),
                    ))
                }
            })
        }
    }
}

const LN2: f64 = 0.6931471805599453;

fn pow2(exponent: Value) -> Value {
    Value::Op(Op::Exp(Box::new(Value::Op(Op::Mul(
        Box::new(Value::Known(KnownVal::Num(LN2))),
        Box::new(exponent),
    )))))
}

fn i64_bits_to_double(low: Value, high: Value) -> Value {
    let sign = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
        Box::new(high.clone()),
        Box::new(Value::Known(KnownVal::Num(32768.0))),
    )))));
    let sign_val = Value::Op(Op::Sub(
        Box::new(Value::Known(KnownVal::Num(1.0))),
        Box::new(Value::Op(Op::Mul(
            Box::new(Value::Known(KnownVal::Num(2.0))),
            Box::new(sign),
        ))),
    ));
    let exponent = Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
            Box::new(high.clone()),
            Box::new(Value::Known(KnownVal::Num(16.0))),
        )))))),
        Box::new(Value::Known(KnownVal::Num(2048.0))),
    ));
    let mant_high = Value::Op(Op::Mod(
        Box::new(high),
        Box::new(Value::Known(KnownVal::Num(16.0))),
    ));
    let mantissa = Value::Op(Op::Add(
        Box::new(low),
        Box::new(Value::Op(Op::Mul(
            Box::new(mant_high),
            Box::new(Value::Known(KnownVal::Num(281474976710656.0))),
        ))),
    ));
    let is_zero_exp = Value::BoolOp(BoolOp::Eq(
        Box::new(exponent.clone()),
        Box::new(Value::Known(KnownVal::Num(0.0))),
    ));
    let is_max_exp = Value::BoolOp(BoolOp::Eq(
        Box::new(exponent.clone()),
        Box::new(Value::Known(KnownVal::Num(2047.0))),
    ));
    let is_zero_mant = Value::BoolOp(BoolOp::Eq(
        Box::new(mantissa.clone()),
        Box::new(Value::Known(KnownVal::Num(0.0))),
    ));
    let is_inf = Value::BoolOp(BoolOp::And(
        Box::new(is_max_exp.clone()),
        Box::new(is_zero_mant.clone()),
    ));
    let is_nan = Value::BoolOp(BoolOp::And(
        Box::new(is_max_exp),
        Box::new(Value::BoolOp(BoolOp::Not(Box::new(is_zero_mant)))),
    ));
    let two_pow_m52 = Value::Known(KnownVal::Num(2f64.powi(-52)));
    let one_plus_mant = Value::Op(Op::Add(
        Box::new(Value::Known(KnownVal::Num(1.0))),
        Box::new(Value::Op(Op::Mul(Box::new(mantissa.clone()), Box::new(two_pow_m52)))),
    ));
    let two_pow_exp_minus_1023 = pow2(Value::Op(Op::Sub(
        Box::new(exponent.clone()),
        Box::new(Value::Known(KnownVal::Num(1023.0))),
    )));
    let normal_result = Value::Op(Op::Mul(
        Box::new(sign_val.clone()),
        Box::new(Value::Op(Op::Mul(
            Box::new(one_plus_mant),
            Box::new(two_pow_exp_minus_1023),
        ))),
    ));
    let denorm_result = Value::Op(Op::Mul(
        Box::new(sign_val.clone()),
        Box::new(Value::Op(Op::Mul(
            Box::new(mantissa),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(-1022 - 52)))),
        ))),
    ));
    let inf_val = Value::Op(Op::Div(
        Box::new(sign_val),
        Box::new(Value::Known(KnownVal::Num(0.0))),
    ));
    let nan_val = Value::Op(Op::Div(
        Box::new(Value::Known(KnownVal::Num(0.0))),
        Box::new(Value::Known(KnownVal::Num(0.0))),
    ));
    let not_zero_exp = Value::BoolOp(BoolOp::Not(Box::new(is_zero_exp.clone())));
    let exp_choice = Value::Op(Op::Add(
        Box::new(Value::Op(Op::Mul(
            Box::new(Value::Op(Op::BoolToFloat(Box::new(not_zero_exp)))),
            Box::new(normal_result),
        ))),
        Box::new(Value::Op(Op::Mul(
            Box::new(Value::Op(Op::BoolToFloat(Box::new(is_zero_exp.clone())))),
            Box::new(denorm_result),
        ))),
    ));
    let not_nan_or_inf = Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Or(
        Box::new(is_inf.clone()),
        Box::new(is_nan.clone()),
    )))));
    Value::Op(Op::Add(
        Box::new(Value::Op(Op::Mul(
            Box::new(Value::Op(Op::BoolToFloat(Box::new(not_nan_or_inf)))),
            Box::new(exp_choice),
        ))),
        Box::new(Value::Op(Op::Add(
            Box::new(Value::Op(Op::Mul(
                Box::new(Value::Op(Op::BoolToFloat(Box::new(is_inf)))),
                Box::new(inf_val),
            ))),
            Box::new(Value::Op(Op::Mul(
                Box::new(Value::Op(Op::BoolToFloat(Box::new(is_nan)))),
                Box::new(nan_val),
            ))),
        ))),
    ))
}

fn double_to_i64_bits(val: Value) -> (Value, Value) {
    let zero = Value::Known(KnownVal::Num(0.0));
    let sign = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Lt(
        Box::new(val.clone()),
        Box::new(zero.clone()),
    )))));
    let abs_val = Value::Op(Op::Abs(Box::new(val.clone())));
    let is_zero = Value::BoolOp(BoolOp::Eq(
        Box::new(abs_val.clone()),
        Box::new(zero.clone()),
    ));
    let log2_abs = Value::Op(Op::Div(
        Box::new(Value::Op(Op::Ln(Box::new(abs_val.clone())))),
        Box::new(Value::Known(KnownVal::Num(LN2))),
    ));
    let exp2 = Value::Op(Op::Floor(Box::new(log2_abs)));
    let biased_exp = Value::Op(Op::Add(
        Box::new(exp2.clone()),
        Box::new(Value::Known(KnownVal::Num(1023.0))),
    ));
    let pow2_exp2 = pow2(exp2);
    let mant_norm = Value::Op(Op::Div(Box::new(abs_val), Box::new(pow2_exp2)));
    let mant_bits = Value::Op(Op::Mul(
        Box::new(Value::Op(Op::Sub(
            Box::new(mant_norm),
            Box::new(Value::Known(KnownVal::Num(1.0))),
        ))),
        Box::new(Value::Known(KnownVal::Num(4503599627370496.0))),
    ));
    let not_zero = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Not(
        Box::new(is_zero.clone()),
    )))));
    let combine_zero = Value::Op(Op::Mul(
        Box::new(Value::Op(Op::BoolToFloat(Box::new(is_zero.clone())))),
        Box::new(zero.clone()),
    ));
    let sign_shifted = Value::Op(Op::Mul(
        Box::new(sign),
        Box::new(Value::Known(KnownVal::Num(9223372036854775808.0))),
    ));
    let exp_shifted = Value::Op(Op::Mul(
        Box::new(biased_exp),
        Box::new(Value::Known(KnownVal::Num(4503599627370496.0))),
    ));
    let int_bits = Value::Op(Op::Add(
        Box::new(sign_shifted),
        Box::new(Value::Op(Op::Add(
            Box::new(exp_shifted),
            Box::new(mant_bits),
        ))),
    ));
    let combine_val = Value::Op(Op::Mul(
        Box::new(not_zero),
        Box::new(int_bits),
    ));
    let combined = Value::Op(Op::Add(
        Box::new(combine_zero),
        Box::new(combine_val),
    ));
    let base48 = Value::Known(KnownVal::Num(281474976710656.0));
    let high_mask = Value::Known(KnownVal::Num(65536.0));
    let low = Value::Op(Op::Mod(
        Box::new(combined.clone()),
        Box::new(base48),
    ));
    let high = Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
            Box::new(combined),
            Box::new(Value::Known(KnownVal::Num(281474976710656.0))),
        )))))),
        Box::new(high_mask),
    ));
    (low, high)
}

fn trans_instr(
    instr: &ir::Instr,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();

    match instr {
        ir::Instr::Alloca(alloc) => {
            let size = memory::get_size_of(&alloc.allocated_type, ctx.cfg.accurate_byte_spacing)?;
            let result_var = Variable {
                var_name: localize_var(&alloc.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            let offset = if bctx.fn_info.skip_stack_size_change {
                0isize
            } else if bctx.fn_info.total_alloca_size.is_none() {
                -(bctx.fn_info.block_alloca_size.get(&bctx.label.clone().unwrap_or_default()).copied().unwrap_or(0) as isize)
            } else {
                -(bctx.fn_info.total_alloca_size.unwrap_or(0) as isize)
            };
            let final_offset = offset + bctx.allocated as isize + size as isize - 1;

            let stack_ptr_val = Value::GetVar { name: ctx.cfg.stack_pointer_var.clone() };
            // Match Python's offsetStackSize(stack_pointer_var, -final_offset) behavior:
            // negate the offset so stack grows backward.
            let offset_val = if final_offset > 0 {
                Value::Op(Op::Sub(
                    Box::new(stack_ptr_val),
                    Box::new(Value::Known(KnownVal::Num(final_offset as f64))),
                ))
            } else if final_offset < 0 {
                Value::Op(Op::Add(
                    Box::new(stack_ptr_val),
                    Box::new(Value::Known(KnownVal::Num((-final_offset) as f64))),
                ))
            } else {
                stack_ptr_val
            };

            blocks.add_block(result_var.set_value(offset_val, VarOp::Set, None)?);

            bctx.allocated += size;
        }

        ir::Instr::Load(load) => {
            let address = trans_value(&load.address, ctx, Some(bctx))?.into_single()?;
            let result_var = Variable {
                var_name: localize_var(&load.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(trans_load(&result_var, address, &load.loaded_type, ctx)?);
        }

        ir::Instr::Store(store) => {
            let address = trans_value(&store.address, ctx, Some(bctx))?.into_single()?;
            let value = trans_value(&store.value, ctx, Some(bctx))?;
            blocks.add(trans_store(value, address, store.value.type_(), ctx)?);
        }

        ir::Instr::BinaryOp(bop) => {
            let lft_iv = trans_value(&bop.left, ctx, Some(bctx))?;
            let rgt_iv = trans_value(&bop.right, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&bop.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            let mut res_val: Option<InferredValue> = None;

            match bop.opcode {
                ir::instructions::BinaryOpcode::Add | ir::instructions::BinaryOpcode::Sub => {
                    let op_str = if bop.opcode == ir::instructions::BinaryOpcode::Add { "add" } else { "sub" };
                    let width = get_value_width(&bop.left);
                    let (val, extra_blocks) = calculate_sum_diff(op_str, lft_iv, rgt_iv, width, ctx, bop.is_nuw)?;
                    blocks.add(extra_blocks);
                    res_val = Some(val);
                }
                ir::instructions::BinaryOpcode::Shl => {
                    let width = get_value_width(&bop.left);
                    let can_shift_out = !(bop.is_nsw && bop.is_nuw);
                    let (val, extra_blocks) = bit_shift("left", width, lft_iv, rgt_iv, ctx, can_shift_out)?;
                    blocks.add(extra_blocks);
                    res_val = Some(val);
                }
                ir::instructions::BinaryOpcode::LShr => {
                    let width = get_value_width(&bop.left);
                    let can_shift_out = !bop.is_exact;
                    let (val, extra_blocks) = bit_shift("right", width, lft_iv, rgt_iv, ctx, can_shift_out)?;
                    blocks.add(extra_blocks);
                    res_val = Some(val);
                }
                ir::instructions::BinaryOpcode::AShr => {
                    let width = get_value_width(&bop.left);
                    if width > super::config::VARIABLE_MAX_BITS {
                        let shift_single = match &rgt_iv {
                            InferredValue::Indexed(idx) => idx.vals[0].clone(),
                            InferredValue::Single(v) => v.clone(),
                        };
                        if width == 64 && shift_single == Value::Known(KnownVal::Num(32.0)) {
                            let lft_idx = match lft_iv {
                                InferredValue::Indexed(iv) => iv,
                                _ => return Err(CompException(
                                    "Expected indexed value for wide ashr".to_string(),
                                )),
                            };
                            res_val = Some(InferredValue::Indexed(ashr_i64_by_32(&lft_idx)?));
                        } else {
                            return Err(CompException(format!(
                                "AShr of {}-bit integers by {:?} is not supported",
                                width, shift_single
                            )));
                        }
                    } else {
                        let lft = lft_iv.into_single()?;
                        let rgt = rgt_iv.into_single()?;
                        blocks.add(ashr_blocks(&res_var, lft, rgt, width, bop.is_exact, ctx)?);
                    }
                }
                _ => {
                    match bop.opcode {
                        ir::instructions::BinaryOpcode::And
                        | ir::instructions::BinaryOpcode::Or
                        | ir::instructions::BinaryOpcode::Xor => {
                            let width = get_value_width(&bop.left);
                            res_val = Some(trans_bitwise_binop(
                                bop.opcode, lft_iv, rgt_iv, width, ctx,
                            )?);
                        }
                        ir::instructions::BinaryOpcode::Mul => {
                            let width = get_value_width(&bop.left);
                            if width > super::config::VARIABLE_MAX_BITS {
                                let lft_idx = match lft_iv {
                                    InferredValue::Indexed(iv) => iv,
                                    _ => return Err(CompException(
                                        "Expected indexed value for wide multiplication".to_string(),
                                    )),
                                };
                                let rgt_idx = match rgt_iv {
                                    InferredValue::Indexed(iv) => iv,
                                    _ => return Err(CompException(
                                        "Expected indexed value for wide multiplication".to_string(),
                                    )),
                                };
                                res_val = Some(InferredValue::Indexed(multiply_wide_u64(
                                    &lft_idx, &rgt_idx,
                                )?));
                            } else {
                                let lft = lft_iv.into_single()?;
                                let rgt = rgt_iv.into_single()?;
                                if bop.is_nuw && bop.is_nsw {
                                    res_val = Some(InferredValue::Single(multiply_no_wrap(lft, rgt, width)?));
                                } else {
                                    let (val, extra_blocks) = multiply_wrap(lft, rgt, width, ctx)?;
                                    blocks.add(extra_blocks);
                                    res_val = Some(InferredValue::Single(val));
                                }
                            }
                        }
                        _ => {
                            let width = get_value_width(&bop.left);
                            if width > super::config::VARIABLE_MAX_BITS && matches!(&lft_iv, InferredValue::Indexed(_)) {
                                let lft_idx = match lft_iv {
                                    InferredValue::Indexed(iv) => iv,
                                    _ => unreachable!(),
                                };
                                let rgt_idx = match rgt_iv {
                                    InferredValue::Indexed(iv) => iv,
                                    _ => IdxbleValue { vals: vec![rgt_iv.into_single()?] },
                                };
                                match bop.opcode {
                                    ir::instructions::BinaryOpcode::UDiv => {
                                        res_val = Some(wide_udiv(&lft_idx, &rgt_idx, width)?);
                                    }
                                    ir::instructions::BinaryOpcode::URem => {
                                        let (_, rem) = wide_udiv_with_rem(&lft_idx, &rgt_idx, width)?;
                                        res_val = Some(rem);
                                    }
                                    _ => {
                                        return Err(CompException(format!(
                                            "{}-bit wide integer operation {:?} is not supported",
                                            width, bop.opcode
                                        )));
                                    }
                                }
                            } else {
                                let lft = lft_iv.into_single()?;
                                let rgt = rgt_iv.into_single()?;
                                match bop.opcode {
                                    ir::instructions::BinaryOpcode::UDiv => {
                                        let div = Value::Op(Op::Div(Box::new(lft), Box::new(rgt)));
                                        let res = if bop.is_exact {
                                            div
                                        } else {
                                            Value::Op(Op::Floor(Box::new(div)))
                                        };
                                        res_val = Some(InferredValue::Single(res));
                                    }
                                    ir::instructions::BinaryOpcode::SDiv => {
                                        blocks.add(signed_div_blocks(&res_var, lft, rgt, width, bop.is_exact, ctx)?);
                                    }
                                    ir::instructions::BinaryOpcode::URem => {
                                        res_val = Some(InferredValue::Single(Value::Op(Op::Mod(Box::new(lft), Box::new(rgt)))));
                                    }
                                    ir::instructions::BinaryOpcode::SRem => {
                                        blocks.add(signed_rem_blocks(&res_var, lft, rgt, width, ctx)?);
                                    }
                                    _ => {
                                        res_val = handle_arith_binop(bop.opcode, lft, rgt, &res_var, &mut blocks, ctx)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some(rv) = res_val {
                blocks.add(res_var.set_inferred_value(rv)?);
            }
        }

        ir::Instr::UnaryOp(uop) => {
            let operand_iv = trans_value(&uop.operand, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&uop.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            match uop.opcode {
                ir::instructions::UnaryOpcode::FNeg => {
                    let operand = operand_iv.into_single()?;
                    blocks.add_block(res_var.set_value(
                        Value::Op(Op::Sub(
                            Box::new(Value::Known(KnownVal::Num(0.0))),
                            Box::new(operand),
                        )),
                        VarOp::Set,
                        None,
                    )?);
                }
            }
        }

        ir::Instr::ICmp(icmp) => {
            let lft_iv = trans_value(&icmp.left, ctx, Some(bctx))?;
            let rgt_iv = trans_value(&icmp.right, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&icmp.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            let width = get_value_width(&icmp.left);
            let is_wide = width > super::config::VARIABLE_MAX_BITS
                || matches!(lft_iv, InferredValue::Indexed(_))
                || matches!(rgt_iv, InferredValue::Indexed(_));

            if is_wide {
                let lft_idx = lft_iv.into_indexed()?;
                let rgt_idx = rgt_iv.into_indexed()?;
                let (maybe_cond, cmp_blocks) = large_int_compare(&lft_idx, &rgt_idx, width, icmp.cond, ctx, &res_var)?;
                blocks.add(cmp_blocks);
                if let Some(cond) = maybe_cond {
                    let cond_val = Value::Op(Op::BoolToFloat(Box::new(cond)));
                    blocks.add_block(res_var.set_value(cond_val, VarOp::Set, None)?);
                }
            } else {
                let lft = lft_iv.into_single()?;
                let rgt = rgt_iv.into_single()?;
                let cond_bool = match icmp.cond {
                    ir::instructions::ICmpCond::Eq => BoolOp::Eq(Box::new(lft), Box::new(rgt)),
                    ir::instructions::ICmpCond::Ne => BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Eq(Box::new(lft), Box::new(rgt))))),
                    ir::instructions::ICmpCond::Sgt => {
                        let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
                        let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
                        BoolOp::Gt(Box::new(sl), Box::new(sr))
                    }
                    ir::instructions::ICmpCond::Sge => {
                        let special_signed_handling = ctx.cfg.compiler_opt
                            && !matches!(lft, Value::Known(_))
                            && !matches!(rgt, Value::Known(_));
                        if special_signed_handling {
                            let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
                            let sr = simplify_value(&twos_complement::reverse_twos_complement_and_sub_half(rgt, width));
                            BoolOp::Gt(Box::new(sl), Box::new(sr))
                        } else {
                            let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
                            let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
                            match (&sl, &sr) {
                                (Value::Known(KnownVal::Num(a)), _) => {
                                    BoolOp::Gt(
                                        Box::new(Value::Known(KnownVal::Num(*a + 1.0))),
                                        Box::new(sr),
                                    )
                                }
                                (_, Value::Known(KnownVal::Num(b))) => {
                                    BoolOp::Gt(
                                        Box::new(sl),
                                        Box::new(Value::Known(KnownVal::Num(*b - 1.0))),
                                    )
                                }
                                _ => BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Lt(Box::new(sl), Box::new(sr)))))
                            }
                        }
                    }
                    ir::instructions::ICmpCond::Slt => {
                        let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
                        let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
                        BoolOp::Lt(Box::new(sl), Box::new(sr))
                    }
                    ir::instructions::ICmpCond::Sle => {
                        let special_signed_handling = ctx.cfg.compiler_opt
                            && !matches!(lft, Value::Known(_))
                            && !matches!(rgt, Value::Known(_));
                        if special_signed_handling {
                            let sl = simplify_value(&twos_complement::reverse_twos_complement_and_sub_half(lft, width));
                            let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
                            BoolOp::Lt(Box::new(sl), Box::new(sr))
                        } else {
                            let sl = simplify_value(&twos_complement::reverse_twos_complement(lft, width));
                            let sr = simplify_value(&twos_complement::reverse_twos_complement(rgt, width));
                            match (&sl, &sr) {
                                (Value::Known(KnownVal::Num(a)), _) => {
                                    BoolOp::Lt(
                                        Box::new(Value::Known(KnownVal::Num(*a - 1.0))),
                                        Box::new(sr),
                                    )
                                }
                                (_, Value::Known(KnownVal::Num(b))) => {
                                    BoolOp::Lt(
                                        Box::new(sl),
                                        Box::new(Value::Known(KnownVal::Num(*b + 1.0))),
                                    )
                                }
                                _ => BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(Box::new(sl), Box::new(sr)))))
                            }
                        }
                    }
                    ir::instructions::ICmpCond::Ugt => BoolOp::Gt(Box::new(lft), Box::new(rgt)),
                    ir::instructions::ICmpCond::Uge => BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Lt(Box::new(lft), Box::new(rgt))))),
                    ir::instructions::ICmpCond::Ult => BoolOp::Lt(Box::new(lft), Box::new(rgt)),
                    ir::instructions::ICmpCond::Ule => BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Gt(Box::new(lft), Box::new(rgt))))),
                };

                // Match Python: cast boolean comparison result to a float (0/1).
                let cond_val = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(cond_bool))));
                blocks.add_block(res_var.set_value(cond_val, VarOp::Set, None)?);
            }
        }

        ir::Instr::FCmp(fcmp) => {
            let lft = trans_value(&fcmp.left, ctx, Some(bctx))?.into_single()?;
            let rgt = trans_value(&fcmp.right, ctx, Some(bctx))?.into_single()?;
            let res_var = Variable {
                var_name: localize_var(&fcmp.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            // NaN checks matching Python's lexical comparison.
            // In Scratch string comparison: "Infinity" < "j" < "NaN".
            let is_not_nan = |val: Value| {
                Value::BoolOp(BoolOp::Lt(
                    Box::new(val),
                    Box::new(Value::Known(KnownVal::Str("j".to_string()))),
                ))
            };
            let is_nan = |val: Value| {
                Value::BoolOp(BoolOp::Gt(
                    Box::new(val),
                    Box::new(Value::Known(KnownVal::Num(f64::INFINITY))),
                ))
            };
            let both_not_nan = Value::BoolOp(BoolOp::And(
                Box::new(is_not_nan(lft.clone())),
                Box::new(is_not_nan(rgt.clone())),
            ));
            let either_nan = Value::BoolOp(BoolOp::Or(
                Box::new(is_nan(lft.clone())),
                Box::new(is_nan(rgt.clone())),
            ));
            let cmp_eq = Value::BoolOp(BoolOp::Eq(Box::new(lft.clone()), Box::new(rgt.clone())));
            let cmp_gt = Value::BoolOp(BoolOp::Gt(Box::new(lft.clone()), Box::new(rgt.clone())));
            let cmp_lt = Value::BoolOp(BoolOp::Lt(Box::new(lft.clone()), Box::new(rgt.clone())));

            let cond_bool = match fcmp.cond {
                ir::instructions::FCmpCond::TrueCond => Value::KnownBool(true),
                ir::instructions::FCmpCond::FalseCond => Value::KnownBool(false),
                ir::instructions::FCmpCond::Oeq => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(is_not_nan(rgt.clone())),
                        Box::new(cmp_eq),
                    ))
                }
                ir::instructions::FCmpCond::One => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(both_not_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_eq.clone())))),
                    ))
                }
                ir::instructions::FCmpCond::Ogt => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(both_not_nan.clone()),
                        Box::new(cmp_gt.clone()),
                    ))
                }
                ir::instructions::FCmpCond::Oge => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(both_not_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_lt.clone())))),
                    ))
                }
                ir::instructions::FCmpCond::Olt => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(both_not_nan.clone()),
                        Box::new(cmp_lt.clone()),
                    ))
                }
                ir::instructions::FCmpCond::Ole => {
                    Value::BoolOp(BoolOp::And(
                        Box::new(both_not_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_gt.clone())))),
                    ))
                }
                ir::instructions::FCmpCond::Ord => both_not_nan,
                ir::instructions::FCmpCond::Uno => either_nan.clone(),
                ir::instructions::FCmpCond::Ueq => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(cmp_eq),
                    ))
                }
                ir::instructions::FCmpCond::Une => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_eq.clone())))),
                    ))
                }
                ir::instructions::FCmpCond::Ugt => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(cmp_gt),
                    ))
                }
                ir::instructions::FCmpCond::Uge => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_lt)))),
                    ))
                }
                ir::instructions::FCmpCond::Ult => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(cmp_lt),
                    ))
                }
                ir::instructions::FCmpCond::Ule => {
                    Value::BoolOp(BoolOp::Or(
                        Box::new(either_nan.clone()),
                        Box::new(Value::BoolOp(BoolOp::Not(Box::new(cmp_gt)))),
                    ))
                }
            };

            let cond_val = match cond_bool {
                Value::KnownBool(b) => Value::Known(KnownVal::Num(if b { 1.0 } else { 0.0 })),
                _ => Value::Op(Op::BoolToFloat(Box::new(cond_bool))),
            };
            blocks.add_block(res_var.set_value(cond_val, VarOp::Set, None)?);
        }

        ir::Instr::Conversion(conv) => {
            let val_iv = trans_value(&conv.value, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&conv.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            let res_val: InferredValue = match conv.opcode {
                ir::instructions::ConvOpcode::Trunc => {
                    let width = get_type_width(&conv.res_type);
                    match val_iv {
                        InferredValue::Single(v) => InferredValue::Single(twos_complement::apply_twos_complement(v, width)),
                        InferredValue::Indexed(idx) => {
                            // The lowest part holds the least-significant VARIABLE_MAX_BITS bits,
                            // which is sufficient for any truncation to <= VARIABLE_MAX_BITS bits.
                            let low = idx.vals[0].clone();
                            InferredValue::Single(twos_complement::apply_twos_complement(low, width))
                        }
                    }
                }
                ir::instructions::ConvOpcode::ZExt => {
                    let to_width = get_type_width(&conv.res_type);
                    if to_width > super::config::VARIABLE_MAX_BITS {
                        // Zero-extend a narrow value into a wide integer represented as
                        // multiple Scratch variables.  Only the low chunk gets the value;
                        // higher chunks are zero.
                        let low = val_iv.into_single()?;
                        let mut vals = vec![low];
                        let chunks = (to_width + super::config::VARIABLE_MAX_BITS - 1)
                            / super::config::VARIABLE_MAX_BITS;
                        vals.resize(chunks, Value::Known(KnownVal::Num(0.0)));
                        InferredValue::Indexed(IdxbleValue { vals })
                    } else {
                        let val = val_iv.into_single()?;
                        InferredValue::Single(val)
                    }
                }
                ir::instructions::ConvOpcode::SExt => {
                    let val = val_iv.into_single()?;
                    let from_bits = get_value_width(&conv.value);
                    let to_bits = get_type_width(&conv.res_type);
                    let limit = 2f64.powi(from_bits as i32 - 1) - 1.0;
                    let diff = (2f64.powi(to_bits as i32) - 1.0) - (2f64.powi(from_bits as i32) - 1.0);
                    InferredValue::Single(Value::Op(Op::Add(
                        Box::new(val.clone()),
                        Box::new(Value::Op(Op::Mul(
                            Box::new(Value::Known(KnownVal::Num(diff))),
                            Box::new(Value::BoolOp(BoolOp::Gt(
                                Box::new(val),
                                Box::new(Value::Known(KnownVal::Num(limit))),
                            ))),
                        ))),
                    )))
                }
                ir::instructions::ConvOpcode::IntToPtr | ir::instructions::ConvOpcode::PtrToInt => {
                    InferredValue::Single(val_iv.into_single()?)
                }
                ir::instructions::ConvOpcode::BitCast => {
                    let src_width = get_value_width(&conv.value);
                    let dst_width = get_type_width(&conv.res_type);
                    match (src_width, dst_width) {
                        (64, 64) => {
                            match val_iv {
                                InferredValue::Indexed(idx) => {
                                    let low = idx.vals[0].clone();
                                    let high = idx.vals[1].clone();
                                    InferredValue::Single(i64_bits_to_double(low, high))
                                }
                                InferredValue::Single(v) => {
                                    let dst_is_float = conv.res_type.is_floating_point();
                                    if dst_is_float {
                                        InferredValue::Single(v)
                                    } else {
                                        let (low, high) = double_to_i64_bits(v);
                                        InferredValue::Indexed(IdxbleValue { vals: vec![low, high] })
                                    }
                                }
                            }
                        }
                        _ => {
                            InferredValue::Single(val_iv.into_single()?)
                        }
                    }
                }
                ir::instructions::ConvOpcode::SIToFP => {
                    let width = get_value_width(&conv.value);
                    if width == 64 {
                        let idx = val_iv.into_indexed()?;
                        let low = idx.vals[0].clone();
                        let high = idx.vals[1].clone();
                        let base48 = Value::Known(KnownVal::Num(281474976710656.0)); // 2^48
                        let combined = Value::Op(Op::Add(
                            Box::new(low),
                            Box::new(Value::Op(Op::Mul(Box::new(high), Box::new(base48)))),
                        ));
                        InferredValue::Single(twos_complement::undo_twos_complement(combined, 64))
                    } else {
                        let val = val_iv.into_single()?;
                        InferredValue::Single(twos_complement::undo_twos_complement(val, width))
                    }
                }
                ir::instructions::ConvOpcode::FPToSI => {
                    let val = val_iv.into_single()?;
                    let width = get_type_width(&conv.res_type);
                    let abs_val = Value::Op(Op::Abs(Box::new(val.clone())));
                    let floored_abs = Value::Op(Op::Floor(Box::new(abs_val)));
                    let sign = Value::Op(Op::Sub(
                        Box::new(Value::Op(Op::Mul(
                            Box::new(Value::BoolOp(BoolOp::Gt(
                                Box::new(val.clone()),
                                Box::new(Value::Known(KnownVal::Num(0.0))),
                            ))),
                            Box::new(Value::Known(KnownVal::Num(2.0))),
                        ))),
                        Box::new(Value::Known(KnownVal::Num(1.0))),
                    ));
                    let signed = Value::Op(Op::Mul(Box::new(floored_abs), Box::new(sign)));
                    let wrapped = twos_complement::apply_twos_complement(signed, width);

                    if width == 64 {
                        let base48 = Value::Known(KnownVal::Num(281474976710656.0)); // 2^48
                        let high_mask = Value::Known(KnownVal::Num(65536.0)); // 2^16
                        let low = Value::Op(Op::Mod(Box::new(wrapped.clone()), Box::new(base48.clone())));
                        let high = Value::Op(Op::Mod(
                            Box::new(Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                                Box::new(wrapped),
                                Box::new(base48),
                            )))))),
                            Box::new(high_mask),
                        ));
                        InferredValue::Indexed(IdxbleValue { vals: vec![low, high] })
                    } else {
                        InferredValue::Single(wrapped)
                    }
                }
                ir::instructions::ConvOpcode::FPToUI => {
                    let val = val_iv.into_single()?;
                    InferredValue::Single(Value::Op(Op::Floor(Box::new(val))))
                }
                ir::instructions::ConvOpcode::UIToFP => {
                    InferredValue::Single(val_iv.into_single()?)
                }
                ir::instructions::ConvOpcode::FPTrunc | ir::instructions::ConvOpcode::FPExt => {
                    InferredValue::Single(val_iv.into_single()?)
                }
                _ => return Err(CompException(format!("Unsupported conversion opcode: {:?}", conv.opcode))),
            };

            blocks.add(res_var.set_inferred_value(res_val)?);
        }

        ir::Instr::GetElementPtr(gep) => {
            let final_val = trans_gep_value(
                &gep.base_ptr,
                &gep.base_ptr_type,
                &gep.indices,
                gep.is_nuw,
                ctx,
                Some(bctx),
                false,
            )?;
            let res_var = Variable {
                var_name: localize_var(&gep.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add_block(res_var.set_value(final_val, VarOp::Set, None)?);
        }

        ir::Instr::ExtractValue(ev) => {
            let agg_iv = trans_value(&ev.agg, ctx, Some(bctx))?;
            let agg_vals = inferred_to_values(agg_iv);
            let indices: Vec<usize> = ev.indices.iter().map(|&i| i as usize).collect();
            let offset_result = memory::get_agg_offset(ev.agg.type_(), &indices, false)?;
            let offset = offset_result.offset as usize;
            let size = offset_result.size;
            let res_vals: Vec<Value> = agg_vals[offset..offset + size].to_vec();
            let res_iv = if res_vals.len() == 1 {
                InferredValue::Single(res_vals[0].clone())
            } else {
                InferredValue::Indexed(IdxbleValue { vals: res_vals })
            };
            let res_var = Variable {
                var_name: localize_var(&ev.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(res_var.set_inferred_value(res_iv)?);
        }

        ir::Instr::InsertValue(iv) => {
            let agg_iv = trans_value(&iv.agg, ctx, Some(bctx))?;
            let mut agg_vals = inferred_to_values(agg_iv);
            let el_iv = trans_value(&iv.element, ctx, Some(bctx))?;
            let mut el_vals = inferred_to_values(el_iv);
            let indices: Vec<usize> = iv.indices.iter().map(|&i| i as usize).collect();
            let offset_result = memory::get_agg_offset(iv.agg.type_(), &indices, false)?;
            let offset = offset_result.offset as usize;
            let size = offset_result.size;
            el_vals.resize(size, Value::Known(KnownVal::Num(0.0)));
            for (i, v) in el_vals.iter().enumerate() {
                agg_vals[offset + i] = v.clone();
            }
            let res_iv = if agg_vals.len() == 1 {
                InferredValue::Single(agg_vals[0].clone())
            } else {
                InferredValue::Indexed(IdxbleValue { vals: agg_vals })
            };
            let res_var = Variable {
                var_name: localize_var(&iv.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(res_var.set_inferred_value(res_iv)?);
        }

        ir::Instr::ExtractElement(ee) => {
            let vec_iv = trans_value(&ee.agg, ctx, Some(bctx))?;
            let idx_iv = trans_value(&ee.index, ctx, Some(bctx))?;
            let idx_val = idx_iv.into_single()?;
            let idx = match &idx_val {
                Value::Known(KnownVal::Num(n)) => {
                    if n.fract() == 0.0 && *n >= 0.0 {
                        Some(*n as usize)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let idx = idx.ok_or_else(|| CompException("Dynamic extractelement index not yet supported".to_string()))?;
            let vec_ty = ee.agg.type_();
            let elem_ty = match vec_ty {
                ir::Type::Vector(vt) => &*vt.inner,
                _ => return Err(CompException("extractelement requires vector type".to_string())),
            };
            let elem_size = memory::get_size_of(elem_ty, false)?;
            let vals = inferred_to_values(vec_iv);
            let offset = idx * elem_size;
            let res_vals: Vec<Value> = vals[offset..offset + elem_size].to_vec();
            let res_iv = if res_vals.len() == 1 {
                InferredValue::Single(res_vals.into_iter().next().unwrap())
            } else {
                InferredValue::Indexed(IdxbleValue { vals: res_vals })
            };
            let res_var = Variable {
                var_name: localize_var(&ee.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(res_var.set_inferred_value(res_iv)?);
        }

        ir::Instr::InsertElement(ie) => {
            let vec_iv = trans_value(&ie.agg, ctx, Some(bctx))?;
            let item_iv = trans_value(&ie.item, ctx, Some(bctx))?;
            let idx_iv = trans_value(&ie.index, ctx, Some(bctx))?;
            let idx_val = idx_iv.into_single()?;
            let idx_known = match &idx_val {
                Value::Known(KnownVal::Num(n)) => {
                    if n.fract() == 0.0 && *n >= 0.0 {
                        Some(*n as usize)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let vec_ty = ie.agg.type_();
            let elem_ty = match vec_ty {
                ir::Type::Vector(vt) => &*vt.inner,
                _ => return Err(CompException("insertelement requires vector type".to_string())),
            };
            let elem_size = memory::get_size_of(elem_ty, false)?;
            match idx_known {
                Some(idx) => {
                    let mut vals = inferred_to_values(vec_iv);
                    let item_vals = inferred_to_values(item_iv);
                    let offset = idx * elem_size;
                    for (i, v) in item_vals.iter().enumerate() {
                        vals[offset + i] = v.clone();
                    }
                    let res_iv = if vals.len() == 1 {
                        InferredValue::Single(vals.into_iter().next().unwrap())
                    } else {
                        InferredValue::Indexed(IdxbleValue { vals })
                    };
                    let res_var = Variable {
                        var_name: localize_var(&ie.result.name, false, Some(&bctx.fn_info.name), false),
                        var_type: VarType::Var,
                        fn_name: None,
                    };
                    blocks.add(res_var.set_inferred_value(res_iv)?);
                }
                None => {
                    return Err(CompException("Dynamic insertelement index not yet supported".to_string()));
                }
            }
        }

        ir::Instr::ShuffleVector(sv) => {
            let v1_iv = trans_value(&sv.fst_vector, ctx, Some(bctx))?;
            let v2_iv = trans_value(&sv.snd_vector, ctx, Some(bctx))?;
            let mask_iv = trans_value(&sv.mask_vector, ctx, Some(bctx))?;
            let vec_ty = sv.fst_vector.type_();
            let elem_ty = match vec_ty {
                ir::Type::Vector(vt) => &*vt.inner,
                _ => return Err(CompException("shufflevector requires vector type".to_string())),
            };
            let elem_size = memory::get_size_of(elem_ty, false)?;
            let res_iv = shuffle_vector(v1_iv, v2_iv, mask_iv, elem_size)?;
            let res_var = Variable {
                var_name: localize_var(&sv.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(res_var.set_inferred_value(res_iv)?);
        }

        ir::Instr::VaArg(va) => {
            let arglist_val = trans_value(&va.arglist, ctx, Some(bctx))?.into_single()?;
            let res_var = Variable {
                var_name: localize_var(&va.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            let arg_size = memory::get_size_of(&va.argty, ctx.cfg.accurate_byte_spacing)?;
            let arg_ptr = Value::GetOfList(GetOfList {
                op: scratch::ast::ListOp::AtIndex,
                name: ctx.cfg.mem_var.clone(),
                value: Box::new(arglist_val.clone()),
            });
            blocks.add(trans_load(&res_var, arg_ptr.clone(), &va.argty, ctx)?);
            let new_ptr = Value::Op(Op::Add(
                Box::new(arg_ptr),
                Box::new(Value::Known(KnownVal::Num(arg_size as f64))),
            ));
            blocks.add_block(Block::EditList(scratch::ast::EditListData {
                op: scratch::ast::ListEditOp::ReplaceAt,
                name: ctx.cfg.mem_var.clone(),
                index: Some(arglist_val),
                value: Some(new_ptr),
            }));
        }

        ir::Instr::Phi(_) => {}

        ir::Instr::Select(sel) => {
            let cond = trans_value(&sel.cond, ctx, Some(bctx))?.into_single()?;
            let true_val = trans_value(&sel.true_value, ctx, Some(bctx))?;
            let false_val = trans_value(&sel.false_value, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&sel.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };

            match (true_val, false_val) {
                (InferredValue::Single(t), InferredValue::Single(f)) => {
                    // Optimise select of two known scalar constants to arithmetic,
                    // matching Python's output structure.
                    if let (
                        Value::Known(KnownVal::Num(true_num)),
                        Value::Known(KnownVal::Num(false_num)),
                    ) = (&t, &f)
                    {
                        if true_num.is_finite() && false_num.is_finite() {
                            let diff = true_num - false_num;
                            let (op, abs_diff) = if diff < 0.0 {
                                (BinaryOpcode::Sub, -diff)
                            } else {
                                (BinaryOpcode::Add, diff)
                            };
                            let offset = if abs_diff == 1.0 {
                                cond.clone()
                            } else {
                                Value::Op(Op::Mul(
                                    Box::new(cond.clone()),
                                    Box::new(Value::Known(KnownVal::Num(abs_diff))),
                                ))
                            };
                            let res_val = Value::Op(match op {
                                BinaryOpcode::Add => Op::Add(Box::new(f.clone()), Box::new(offset)),
                                BinaryOpcode::Sub => Op::Sub(Box::new(f.clone()), Box::new(offset)),
                                _ => unreachable!(),
                            });
                            blocks.add_block(res_var.set_value(res_val, VarOp::Set, None)?);
                            return Ok(blocks);
                        }
                    }

                    blocks.add_block(Block::ControlFlow(ControlFlow {
                        op: ControlOp::IfElse,
                        condition: Some(Value::BoolOp(BoolOp::Eq(Box::new(cond), Box::new(Value::Known(KnownVal::Num(1.0)))))),
                        var: None,
                        body: Some(BlockList::from_blocks(vec![
                            res_var.set_value(t, VarOp::Set, None)?,
                        ])),
                        else_body: Some(BlockList::from_blocks(vec![
                            res_var.set_value(f, VarOp::Set, None)?,
                        ])),
                    }));
                }
                (InferredValue::Indexed(t), InferredValue::Indexed(f)) => {
                    if t.vals.len() != f.vals.len() {
                        return Err(CompException(format!(
                            "Select operands have different chunk counts: {} vs {}",
                            t.vals.len(),
                            f.vals.len()
                        )));
                    }
                    let mut body_blocks = BlockList::new();
                    let mut else_blocks = BlockList::new();
                    for (i, (tv, fv)) in t.vals.iter().zip(f.vals.iter()).enumerate() {
                        body_blocks.add_block(res_var.set_value(tv.clone(), VarOp::Set, Some(i))?);
                        else_blocks.add_block(res_var.set_value(fv.clone(), VarOp::Set, Some(i))?);
                    }
                    blocks.add_block(Block::ControlFlow(ControlFlow {
                        op: ControlOp::IfElse,
                        condition: Some(Value::BoolOp(BoolOp::Eq(Box::new(cond), Box::new(Value::Known(KnownVal::Num(1.0)))))),
                        var: None,
                        body: Some(body_blocks),
                        else_body: Some(else_blocks),
                    }));
                }
                _ => {
                    return Err(CompException(
                        "Select operands must both be single or both be indexed".to_string(),
                    ));
                }
            }
        }

        ir::Instr::Freeze(freeze) => {
            let value = trans_value(&freeze.value, ctx, Some(bctx))?;
            let res_var = Variable {
                var_name: localize_var(&freeze.result.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            };
            blocks.add(res_var.set_inferred_value(value)?);
        }

        ir::Instr::Call(call) => {
            // Compute result variable (if any) before handling intrinsics, since some
            // intrinsics such as llvm.fmuladd write to a local result.
            let res_var = call.result.as_ref().map(|r| Variable {
                var_name: localize_var(&r.name, false, Some(&bctx.fn_info.name), false),
                var_type: VarType::Var,
                fn_name: None,
            });

            // Handle intrinsics first; they are not normal procedure calls.
            if let Some(intrinsic) = &call.intrinsic {
                blocks.add(trans_intrinsic(intrinsic, &call.args, res_var, ctx, bctx)?);
                return Ok(blocks);
            }

            let (pre_call, post_call, _) = trans_call_instr(call, ctx, bctx, None)?;
            blocks.add(pre_call);
            blocks.add(post_call);
        }

        _ => {
            return Err(CompException(format!("Unsupported instruction: {:?}", instr)));
        }
    }

    Ok(blocks)
}

fn current_vararg_ptr_value(ctx: &Context, bctx: &BlockInfo) -> Value {
    let name = &ctx.cfg.vararg_ptr_local;
    let is_param = bctx.available_params.iter().any(|p| p.var_name == *name);
    let var = Variable {
        var_name: name.clone(),
        var_type: if is_param { VarType::Param } else { VarType::Var },
        fn_name: Some(bctx.fn_info.name.clone()),
    };
    var.get_value(None)
}

fn trans_intrinsic(
    intrinsic: &ir::instructions::Intrinsic,
    args: &[ir::Value],
    result_var: Option<Variable>,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<BlockList, CompException> {
    use ir::instructions::Intrinsic;

    let mut blocks = BlockList::new();

    // No-op intrinsics
    match intrinsic {
        Intrinsic::VaEnd
        | Intrinsic::LifetimeStart
        | Intrinsic::LifetimeEnd
        | Intrinsic::NoAliasScopeDecl
        | Intrinsic::Expect
        | Intrinsic::ExpectWithProbability
        | Intrinsic::Assume
        | Intrinsic::InlineAsm => return Ok(blocks),
        _ => {}
    }

    let mut values: Vec<Value> = Vec::new();
    for arg in args {
        let iv = trans_value(arg, ctx, Some(bctx))?;
        // Wide integer constants whose magnitude fits in a single Scratch value
        // (e.g. small i64 lengths for memset/memmove) can be used as singles.
        values.push(match iv {
            InferredValue::Single(v) => v,
            InferredValue::Indexed(ref idx) if idx.vals.len() == 2 => {
                if let (Some(Value::Known(KnownVal::Num(low))), Some(Value::Known(KnownVal::Num(high)))) =
                    (idx.vals.get(0), idx.vals.get(1))
                {
                    if *high == 0.0 && low.is_finite() && low.fract() == 0.0 && *low >= 0.0 {
                        Value::Known(KnownVal::Num(*low))
                    } else {
                        iv.into_single()?
                    }
                } else {
                    iv.into_single()?
                }
            }
            InferredValue::Indexed(_) => iv.into_single()?,
        });
    }

    match intrinsic {
        Intrinsic::VaStart => {
            // arglist_ptr points to a va_list; for our target va_list is just a ptr,
            // so store the received vararg pointer at that address.
            let arglist_ptr = values.into_iter().next().ok_or_else(|| {
                CompException("va_start requires one argument".to_string())
            })?;
            let vararg_ptr = current_vararg_ptr_value(ctx, bctx);
            blocks.add(trans_store(
                InferredValue::Single(vararg_ptr),
                arglist_ptr,
                &ir::types::Type::Pointer(ir::types::PointerTy {
                    addrspace: ir::types::AddrSpace::Default,
                }),
                ctx,
            )?);
        }
        Intrinsic::VaCopy => {
            // Copy the vararg pointer stored at src into dest.
            let mut it = values.into_iter();
            let dest = it.next().ok_or_else(|| CompException("va_copy requires dest".to_string()))?;
            let src = it.next().ok_or_else(|| CompException("va_copy requires src".to_string()))?;
            let src_ptr = Value::GetOfList(GetOfList {
                op: ListOp::AtIndex,
                name: ctx.cfg.mem_var.clone(),
                value: Box::new(src),
            });
            blocks.add(trans_store(
                InferredValue::Single(src_ptr),
                dest,
                &ir::types::Type::Pointer(ir::types::PointerTy {
                    addrspace: ir::types::AddrSpace::Default,
                }),
                ctx,
            )?);
        }
        Intrinsic::FMulAdd => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("fmuladd requires 3 args".to_string()))?;
            let b = it.next().ok_or_else(|| CompException("fmuladd requires 3 args".to_string()))?;
            let c = it.next().ok_or_else(|| CompException("fmuladd requires 3 args".to_string()))?;
            let res = Value::Op(Op::Add(
                Box::new(Value::Op(Op::Mul(Box::new(a), Box::new(b)))),
                Box::new(c),
            ));
            if let Some(res_var) = result_var {
                blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
            }
        }
        Intrinsic::UAddSat | Intrinsic::USubSat | Intrinsic::SAddSat | Intrinsic::SSubSat => {
            let mut it = values.into_iter();
            let lft = it.next().ok_or_else(|| CompException("saturating arithmetic requires left operand".to_string()))?;
            let rgt = it.next().ok_or_else(|| CompException("saturating arithmetic requires right operand".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("saturating arithmetic requires integer operands".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("saturating arithmetic requires a result variable".to_string()))?;

            if intrinsic == &Intrinsic::UAddSat || intrinsic == &Intrinsic::USubSat {
                let mod_base = 2f64.powi(width as i32);
                if intrinsic == &Intrinsic::UAddSat {
                    // Unsigned saturating add: min(a + b, 2^width - 1)
                    let unwrapped = Value::Op(Op::Add(Box::new(lft.clone()), Box::new(rgt.clone())));
                    let max_val = Value::Known(KnownVal::Num(mod_base - 1.0));
                    // min(x, y) = (x + y - abs(x - y)) / 2
                    let diff = Value::Op(Op::Sub(Box::new(unwrapped.clone()), Box::new(max_val.clone())));
                    let numerator = Value::Op(Op::Sub(
                        Box::new(Value::Op(Op::Add(Box::new(unwrapped), Box::new(max_val)))),
                        Box::new(Value::Op(Op::Abs(Box::new(diff)))),
                    ));
                    let res = Value::Op(Op::Div(
                        Box::new(numerator),
                        Box::new(Value::Known(KnownVal::Num(2.0))),
                    ));
                    blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
                } else {
                    // Unsigned saturating sub: max(a - b, 0) = (a - b) * (a >= b)
                    let unwrapped = Value::Op(Op::Sub(Box::new(lft.clone()), Box::new(rgt.clone())));
                    let cond = Value::BoolOp(BoolOp::Gt(
                        Box::new(lft),
                        Box::new(rgt),
                    ));
                    let res = Value::Op(Op::Mul(Box::new(unwrapped), Box::new(cond)));
                    blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
                }
            } else {
                // Signed saturating add/sub.  Inputs are in two's-complement bit form
                // in [0, 2^width); convert to signed, clamp, then convert back.
                let mod_base = 2f64.powi(width as i32);
                let half = mod_base / 2.0;
                let max_s = half - 1.0;
                let min_s = -half;

                let to_signed = |v: Value| -> Value {
                    let cond = Value::BoolOp(BoolOp::Lt(
                        Box::new(v.clone()),
                        Box::new(Value::Known(KnownVal::Num(half))),
                    ));
                    Value::Op(Op::Add(
                        Box::new(v),
                        Box::new(Value::Op(Op::Mul(
                            Box::new(Value::Op(Op::Sub(
                                Box::new(Value::Known(KnownVal::Num(1.0))),
                                Box::new(cond),
                            ))),
                            Box::new(Value::Known(KnownVal::Num(-mod_base))),
                        ))),
                    ))
                };
                let from_signed = |v: Value| -> Value {
                    let cond = Value::BoolOp(BoolOp::Lt(
                        Box::new(v.clone()),
                        Box::new(Value::Known(KnownVal::Num(0.0))),
                    ));
                    Value::Op(Op::Add(
                        Box::new(v),
                        Box::new(Value::Op(Op::Mul(
                            Box::new(cond),
                            Box::new(Value::Known(KnownVal::Num(mod_base))),
                        ))),
                    ))
                };

                let a_s = to_signed(lft);
                let b_s = to_signed(rgt);
                let raw = if intrinsic == &Intrinsic::SAddSat {
                    Value::Op(Op::Add(Box::new(a_s), Box::new(b_s)))
                } else {
                    Value::Op(Op::Sub(Box::new(a_s), Box::new(b_s)))
                };

                let cond_gt = Value::BoolOp(BoolOp::Gt(
                    Box::new(raw.clone()),
                    Box::new(Value::Known(KnownVal::Num(max_s))),
                ));
                let cond_lt = Value::BoolOp(BoolOp::Lt(
                    Box::new(raw.clone()),
                    Box::new(Value::Known(KnownVal::Num(min_s))),
                ));

                let mut else_blocks = BlockList::new();
                else_blocks.add(res_var.set_inferred_value(InferredValue::Single(from_signed(Value::Known(KnownVal::Num(min_s)))))?);

                let mut then_blocks = BlockList::new();
                then_blocks.add(res_var.set_inferred_value(InferredValue::Single(from_signed(raw)))?);

                let mut inner_if = BlockList::new();
                inner_if.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(cond_lt),
                    body: Some(else_blocks),
                    else_body: Some(then_blocks),
                    var: None,
                }));

                let mut max_blocks = BlockList::new();
                max_blocks.add(res_var.set_inferred_value(InferredValue::Single(from_signed(Value::Known(KnownVal::Num(max_s)))))?);

                blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(cond_gt),
                    body: Some(max_blocks),
                    else_body: Some(inner_if),
                    var: None,
                }));
            }
        }
        Intrinsic::MemCpy | Intrinsic::MemCpyInline => {
            let mut it = values.into_iter();
            let dest = it.next().ok_or_else(|| CompException("memcpy requires dest".to_string()))?;
            let src = it.next().ok_or_else(|| CompException("memcpy requires src".to_string()))?;
            let length = it.next().ok_or_else(|| CompException("memcpy requires length".to_string()))?;

            let known_length = match &length {
                Value::Known(KnownVal::Num(n)) if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n < 12.0 => Some(*n as usize),
                _ => None,
            };

            if let Some(len) = known_length {
                for offset in 0..len {
                    let offset_val = Value::Known(KnownVal::Num(offset as f64));
                    let get_ptr = Value::GetOfList(GetOfList {
                        op: ListOp::AtIndex,
                        name: ctx.cfg.mem_var.clone(),
                        value: Box::new(Value::Op(Op::Add(
                            Box::new(src.clone()),
                            Box::new(offset_val.clone()),
                        ))),
                    });
                    blocks.add_block(Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(offset_val),
                        ))),
                        value: Some(get_ptr),
                    }));
                }
            } else {
                let ptr_offset = gen_temp_var(ctx);
                blocks.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: ptr_offset.clone(),
                    value: Value::Known(KnownVal::Num(0.0)),
                }));

                let body = BlockList::from_blocks(vec![
                    Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(Value::GetVar { name: ptr_offset.clone() }),
                        ))),
                        value: Some(Value::GetOfList(GetOfList {
                            op: ListOp::AtIndex,
                            name: ctx.cfg.mem_var.clone(),
                            value: Box::new(Value::Op(Op::Add(
                                Box::new(src.clone()),
                                Box::new(Value::GetVar { name: ptr_offset.clone() }),
                            ))),
                        })),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ptr_offset.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }),
                ]);

                blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::RepTimes,
                    condition: Some(length),
                    var: None,
                    body: Some(body),
                    else_body: None,
                }));
            }
        }
        Intrinsic::MemSet => {
            let mut it = values.into_iter();
            let dest = it.next().ok_or_else(|| CompException("memset requires dest".to_string()))?;
            let value = it.next().ok_or_else(|| CompException("memset requires value".to_string()))?;
            let length = it.next().ok_or_else(|| CompException("memset requires length".to_string()))?;

            let known_length = match &length {
                Value::Known(KnownVal::Num(n)) if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n < 12.0 => Some(*n as usize),
                _ => None,
            };

            if let Some(len) = known_length {
                for offset in 0..len {
                    let offset_val = Value::Known(KnownVal::Num(offset as f64));
                    blocks.add_block(Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(offset_val),
                        ))),
                        value: Some(value.clone()),
                    }));
                }
            } else {
                let ptr_offset = gen_temp_var(ctx);
                blocks.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: ptr_offset.clone(),
                    value: Value::Known(KnownVal::Num(0.0)),
                }));

                let body = BlockList::from_blocks(vec![
                    Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(Value::GetVar { name: ptr_offset.clone() }),
                        ))),
                        value: Some(value.clone()),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ptr_offset.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }),
                ]);

                blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::RepTimes,
                    condition: Some(length),
                    var: None,
                    body: Some(body),
                    else_body: None,
                }));
            }
        }
        Intrinsic::MemMove => {
            let mut it = values.into_iter();
            let dest = it.next().ok_or_else(|| CompException("memmove requires dest".to_string()))?;
            let src = it.next().ok_or_else(|| CompException("memmove requires src".to_string()))?;
            let length = it.next().ok_or_else(|| CompException("memmove requires length".to_string()))?;
            // Fourth argument is the volatile flag; it is ignored for Scratch.

            // Memmove must handle overlapping regions.  If dest > src we copy
            // from the end backwards, otherwise we copy forwards.
            let known_length = match &length {
                Value::Known(KnownVal::Num(n)) if n.is_finite() && n.fract() == 0.0 && *n >= 0.0 && *n < 12.0 => Some(*n as usize),
                _ => None,
            };

            if let Some(len) = known_length {
                // For known lengths we cannot decide direction at compile time
                // because dest/src may still be runtime values.  Use a single
                // runtime direction test and unrolled copies in each branch.
                let forward_blocks = {
                    let mut b = BlockList::new();
                    for offset in 0..len {
                        let offset_val = Value::Known(KnownVal::Num(offset as f64));
                        let get_ptr = Value::GetOfList(GetOfList {
                            op: ListOp::AtIndex,
                            name: ctx.cfg.mem_var.clone(),
                            value: Box::new(Value::Op(Op::Add(
                                Box::new(src.clone()),
                                Box::new(offset_val.clone()),
                            ))),
                        });
                        b.add_block(Block::EditList(EditListData {
                            op: ListEditOp::ReplaceAt,
                            name: ctx.cfg.mem_var.clone(),
                            index: Some(Value::Op(Op::Add(
                                Box::new(dest.clone()),
                                Box::new(offset_val),
                            ))),
                            value: Some(get_ptr),
                        }));
                    }
                    b
                };
                let backward_blocks = {
                    let mut b = BlockList::new();
                    for offset in (0..len).rev() {
                        let offset_val = Value::Known(KnownVal::Num(offset as f64));
                        let get_ptr = Value::GetOfList(GetOfList {
                            op: ListOp::AtIndex,
                            name: ctx.cfg.mem_var.clone(),
                            value: Box::new(Value::Op(Op::Add(
                                Box::new(src.clone()),
                                Box::new(offset_val.clone()),
                            ))),
                        });
                        b.add_block(Block::EditList(EditListData {
                            op: ListEditOp::ReplaceAt,
                            name: ctx.cfg.mem_var.clone(),
                            index: Some(Value::Op(Op::Add(
                                Box::new(dest.clone()),
                                Box::new(offset_val),
                            ))),
                            value: Some(get_ptr),
                        }));
                    }
                    b
                };
                blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(Value::BoolOp(BoolOp::Gt(
                        Box::new(dest.clone()),
                        Box::new(src.clone()),
                    ))),
                    var: None,
                    body: Some(backward_blocks),
                    else_body: Some(forward_blocks),
                }));
            } else {
                let ptr_offset = gen_temp_var(ctx);
                let forward_body = BlockList::from_blocks(vec![
                    Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(Value::GetVar { name: ptr_offset.clone() }),
                        ))),
                        value: Some(Value::GetOfList(GetOfList {
                            op: ListOp::AtIndex,
                            name: ctx.cfg.mem_var.clone(),
                            value: Box::new(Value::Op(Op::Add(
                                Box::new(src.clone()),
                                Box::new(Value::GetVar { name: ptr_offset.clone() }),
                            ))),
                        })),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ptr_offset.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }),
                ]);
                let backward_body = BlockList::from_blocks(vec![
                    Block::EditList(EditListData {
                        op: ListEditOp::ReplaceAt,
                        name: ctx.cfg.mem_var.clone(),
                        index: Some(Value::Op(Op::Add(
                            Box::new(dest.clone()),
                            Box::new(Value::GetVar { name: ptr_offset.clone() }),
                        ))),
                        value: Some(Value::GetOfList(GetOfList {
                            op: ListOp::AtIndex,
                            name: ctx.cfg.mem_var.clone(),
                            value: Box::new(Value::Op(Op::Add(
                                Box::new(src.clone()),
                                Box::new(Value::GetVar { name: ptr_offset.clone() }),
                            ))),
                        })),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ptr_offset.clone(),
                        value: Value::Known(KnownVal::Num(-1.0)),
                    }),
                ]);

                let forward_setup = BlockList::from_blocks(vec![
                    Block::EditVar(EditVarData {
                        op: VarOp::Set,
                        name: ptr_offset.clone(),
                        value: Value::Known(KnownVal::Num(0.0)),
                    }),
                    Block::ControlFlow(ControlFlow {
                        op: ControlOp::RepTimes,
                        condition: Some(length.clone()),
                        var: None,
                        body: Some(forward_body),
                        else_body: None,
                    }),
                ]);
                let backward_setup = BlockList::from_blocks(vec![
                    Block::EditVar(EditVarData {
                        op: VarOp::Set,
                        name: ptr_offset.clone(),
                        value: Value::Op(Op::Sub(Box::new(length.clone()), Box::new(Value::Known(KnownVal::Num(1.0))))),
                    }),
                    Block::ControlFlow(ControlFlow {
                        op: ControlOp::RepTimes,
                        condition: Some(length.clone()),
                        var: None,
                        body: Some(backward_body),
                        else_body: None,
                    }),
                ]);

                blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(Value::BoolOp(BoolOp::Gt(
                        Box::new(dest.clone()),
                        Box::new(src.clone()),
                    ))),
                    var: None,
                    body: Some(backward_setup),
                    else_body: Some(forward_setup),
                }));
            }
        }
        Intrinsic::Abs => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("abs requires value".to_string()))?;
            // Second argument is the int-min-poison flag; it is ignored for Scratch.
            let _is_int_min_poison = it.next();

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("abs requires integer operand".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("abs requires a result variable".to_string()))?;

            // Match Python: interpret the unsigned two's-complement bits as signed,
            // negate when negative, then wrap back into unsigned representation.
            let half = 2f64.powi(width as i32 - 1);
            let modulus = 2f64.powi(width as i32);
            let is_pos = Value::BoolOp(BoolOp::Lt(
                Box::new(val.clone()),
                Box::new(Value::Known(KnownVal::Num(half))),
            ));
            let sign = Value::Op(Op::Sub(
                Box::new(Value::Op(Op::Mul(
                    Box::new(is_pos),
                    Box::new(Value::Known(KnownVal::Num(2.0))),
                ))),
                Box::new(Value::Known(KnownVal::Num(1.0))),
            ));
            let res = Value::Op(Op::Mod(
                Box::new(Value::Op(Op::Mul(Box::new(val), Box::new(sign)))),
                Box::new(Value::Known(KnownVal::Num(modulus))),
            ));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::FAbs => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("fabs requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("fabs requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Op(Op::Abs(Box::new(val)))))?);
        }
        Intrinsic::SMax | Intrinsic::SMin | Intrinsic::UMax | Intrinsic::UMin => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("min/max requires two args".to_string()))?;
            let b = it.next().ok_or_else(|| CompException("min/max requires two args".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("min/max requires integer operands".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("min/max requires a result variable".to_string()))?;

            // Unsigned comparisons work directly on the unsigned representation.
            // Signed comparisons must reverse two's complement first, matching Python's intCompare.
            let (a_cmp, b_cmp) = match intrinsic {
                Intrinsic::SMax | Intrinsic::SMin => (
                    twos_complement::reverse_twos_complement(a.clone(), width),
                    twos_complement::reverse_twos_complement(b.clone(), width),
                ),
                Intrinsic::UMax | Intrinsic::UMin => (a.clone(), b.clone()),
                _ => unreachable!(),
            };

            let (cmp, true_is_a) = match intrinsic {
                Intrinsic::SMax | Intrinsic::UMax => (BoolOp::Gt(Box::new(a_cmp), Box::new(b_cmp)), true),
                Intrinsic::SMin | Intrinsic::UMin => (BoolOp::Lt(Box::new(a_cmp), Box::new(b_cmp)), true),
                _ => unreachable!(),
            };
            let (true_val, false_val) = if true_is_a { (a, b) } else { (b, a) };
            blocks.add_block(Block::ControlFlow(ControlFlow {
                op: ControlOp::IfElse,
                condition: Some(Value::BoolOp(cmp)),
                var: None,
                body: Some(BlockList::from_block(res_var.set_value(true_val, VarOp::Set, None)?)),
                else_body: Some(BlockList::from_block(res_var.set_value(false_val, VarOp::Set, None)?)),
            }));
        }
        Intrinsic::PtrMask => {
            let mut it = values.into_iter();
            let ptr = it.next().ok_or_else(|| CompException("ptrmask requires pointer".to_string()))?;
            let mask = it.next().ok_or_else(|| CompException("ptrmask requires mask".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("ptrmask requires a result variable".to_string()))?;
            let res = binop::binop(
                binop::BinopKind::And, ptr, mask, PTR_WIDTH_BITS, &ctx.cfg,
                &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
            )?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::FShl | Intrinsic::FShr => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("fshl/fshr requires three arguments".to_string()))?;
            let b_val = it.next().ok_or_else(|| CompException("fshl/fshr requires three arguments".to_string()))?;
            let c = it.next().ok_or_else(|| CompException("fshl/fshr requires three arguments".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("fshl/fshr requires a result variable".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("fshl/fshr requires integer operands".to_string())),
            };

            // fshr(a, b, c) = fshl(b, a, c): swap a and b for fshr
            let (a, b) = if matches!(intrinsic, Intrinsic::FShr) {
                (b_val, a)
            } else {
                (a, b_val)
            };

            let modulus = Value::Known(KnownVal::Num(2f64.powi(width as i32)));

            // Compute fshl(a, b, shift) = (a*2^shift + b/2^(width-shift)) mod 2^width
            let compute_fshl = |shift_val: f64| -> Value {
                let a_shift = 2f64.powi(shift_val as i32);
                let b_shift = 2f64.powi((width as i32) - shift_val as i32);
                let shifted_a = Value::Op(Op::Mod(
                    Box::new(Value::Op(Op::Mul(Box::new(a.clone()), Box::new(Value::Known(KnownVal::Num(a_shift)))))),
                    Box::new(modulus.clone()),
                ));
                let shifted_b = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                    Box::new(b.clone()),
                    Box::new(Value::Known(KnownVal::Num(b_shift))),
                )))));
                let raw = Value::Op(Op::Add(Box::new(shifted_a), Box::new(shifted_b)));
                Value::Op(Op::Mod(Box::new(raw), Box::new(modulus.clone())))
            };

            match c {
                Value::Known(KnownVal::Num(n)) if n.is_finite() && n.fract() == 0.0 && n >= 0.0 => {
                    let c_num = n as i32;
                    if c_num as usize >= width {
                        return Err(CompException(format!("fshl/fshr shift amount {} out of range for width {}", c_num, width)));
                    }
                    let res = compute_fshl(n);
                    blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
                }
                c => {
                    // Runtime shift amount: generate if/else chain for all possible
                    // shift values 0..width. The last matching condition wins.
                    let res = compute_fshl(0.0);
                    blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
                    for shift_val in 1..width {
                        let shift_f = shift_val as f64;
                        let cond = Value::BoolOp(BoolOp::Eq(
                            Box::new(c.clone()),
                            Box::new(Value::Known(KnownVal::Num(shift_f))),
                        ));
                        let then_res = compute_fshl(shift_f);
                        let then_blocks = BlockList::from_blocks(vec![
                            Block::EditVar(EditVarData {
                                op: VarOp::Set,
                                name: res_var.var_name.clone(),
                                value: then_res,
                            }),
                        ]);
                        blocks.add_block(Block::ControlFlow(ControlFlow {
                            op: ControlOp::If,
                            condition: Some(cond),
                            body: Some(then_blocks),
                            else_body: None,
                            var: None,
                        }));
                    }
                }
            }
        }
        Intrinsic::UMulWithOverflow => {
            let mut it = values.into_iter();
            let lft = it.next().ok_or_else(|| CompException("umul.with.overflow requires left operand".to_string()))?;
            let rgt = it.next().ok_or_else(|| CompException("umul.with.overflow requires right operand".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("umul.with.overflow requires integer operands".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("umul.with.overflow requires a result variable".to_string()))?;

            let modulus = 2f64.powi(width as i32);
            let mul_val = twos_complement::multiply_wrap(lft.clone(), rgt.clone(), width)?;
            let raw_mul = Value::Op(Op::Mul(Box::new(lft), Box::new(rgt)));
            let did_overflow = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Gt(
                Box::new(raw_mul),
                Box::new(Value::Known(KnownVal::Num(modulus - 1.0))),
            )))));

            blocks.add(res_var.set_inferred_value(InferredValue::Indexed(IdxbleValue {
                vals: vec![mul_val, did_overflow],
            }))?);
        }
        Intrinsic::CopySign => {
            let mut it = values.into_iter();
            let mag = it.next().ok_or_else(|| CompException("copysign requires magnitude".to_string()))?;
            let sgn = it.next().ok_or_else(|| CompException("copysign requires sign".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("copysign requires a result variable".to_string()))?;

            let is_neg = Value::BoolOp(BoolOp::Lt(
                Box::new(sgn.clone()),
                Box::new(Value::Known(KnownVal::Num(0.0))),
            ));
            let sign = Value::Op(Op::Sub(
                Box::new(Value::Known(KnownVal::Num(1.0))),
                Box::new(Value::Op(Op::Mul(Box::new(is_neg), Box::new(Value::Known(KnownVal::Num(2.0)))))),
            ));
            let res = Value::Op(Op::Mul(
                Box::new(Value::Op(Op::Abs(Box::new(mag)))),
                Box::new(sign),
            ));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::Ctlz => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("ctlz requires value".to_string()))?;
            let _is_zero_poison = it.next(); // ignored; caller guarantees input is non-zero
            let res_var = result_var.ok_or_else(|| CompException("ctlz requires a result variable".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("ctlz requires integer operand".to_string())),
            };

            if width == 0 || width > 48 {
                return Err(CompException(format!("ctlz width {} not supported (1..48 supported)", width)));
            }

            // Standard Hacker's Delight binary search for leading zeros.
            // Pad to next power of two, run the search on the padded domain,
            // then subtract the excess padding bits from the result.
            let domain = width.next_power_of_two();
            let excess = domain - width;

            let mut ctlz_blocks = BlockList::new();
            let n_var = gen_temp_var(ctx);
            let x_var = gen_temp_var(ctx);

            ctlz_blocks.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set,
                name: n_var.clone(),
                value: Value::Known(KnownVal::Num(domain as f64)),
            }));
            ctlz_blocks.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set,
                name: x_var.clone(),
                value: val,
            }));

            let mut shift = domain / 2;
            while shift > 0 {
                let divisor = Value::Known(KnownVal::Num(2f64.powi(shift as i32)));
                let shifted = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                    Box::new(Value::GetVar { name: x_var.clone() }),
                    Box::new(divisor),
                )))));
                // If the upper `shift` bits are non-zero, discard the lower bits
                let is_zero = Value::BoolOp(BoolOp::Eq(
                    Box::new(shifted.clone()),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                ));
                let cond = Value::BoolOp(BoolOp::Not(Box::new(is_zero)));
                let then_blocks = BlockList::from_blocks(vec![
                    Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: n_var.clone(),
                        value: Value::Known(KnownVal::Num(-(shift as f64))),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Set,
                        name: x_var.clone(),
                        value: shifted,
                    }),
                ]);
                ctlz_blocks.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(cond),
                    body: Some(then_blocks),
                    else_body: None,
                    var: None,
                }));
                shift /= 2;
            }

            // Final correction: n - x (x is 0 or 1 after the search)
            let x_val = Value::GetVar { name: x_var };
            let result = Value::Op(Op::Sub(
                Box::new(Value::GetVar { name: n_var }),
                Box::new(x_val),
            ));
            let result = Value::Op(Op::Sub(
                Box::new(result),
                Box::new(Value::Known(KnownVal::Num(excess as f64))),
            ));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(result))?);
        }
        Intrinsic::GetRounding => {
            let res_var = result_var.ok_or_else(|| CompException("get.rounding requires a result variable".to_string()))?;
            // Scratch uses IEEE-754 round-to-nearest semantics, which corresponds
            // to rounding mode 0.  There is no dynamic rounding-mode support.
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Known(KnownVal::Num(0.0))))?);
        }
        Intrinsic::VaEnd
        | Intrinsic::LifetimeStart
        | Intrinsic::LifetimeEnd
        | Intrinsic::NoAliasScopeDecl
        | Intrinsic::Expect
        | Intrinsic::ExpectWithProbability
        | Intrinsic::Assume
        | Intrinsic::InlineAsm
        | Intrinsic::NoOp
        | Intrinsic::SideEffect => {}
        Intrinsic::Trap | Intrinsic::DebugTrap => {
            blocks.add_block(Block::StopScript(StopOption::All));
        }
        Intrinsic::Bswap => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("bswap requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("bswap requires a result variable".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("bswap requires integer operand".to_string())),
            };

            let total_bytes = width / 8;
            let mut parts: Vec<Value> = Vec::new();
            for byte in 0..total_bytes {
                let src_byte = total_bytes - 1 - byte;
                let right_shift = src_byte * 8;
                let left_shift = byte * 8;
                let shifted = if right_shift == 0 {
                    val.clone()
                } else {
                    Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                        Box::new(val.clone()),
                        Box::new(Value::Known(KnownVal::Num(2f64.powi(right_shift as i32)))),
                    )))))
                };
                let masked = binop::binop(
                    binop::BinopKind::And, shifted,
                    Value::Known(KnownVal::Num(255.0)),
                    width, &ctx.cfg,
                    &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
                )?;
                let part = if left_shift == 0 {
                    masked
                } else {
                    Value::Op(Op::Mul(
                        Box::new(masked),
                        Box::new(Value::Known(KnownVal::Num(2f64.powi(left_shift as i32)))),
                    ))
                };
                parts.push(part);
            }
            let res = parts.into_iter().fold(
                Value::Known(KnownVal::Num(0.0)),
                |acc, v| Value::Op(Op::Add(Box::new(acc), Box::new(v))),
            );
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::BitReverse => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("bitreverse requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("bitreverse requires a result variable".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("bitreverse requires integer operand".to_string())),
            };

            let modulus = Value::Known(KnownVal::Num(2f64.powi(width as i32)));

            let mut cur = val;
            let mut step = 1;
            while step < width {
                let mut mask_val: i128 = 0;
                for i in 0..(width as i32) {
                    if (i / step as i32) % 2 == 0 {
                        mask_val |= 1i128 << i;
                    }
                }
                let mask = Value::Known(KnownVal::Num(mask_val as f64));
                let left_masked = binop::binop(
                    binop::BinopKind::And, cur.clone(), mask.clone(),
                    width, &ctx.cfg,
                    &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
                )?;
                let shift_up = Value::Known(KnownVal::Num(2f64.powi(step as i32)));
                let shifted_left = Value::Op(Op::Mod(
                    Box::new(Value::Op(Op::Mul(Box::new(left_masked), Box::new(shift_up)))),
                    Box::new(modulus.clone()),
                ));
                let right_masked = binop::binop(
                    binop::BinopKind::And, cur, mask,
                    width, &ctx.cfg,
                    &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
                )?;
                let shift_down = Value::Known(KnownVal::Num(2f64.powi(step as i32)));
                let shifted_right = Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                    Box::new(right_masked), Box::new(shift_down),
                )))));
                cur = Value::Op(Op::Add(Box::new(shifted_left), Box::new(shifted_right)));
                step *= 2;
            }
            blocks.add(res_var.set_inferred_value(InferredValue::Single(cur))?);
        }
        Intrinsic::Cttz => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("cttz requires value".to_string()))?;
            let _is_zero_poison = it.next();
            let res_var = result_var.ok_or_else(|| CompException("cttz requires a result variable".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("cttz requires integer operand".to_string())),
            };

            if width != 32 {
                return Err(CompException(format!("cttz width {} not supported", width)));
            }

            let mut blocks_impl = BlockList::new();
            let count_var = gen_temp_var(ctx);
            let val_var = gen_temp_var(ctx);

            blocks_impl.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set, name: count_var.clone(), value: Value::Known(KnownVal::Num(0.0)),
            }));
            blocks_impl.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set, name: val_var.clone(), value: val,
            }));

            for (mask, shift, add) in &[(0x0000FFFFu32, 16, 16), (0x000000FF, 8, 8), (0x0000000F, 4, 4), (0x00000003, 2, 2), (0x00000001, 1, 1)] {
                let val_ref = Value::GetVar { name: val_var.clone() };
                let and_result = binop::binop(
                    binop::BinopKind::And, val_ref,
                    Value::Known(KnownVal::Num(*mask as f64)),
                    32, &ctx.cfg,
                    &mut ctx.needs_and_lut, &mut ctx.needs_or_lut, &mut ctx.needs_xor_lut,
                )?;
                let cond = Value::BoolOp(BoolOp::Eq(
                    Box::new(and_result),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                ));
                let then_blocks = BlockList::from_blocks(vec![
                    Block::EditVar(EditVarData {
                        op: VarOp::Change, name: count_var.clone(), value: Value::Known(KnownVal::Num(*add as f64)),
                    }),
                    Block::EditVar(EditVarData {
                        op: VarOp::Set, name: val_var.clone(),
                        value: Value::Op(Op::Floor(Box::new(Value::Op(Op::Div(
                            Box::new(Value::GetVar { name: val_var.clone() }),
                            Box::new(Value::Known(KnownVal::Num(2f64.powi(*shift)))),
        ))))),
                    }),
                ]);
                blocks_impl.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::If, condition: Some(cond),
                    body: Some(then_blocks), else_body: None, var: None,
                }));
            }

            blocks.add(blocks_impl);
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::GetVar { name: count_var }))?);
        }
        Intrinsic::Sqrt => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("sqrt requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("sqrt requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Op(Op::Sqrt(Box::new(val)))))?);
        }
        Intrinsic::Ceil => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("ceil requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("ceil requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Op(Op::Ceiling(Box::new(val)))))?);
        }
        Intrinsic::Floor => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("floor requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("floor requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Op(Op::Floor(Box::new(val)))))?);
        }
        Intrinsic::Trunc => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("trunc requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("trunc requires a result variable".to_string()))?;
            let cond = Value::BoolOp(BoolOp::Gt(
                Box::new(val.clone()),
                Box::new(Value::Known(KnownVal::Num(0.0))),
            ));
            let floor_val = Value::Op(Op::Floor(Box::new(val.clone())));
            let ceil_val = Value::Op(Op::Ceiling(Box::new(val)));
            blocks.add_block(Block::ControlFlow(ControlFlow {
                op: ControlOp::IfElse,
                condition: Some(cond),
                var: None,
                body: Some(BlockList::from_block(res_var.set_value(floor_val, VarOp::Set, None)?)),
                else_body: Some(BlockList::from_block(res_var.set_value(ceil_val, VarOp::Set, None)?)),
            }));
        }
        Intrinsic::Rint | Intrinsic::NearbyInt | Intrinsic::Round => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("rint/round requires value".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("rint/round requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(Value::Op(Op::Round(Box::new(val)))))?);
        }
        Intrinsic::Fma => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("fma requires 3 args".to_string()))?;
            let b = it.next().ok_or_else(|| CompException("fma requires 3 args".to_string()))?;
            let c = it.next().ok_or_else(|| CompException("fma requires 3 args".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("fma requires a result variable".to_string()))?;
            let res = Value::Op(Op::Add(
                Box::new(Value::Op(Op::Mul(Box::new(a), Box::new(b)))),
                Box::new(c),
            ));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::Pow => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("pow requires 2 args".to_string()))?;
            let b = it.next().ok_or_else(|| CompException("pow requires 2 args".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("pow requires a result variable".to_string()))?;
            let res = Value::Op(Op::Exp(Box::new(Value::Op(Op::Mul(
                Box::new(b),
                Box::new(Value::Op(Op::Ln(Box::new(a)))),
            )))));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::Powi => {
            let mut it = values.into_iter();
            let a = it.next().ok_or_else(|| CompException("powi requires 2 args".to_string()))?;
            let b = it.next().ok_or_else(|| CompException("powi requires 2 args".to_string()))?;
            let res_var = result_var.ok_or_else(|| CompException("powi requires a result variable".to_string()))?;
            // powi takes integer exponent; convert to float via StrToFloat trick
            let b_float = if matches!(&b, Value::Known(KnownVal::Num(_))) {
                b
            } else {
                Value::Op(Op::StrToFloat(Box::new(b)))
            };
            let res = Value::Op(Op::Exp(Box::new(Value::Op(Op::Mul(
                Box::new(b_float),
                Box::new(Value::Op(Op::Ln(Box::new(a)))),
            )))));
            blocks.add(res_var.set_inferred_value(InferredValue::Single(res))?);
        }
        Intrinsic::StackSave => {
            let res_var = result_var.ok_or_else(|| CompException("stacksave requires a result variable".to_string()))?;
            blocks.add(res_var.set_inferred_value(InferredValue::Single(
                Value::GetVar { name: ctx.cfg.stack_pointer_var.clone() }
            ))?);
        }
        Intrinsic::StackRestore => {
            let mut it = values.into_iter();
            let val = it.next().ok_or_else(|| CompException("stackrestore requires value".to_string()))?;
            blocks.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set,
                name: ctx.cfg.stack_pointer_var.clone(),
                value: val,
            }));
        }
        Intrinsic::UAddWithOverflow | Intrinsic::USubWithOverflow => {
            let mut it = values.into_iter();
            let lft = it.next().ok_or_else(|| CompException("uadd/usub.with.overflow requires left operand".to_string()))?;
            let rgt = it.next().ok_or_else(|| CompException("uadd/usub.with.overflow requires right operand".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("uadd/usub.with.overflow requires integer operands".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("uadd/usub.with.overflow requires a result variable".to_string()))?;

            let modulus = 2f64.powi(width as i32);

            let raw = if *intrinsic == Intrinsic::UAddWithOverflow {
                Value::Op(Op::Add(Box::new(lft.clone()), Box::new(rgt.clone())))
            } else {
                Value::Op(Op::Sub(Box::new(lft.clone()), Box::new(rgt.clone())))
            };

            let wrapped = Value::Op(Op::Mod(
                Box::new(Value::Op(Op::Add(
                    Box::new(raw.clone()),
                    Box::new(Value::Known(KnownVal::Num(modulus))),
                ))),
                Box::new(Value::Known(KnownVal::Num(modulus))),
            ));

            let sum = Value::Op(Op::Add(Box::new(lft.clone()), Box::new(rgt.clone())));
            let bool_op = if *intrinsic == Intrinsic::UAddWithOverflow {
                BoolOp::Gt(
                    Box::new(sum),
                    Box::new(Value::Known(KnownVal::Num(modulus - 1.0))),
                )
            } else {
                BoolOp::Lt(
                    Box::new(lft),
                    Box::new(rgt),
                )
            };
            let did_overflow = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(bool_op))));

            blocks.add(res_var.set_inferred_value(InferredValue::Indexed(IdxbleValue {
                vals: vec![wrapped, did_overflow],
            }))?);
        }
        Intrinsic::SAddWithOverflow | Intrinsic::SSubWithOverflow => {
            let mut it = values.into_iter();
            let lft = it.next().ok_or_else(|| CompException("sadd/ssub.with.overflow requires left/right operand".to_string()))?;
            let rgt = it.next().ok_or_else(|| CompException("sadd/ssub.with.overflow requires left/right operand".to_string()))?;

            let width = match args.get(0).map(|a| a.type_()) {
                Some(Type::Integer(IntegerTy { width })) => *width as usize,
                _ => return Err(CompException("sadd/ssub.with.overflow requires integer operands".to_string())),
            };
            let res_var = result_var.ok_or_else(|| CompException("sadd/ssub.with.overflow requires a result variable".to_string()))?;

            let half = 2f64.powi(width as i32 - 1);
            let modulus = 2f64.powi(width as i32);

            let to_signed = |v: Value| -> Value {
                let cond = Value::BoolOp(BoolOp::Lt(
                    Box::new(v.clone()),
                    Box::new(Value::Known(KnownVal::Num(half))),
                ));
                Value::Op(Op::Add(
                    Box::new(v),
                    Box::new(Value::Op(Op::Mul(
                        Box::new(Value::Op(Op::Sub(
                            Box::new(Value::Known(KnownVal::Num(1.0))),
                            Box::new(cond),
                        ))),
                        Box::new(Value::Known(KnownVal::Num(-modulus))),
                    ))),
                ))
            };

            let lft_s = to_signed(lft);
            let rgt_s = to_signed(rgt);
            let raw = if matches!(intrinsic, Intrinsic::SAddWithOverflow) {
                Value::Op(Op::Add(Box::new(lft_s.clone()), Box::new(rgt_s.clone())))
            } else {
                Value::Op(Op::Sub(Box::new(lft_s), Box::new(rgt_s)))
            };

            let max_s = half - 1.0;
            let min_s = -half;

            let from_signed = |v: Value| -> Value {
                let cond = Value::BoolOp(BoolOp::Lt(
                    Box::new(v.clone()),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                ));
                Value::Op(Op::Add(
                    Box::new(v),
                    Box::new(Value::Op(Op::Mul(
                        Box::new(cond),
                        Box::new(Value::Known(KnownVal::Num(modulus))),
                    ))),
                ))
            };

            let wrapped = from_signed(raw.clone());
            let did_overflow = Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(BoolOp::Or(
                Box::new(Value::BoolOp(BoolOp::Gt(Box::new(raw.clone()), Box::new(Value::Known(KnownVal::Num(max_s)))))),
                Box::new(Value::BoolOp(BoolOp::Lt(Box::new(raw), Box::new(Value::Known(KnownVal::Num(min_s)))))),
            )))));

            blocks.add(res_var.set_inferred_value(InferredValue::Indexed(IdxbleValue {
                vals: vec![wrapped, did_overflow],
            }))?);
        }
        _ => {
            return Err(CompException(format!(
                "Unsupported intrinsic: {:?}",
                intrinsic
            )));
        }
    }

    Ok(blocks)
}

fn assign_phi_nodes(
    assignments: &[(Variable, ir::Value)],
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();

    // Track dependency cycles for local-var phi assignments, matching Python's
    // assignPhiNodes behavior.
    let mut end_assignments: Vec<(Variable, InferredValue)> = Vec::new();
    let mut to_resolve: IndexMap<String, String> = IndexMap::new();
    let mut set_by: IndexMap<String, String> = IndexMap::new();
    let mut resolved: Vec<(String, String)> = Vec::new();
    let mut var_lookup: HashMap<String, Variable> = HashMap::new();
    let mut val_lookup: HashMap<String, InferredValue> = HashMap::new();

    for (res_var, ir_val) in assignments {
        if matches!(ir_val, ir::Value::Undef(_)) {
            continue;
        }
        let val = trans_value(ir_val, ctx, Some(bctx))?;

        if let ir::Value::LocalVar(lv) = ir_val {
            // Use the plain local variable name (matching Python's assignPhiNodes),
            // so dependency keys and values are in the same namespace.
            to_resolve.insert(res_var.var_name.clone(), lv.name.clone());
            var_lookup.insert(res_var.var_name.clone(), res_var.clone());
            val_lookup.insert(lv.name.clone(), val);
        } else {
            end_assignments.push((res_var.clone(), val));
        }
    }

    while !to_resolve.is_empty() {
        let cant_set: HashSet<String> = to_resolve.values().cloned().collect();
        let to_set: Vec<String> = to_resolve
            .keys()
            .cloned()
            .filter(|k| !cant_set.contains(k))
            .collect();
        let to_set_empty = to_set.is_empty();

        for var_name in to_set {
            let deps = to_resolve.get(&var_name).cloned().unwrap();
            set_by.insert(deps.clone(), var_name.clone());
            resolved.push((var_name.clone(), deps));
            to_resolve.shift_remove(&var_name);
        }

        if to_set_empty {
            // Dependency cycle: create a temporary or reuse an already-set variable.
            let already_set: Vec<String> = to_resolve
                .keys()
                .cloned()
                .filter(|k| set_by.contains_key(k))
                .collect();

            let (to_make_temp, temp_name) = if let Some(existing) = already_set.first() {
                let temp_name = set_by.get(existing).cloned().unwrap();
                (existing.clone(), temp_name)
            } else {
                let to_make_temp = to_resolve.keys().next().cloned().unwrap();
                let to_make_temp_val = val_lookup.get(&to_make_temp).cloned().unwrap();

                let temp_name = gen_temp_var(ctx);
                let temp_var = Variable {
                    var_name: temp_name.clone(),
                    var_type: VarType::SpecialVar,
                    fn_name: None,
                };
                var_lookup.insert(temp_name.clone(), temp_var.clone());
                let temp_val = match &to_make_temp_val {
                    InferredValue::Single(_) => InferredValue::Single(temp_var.get_value(None)),
                    InferredValue::Indexed(iv) => InferredValue::Indexed(temp_var.get_all_values(iv.vals.len())),
                };
                val_lookup.insert(temp_name.clone(), temp_val);

                resolved.push((temp_name.clone(), to_make_temp.clone()));
                (to_make_temp, temp_name)
            };

            let deps = to_resolve.get(&to_make_temp).cloned().unwrap();
            resolved.push((to_make_temp.clone(), deps.clone()));
            to_resolve.shift_remove(&to_make_temp);

            for deps_val in to_resolve.values_mut() {
                if *deps_val == to_make_temp {
                    *deps_val = temp_name.clone();
                }
            }
        }
    }

    for (var_name, val_name) in resolved {
        let res_var = var_lookup.get(&var_name).cloned().ok_or_else(|| {
            CompException(format!("Missing phi resolution variable: {}", var_name))
        })?;
        let val = val_lookup.get(&val_name).cloned().ok_or_else(|| {
            CompException(format!("Missing phi resolution value: {}", val_name))
        })?;
        blocks.add(res_var.set_inferred_value(val)?);
    }
    for (res_var, val) in end_assignments {
        blocks.add(res_var.set_inferred_value(val)?);
    }

    Ok(blocks)
}

fn trans_return_addr(
    return_address: &Value,
    info: &(dyn ReturnAddrInfo + '_),
    _ctx: &Context,
) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();
    if info.takes_return_address() {
        let mut branches: BTreeMap<usize, BlockList> = BTreeMap::new();
        for (i, addr) in info.return_addresses().iter().enumerate() {
            branches.insert(i, BlockList::from_blocks(vec![Block::ProcedureCall(
                scratch::ast::ProcedureCallData {
                    name: addr.clone(),
                    args: Vec::new(),
                    run_without_refresh: false,
                },
            )]));
        }
        blocks.add(binary_search_jump_table(
            return_address.clone(),
            branches,
            None,
            None,
            None,
            true,
            0,
            info.return_addresses().len().saturating_sub(1).max(1),
        ));
    } else if !info.return_addresses().is_empty() {
        if info.return_addresses().len() != 1 {
            return Err(CompException(format!(
                "trans_return_addr: expected exactly one return address, got {}",
                info.return_addresses().len()
            )));
        }
        blocks.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
            name: info.return_addresses()[0].clone(),
            args: Vec::new(),
            run_without_refresh: false,
        }));
    }
    blocks.end = true;
    Ok(blocks)
}

/// Compute variable use for the instructions following a call, matching Python's
/// `getBlockVarUse`. Used to decide which live variables must be saved around a
/// recursive call.
fn compute_next_var_use(
    following_instrs: &[ir::Instr],
    source_label: &str,
    phi_info: &HashMap<String, HashMap<String, Vec<(Variable, ir::Value)>>>,
    block_var_use: &HashMap<String, BlockVarUse>,
    result_name: Option<&str>,
) -> Result<BlockVarUse, CompException> {
    let mut res = BlockVarUse::default();
    if let Some(name) = result_name {
        res.modifies.insert(name.to_string());
    }

    for instr in following_instrs {
        let mut vals = Vec::new();
        // Collect values used by the instruction.
        match instr {
            ir::Instr::Ret(r) => {
                if let Some(v) = &r.value {
                    vals.push(v.clone());
                }
            }
            ir::Instr::Conversion(c) => vals.push(c.value.clone()),
            ir::Instr::Freeze(f) => vals.push(f.value.clone()),
            ir::Instr::Load(l) => vals.push(l.address.clone()),
            ir::Instr::Store(s) => {
                vals.push(s.address.clone());
                vals.push(s.value.clone());
            }
            ir::Instr::Call(c) => vals.extend(c.args.clone()),
            ir::Instr::UnaryOp(u) => vals.push(u.operand.clone()),
            ir::Instr::BinaryOp(b) => {
                vals.push(b.left.clone());
                vals.push(b.right.clone());
            }
            ir::Instr::ICmp(c) => {
                vals.push(c.left.clone());
                vals.push(c.right.clone());
            }
            ir::Instr::FCmp(c) => {
                vals.push(c.left.clone());
                vals.push(c.right.clone());
            }
            ir::Instr::CondBr(c) => vals.push(c.cond.clone()),
            ir::Instr::Switch(s) => vals.push(s.cond.clone()),
            ir::Instr::Select(s) => {
                vals.push(s.cond.clone());
                vals.push(s.true_value.clone());
                vals.push(s.false_value.clone());
            }
            ir::Instr::GetElementPtr(g) => {
                vals.push(g.base_ptr.clone());
                vals.extend(g.indices.clone());
            }
            ir::Instr::ExtractValue(e) => vals.push(e.agg.clone()),
            ir::Instr::InsertValue(i) => {
                vals.push(i.agg.clone());
                vals.push(i.element.clone());
            }
            ir::Instr::VaArg(v) => vals.push(v.arglist.clone()),
            _ => {}
        }

        // For terminators, also include phi values from outgoing edges.
        if matches!(
            instr,
            ir::Instr::UncondBr(_) | ir::Instr::CondBr(_) | ir::Instr::Switch(_)
        ) {
            let targets = super::graph_util::terminator_branch_labels(instr);
            for (target, phi_vals) in phi_info.get(source_label).unwrap_or(&HashMap::new()) {
                if targets.contains(target) {
                    vals.extend(phi_vals.iter().map(|(_, v)| v.clone()));
                }
            }
        }

        let mut instr_depends = HashSet::new();
        let mut instr_depends_var_sizes = HashMap::new();
        for val in vals {
            match val {
                ir::Value::Argument(arg) => {
                    instr_depends.insert(arg.name.clone());
                    instr_depends_var_sizes.insert(arg.name.clone(), memory::get_size_of(&arg.type_, false)?);
                }
                ir::Value::LocalVar(lv) => {
                    instr_depends.insert(lv.name.clone());
                    instr_depends_var_sizes.insert(lv.name.clone(), memory::get_size_of(&lv.type_, false)?);
                }
                _ => {}
            }
        }
        res.depends.extend(&instr_depends - &res.modifies);
        res.depends_var_sizes.extend(instr_depends_var_sizes);

        if let Some(result) = instr.result() {
            res.modifies.insert(result.name.clone());
        }
    }

    // Merge dependencies from branch targets.
    if let Some(last) = following_instrs.last() {
        for label in super::graph_util::terminator_branch_labels(last) {
            if label == "ret" {
                continue;
            }
            if let Some(bvu) = block_var_use.get(&label) {
                res.depends.extend((&bvu.depends - &res.modifies).iter().cloned());
                res.modifies.extend(bvu.modifies.iter().cloned());
                res.depends_var_sizes.extend(bvu.depends_var_sizes.clone());
            }
        }
    }

    Ok(res)
}

/// Collect variable names that future blocks may depend on, so parameters can
/// be stored in local variables before branching. Matches Python's behavior in
/// `transTerminatorInstr`.
fn terminator_poss_depends(
    instr: &ir::Instr,
    fn_info: &FuncInfo,
    special_locals: &HashSet<String>,
) -> HashSet<String> {
    let mut labels = Vec::new();
    match instr {
        ir::Instr::UncondBr(br) => labels.push(br.branch.label.clone()),
        ir::Instr::CondBr(cbr) => {
            labels.push(cbr.branch_true.label.clone());
            labels.push(cbr.branch_false.label.clone());
        }
        ir::Instr::Switch(switch) => {
            labels.push(switch.branch_default.label.clone());
            for (_, label) in &switch.branch_table {
                labels.push(label.label.clone());
            }
        }
        _ => {}
    }

    let mut poss_depends = HashSet::new();
    for label in labels {
        if let Some(bvu) = fn_info.block_var_use.get(&label) {
            poss_depends.extend(bvu.depends.iter().cloned());
        }
    }
    poss_depends.extend(special_locals.iter().cloned());
    poss_depends
}

fn trans_terminator_instr(
    instr: &ir::Instr,
    ctx: &mut Context,
    bctx: &mut BlockInfo,
) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();
    let poss_depends = terminator_poss_depends(instr, &bctx.fn_info, &ctx.cfg.special_locals);

    match instr {
        ir::Instr::Ret(ret) => {
            if let Some(val) = &ret.value {
                let value = trans_value(val, ctx, Some(bctx))?;
                let return_var = Variable {
                    var_name: ctx.cfg.return_var.clone(),
                    var_type: VarType::Var,
                    fn_name: None,
                };
                blocks.add(return_var.set_inferred_value(value)?);
            }

            if !bctx.fn_info.skip_stack_size_change {
                if let Some(total_alloca) = bctx.fn_info.total_alloca_size {
                    if total_alloca != 0 {
                        blocks.add_block(Block::EditVar(EditVarData {
                            op: VarOp::Change,
                            name: ctx.cfg.stack_pointer_var.clone(),
                            value: Value::Known(KnownVal::Num(total_alloca as f64)),
                        }));
                    }
                }
            }

            if bctx.fn_info.name == ctx.cfg.entrypoint && !ctx.cfg.use_branch_jump_table {
                blocks.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: ctx.cfg.jump_table_id_var.clone(),
                    value: Value::Known(KnownVal::Num(super::config::EXIT_CALL_ID as f64)),
                }));
            }

            if bctx.fn_info.returns_to_address {
                let return_addr = localize_var(&ctx.cfg.return_address_local, false, Some(&bctx.fn_info.name), false);
                let return_addr_var = Variable {
                    var_name: return_addr,
                    var_type: VarType::Var,
                    fn_name: None,
                };
                blocks.add(trans_return_addr(&return_addr_var.get_value(None), &bctx.fn_info, ctx)?);
            }

            if ctx.cfg.use_branch_jump_table {
                blocks.add_block(Block::StopScript(scratch::ast::StopOption::This));
            }

            blocks.end = true;
        }

        ir::Instr::UncondBr(br) => {
            let label = &br.branch.label;
            blocks.add(assign_parameters(
                &bctx.available_params,
                &bctx.available_param_sizes,
                &poss_depends,
                ctx.cfg.use_branch_jump_table,
            )?);

            let source_label = bctx.label.clone().unwrap_or_default();
            let phi_data = bctx.fn_info.phi_info
                .get(&source_label)
                .and_then(|m| m.get(label))
                .cloned()
                .unwrap_or_default();
            blocks.add(assign_phi_nodes(&phi_data, ctx, bctx)?);

            let proc_name = localize_label(label, &bctx.fn_info.name);
            blocks.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: proc_name,
                args: Vec::new(),
                run_without_refresh: false,
            }));
        }

        ir::Instr::CondBr(cbr) => {
            blocks.add(assign_parameters(
                &bctx.available_params,
                &bctx.available_param_sizes,
                &poss_depends,
                ctx.cfg.use_branch_jump_table,
            )?);

            let cond = trans_value(&cbr.cond, ctx, Some(bctx))?.into_single()?;
            let label_true = &cbr.branch_true.label;
            let label_false = &cbr.branch_false.label;

            let source_label = bctx.label.clone().unwrap_or_default();
            let phi_data_true = bctx.fn_info.phi_info
                .get(&source_label)
                .and_then(|m| m.get(label_true))
                .cloned()
                .unwrap_or_default();
            let phi_data_false = bctx.fn_info.phi_info
                .get(&source_label)
                .and_then(|m| m.get(label_false))
                .cloned()
                .unwrap_or_default();
            let fn_name = bctx.fn_info.name.clone();

            let mut true_body = BlockList::new();
            true_body.add(assign_phi_nodes(&phi_data_true, ctx, bctx)?);
            true_body.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: localize_label(label_true, &fn_name),
                args: Vec::new(),
                run_without_refresh: false,
            }));

            let mut false_body = BlockList::new();
            false_body.add(assign_phi_nodes(&phi_data_false, ctx, bctx)?);
            false_body.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: localize_label(label_false, &fn_name),
                args: Vec::new(),
                run_without_refresh: false,
            }));

            blocks.add_block(Block::ControlFlow(ControlFlow {
                op: ControlOp::IfElse,
                condition: Some(Value::BoolOp(BoolOp::Eq(
                    Box::new(cond),
                    Box::new(Value::Known(KnownVal::Num(1.0))),
                ))),
                var: None,
                body: Some(true_body),
                else_body: Some(false_body),
            }));
        }

        ir::Instr::Switch(switch) => {
            blocks.add(assign_parameters(
                &bctx.available_params,
                &bctx.available_param_sizes,
                &poss_depends,
                ctx.cfg.use_branch_jump_table,
            )?);

            let cond = trans_value(&switch.cond, ctx, Some(bctx))?.into_single()?;
            let width = match switch.cond.type_() {
                ir::Type::Integer(it) => it.width,
                _ => return Err(CompException("Switch condition must be an integer".to_string())),
            };
            if memory::get_size_of(switch.cond.type_(), false)? > 1 {
                return Err(CompException(format!(
                    "Cannot currently switch with an integer more than {} bits",
                    super::config::VARIABLE_MAX_BITS
                )));
            }

            let source_label = bctx.label.clone().unwrap_or_default();
            let fn_name = bctx.fn_info.name.clone();

            let mut branches: BTreeMap<usize, BlockList> = BTreeMap::new();
            for (case_val_ir, label) in &switch.branch_table {
                let case_val = trans_value(case_val_ir, ctx, Some(bctx))?.into_single()?;
                let case_num = match case_val {
                    Value::Known(KnownVal::Num(n)) if n.is_finite() && n.fract() == 0.0 && n >= 0.0 => n as usize,
                    _ => return Err(CompException("Switch case values must be constant non-negative integers".to_string())),
                };

                let phi_data = bctx.fn_info.phi_info
                    .get(&source_label)
                    .and_then(|m| m.get(&label.label))
                    .cloned()
                    .unwrap_or_default();

                let mut branch_body = BlockList::new();
                branch_body.add(assign_phi_nodes(&phi_data, ctx, bctx)?);
                branch_body.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                    name: localize_label(&label.label, &fn_name),
                    args: Vec::new(),
                    run_without_refresh: false,
                }));

                branches.insert(case_num, branch_body);
            }

            let default_phi_data = bctx.fn_info.phi_info
                .get(&source_label)
                .and_then(|m| m.get(&switch.branch_default.label))
                .cloned()
                .unwrap_or_default();
            let mut default_branch = BlockList::new();
            default_branch.add(assign_phi_nodes(&default_phi_data, ctx, bctx)?);
            default_branch.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: localize_label(&switch.branch_default.label, &fn_name),
                args: Vec::new(),
                run_without_refresh: false,
            }));

            let min_poss = 0usize;
            let max_poss = ((1u128 << width) - 1) as usize;
            let lo = 0usize;
            let hi = branches.len().saturating_sub(1).max(1);

            blocks.add(binary_search_jump_table(
                cond,
                branches,
                Some(default_branch),
                Some(min_poss),
                Some(max_poss),
                false,
                lo,
                hi,
            ));
        }

        ir::Instr::Unreachable => {}

        _ => {
            return Err(CompException(format!("Unsupported terminator: {:?}", instr)));
        }
    }

    Ok(blocks)
}

fn trans_funcs(mod_: &DecodedModule, mut ctx: Context) -> Result<Context, CompException> {
    ctx = get_fn_info(mod_, ctx)?;
    collect_setjmp_sites(mod_, &mut ctx)?;

    for (_, func) in &mod_.functions {
        if func.blocks.is_empty() {
            continue;
        }

        let fn_name = &func.name;
        let info = ctx.fn_info.get(fn_name).cloned()
            .ok_or_else(|| CompException(format!("Function info not found for {}", fn_name)))?;

        let first_label = func.blocks.values().next().map(|b| b.label.clone()).unwrap_or_default();
        let mut is_first_block = true;
        let mut total_fn_allocated: usize = 0;

        // If the function can branch back to its first block, emit an entry wrapper
        // that assigns parameters and calls the first-block procedure. This matches
        // Python's transFuncs branches_to_first handling.
        if info.branches_to_first {
            let mut wrapper = BlockList::new();
            let wrapper_param_names: Vec<String> = info.params
                .iter()
                .zip(info.param_sizes.iter())
                .flat_map(|(p, size)| {
                    let n = (*size).max(1);
                    (0..n).map(move |i| {
                        p.get_raw_var_name(if *size == 1 { None } else { Some(i) })
                    })
                })
                .collect();
            wrapper.add_block(Block::ProcedureDef(scratch::ast::ProcedureDefData {
                name: fn_name.clone(),
                params: wrapper_param_names,
                warp: true,
            }));

            if info.returns_to_address {
                wrapper.add_block(Block::EditCounter(CounterOp::Increment));
            }

            if !info.skip_stack_size_change {
                let prev_stack_var = Variable {
                    var_name: localize_var(&ctx.cfg.previous_stack_size_local, false, Some(fn_name), false),
                    var_type: VarType::Var,
                    fn_name: None,
                };
                wrapper.add_block(prev_stack_var.set_value(
                    Value::GetVar { name: ctx.cfg.stack_pointer_var.clone() },
                    VarOp::Set,
                    None,
                )?);
            }

            let mut poss_depends = info.block_var_use
                .get(&first_label)
                .map(|bvu| bvu.depends.clone())
                .unwrap_or_default();
            poss_depends.extend(ctx.cfg.special_locals.iter().cloned());
            wrapper.add(assign_parameters(
                &info.params,
                &info.param_sizes,
                &poss_depends,
                ctx.cfg.use_branch_jump_table,
            )?);

            let first_block_proc_name = localize_label(&first_label, fn_name);
            wrapper.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: first_block_proc_name,
                args: Vec::new(),
                run_without_refresh: false,
            }));

            ctx.proj.code.push(wrapper);
            is_first_block = false;
        }

        for (_, block) in &func.blocks {
            let proc_name = if is_first_block {
                fn_name.clone()
            } else {
                localize_label(&block.label, fn_name)
            };

            let (localized_params, localized_param_sizes) = if is_first_block {
                (info.params.clone(), info.param_sizes.clone())
            } else {
                (Vec::new(), Vec::new())
            };

            let mut starting_code = BlockList::new();
            let proc_param_names: Vec<String> = localized_params
                .iter()
                .zip(localized_param_sizes.iter())
                .flat_map(|(p, size)| {
                    let n = (*size).max(1);
                    (0..n).map(move |i| {
                        p.get_raw_var_name(if *size == 1 { None } else { Some(i) })
                    })
                })
                .collect();
            starting_code.add_block(Block::ProcedureDef(scratch::ast::ProcedureDefData {
                name: proc_name.clone(),
                params: proc_param_names,
                warp: true,
            }));

            // Functions that may recurse need a counter increment on every branch entry
            if info.returns_to_address {
                starting_code.add_block(Block::EditCounter(CounterOp::Increment));
            }

            // Add recursion stack reset check for blocks that are part of a cycle
            if info.checked_blocks.contains(&block.label) {
                let reset_id = super::config::START_STACK_RESET_ID
                    + ctx.all_check_locations
                        .iter()
                        .position(|(fname, lbl)| fname == fn_name && lbl == &block.label)
                        .ok_or_else(|| CompException(format!(
                            "Could not find check location for {}:{}", fn_name, block.label
                        )))?;

                let mut check_body = BlockList::new();
                check_body.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: ctx.cfg.jump_table_id_var.clone(),
                    value: Value::Known(KnownVal::Num(reset_id as f64)),
                }));
                if !ctx.cfg.progress_var.is_empty() {
                    check_body.add_block(Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ctx.cfg.progress_var.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }));
                    if ctx.cfg.progress_say {
                        check_body.add_block(Block::Say {
                            value: Value::GetVar {
                                name: ctx.cfg.progress_var.clone(),
                            },
                        });
                    }
                }
                check_body.add_block(Block::StopScript(StopOption::This));

                starting_code.add_block(Block::ControlFlow(ControlFlow {
                    op: ControlOp::If,
                    condition: Some(Value::BoolOp(BoolOp::Gt(
                        Box::new(Value::GetCounter),
                        Box::new(Value::Known(KnownVal::Num(ctx.cfg.max_branch_recursion as f64))),
                    ))),
                    var: None,
                    body: Some(check_body),
                    else_body: None,
                }));
            }

            if is_first_block && info.total_alloca_size.is_none() && !info.skip_stack_size_change {
                let prev_stack_var = Variable {
                    var_name: localize_var(&ctx.cfg.previous_stack_size_local, false, Some(fn_name), false),
                    var_type: VarType::Var,
                    fn_name: None,
                };
                starting_code.add_block(prev_stack_var.set_value(
                    Value::GetVar { name: ctx.cfg.stack_pointer_var.clone() },
                    VarOp::Set,
                    None,
                )?);
            }

            let to_allocate = if is_first_block {
                info.total_alloca_size.unwrap_or(0)
            } else {
                *info.block_alloca_size.get(&block.label).unwrap_or(&0)
            };

            if to_allocate != 0 && !info.skip_stack_size_change {
                starting_code.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Change,
                    name: ctx.cfg.stack_pointer_var.clone(),
                    value: Value::Known(KnownVal::Num(-(to_allocate as f64))),
                }));
            }

            let (available_params, available_param_sizes) = if is_first_block || ctx.cfg.use_branch_jump_table {
                (info.params.clone(), info.param_sizes.clone())
            } else {
                (Vec::new(), Vec::new())
            };

            let mut bctx = BlockInfo {
                fn_info: info.clone(),
                available_params,
                available_param_sizes,
                code: starting_code,
                label: Some(block.label.clone()),
                allocated: total_fn_allocated,
                next_call_id: 0,
            };

            let mut instr_idx = 0;
            while instr_idx < block.instrs.len() {
                let instr = &block.instrs[instr_idx];
                if instr.is_terminator() {
                    break;
                }

                if let ir::Instr::Call(call) = instr {
                    let label = bctx.label.clone().unwrap_or_default();

                    // Handle setjmp/longjmp before ordinary call analysis. These
                    // intrinsics do not go through the regular procedure-call
                    // path and may split the current block into a continuation.
                    if let ir::Value::Function(fv) = &call.func {
                        if is_setjmp_name(&fv.name) || is_longjmp_name(&fv.name) {
                            if ctx.cfg.use_branch_jump_table {
                                return Err(CompException(format!(
                                    "{} is not supported with branch jump tables in Phase 1; \
                                     use --opt-target scratch3 or -T scratch3",
                                    fv.name
                                )));
                            }
                        }
                        if is_setjmp_name(&fv.name) {
                            let site = ctx.setjmp_sites
                                .iter()
                                .find(|s| {
                                    s.fn_name == *fn_name
                                        && s.block_label == block.label
                                        && s.call_idx == bctx.next_call_id
                                })
                                .cloned()
                                .ok_or_else(|| CompException(format!(
                                    "setjmp site not found for {} in {}:{} call_idx {}",
                                    fv.name, fn_name, block.label, bctx.next_call_id
                                )))?;
                            trans_setjmp_call(call, &site, &mut ctx, &mut bctx)?;
                            bctx.next_call_id += 1;
                            instr_idx += 1;
                            continue;
                        }
                        if is_longjmp_name(&fv.name) {
                            let blocks = trans_longjmp_call(call, &mut ctx, &mut bctx)?;
                            bctx.code.add(blocks);
                            bctx.next_call_id += 1;
                            instr_idx += 1;
                            continue;
                        }
                    }

                    if let Some((returns_to_address, takes_return_address, return_addresses)) =
                        get_call_return_addr_info(call, &ctx)
                    {
                        let callee_name = get_call_callee_name(call, &ctx);
                        let poss_recursive = callee_name.as_ref().map_or(false, |name| {
                            name == fn_name && bctx.fn_info.can_call.contains(name)
                        });

                        let following_instrs = &block.instrs[instr_idx + 1..];
                        let (must_store, must_store_sizes) = if poss_recursive {
                            compute_must_store(call, &bctx, following_instrs, &ctx)?
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        let do_recursive = poss_recursive && !must_store.is_empty();

                        if do_recursive && !returns_to_address && !ctx.cfg.use_branch_jump_table {
                            // Use Scratch's procedure parameters as a stack to save live variables.
                            let recurse_proc_name = localize_call_id(bctx.next_call_id, &label, fn_name, true);

                            let must_store_values: Vec<Value> = must_store
                                .iter()
                                .zip(&must_store_sizes)
                                .map(|(var, size)| {
                                    if *size == 1 {
                                        var.get_value(None)
                                    } else {
                                        // Flatten indexed values into individual reporter arguments.
                                        let indexed = var.get_all_values(*size);
                                        indexed.vals.into_iter().next().unwrap_or(var.get_value(None))
                                    }
                                })
                                .collect();
                            bctx.code.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                                name: recurse_proc_name.clone(),
                                args: must_store_values,
                                run_without_refresh: false,
                            }));

                            ctx.proj.code.push(bctx.code);

                            let new_code = BlockList::from_blocks(vec![Block::ProcedureDef(
                                scratch::ast::ProcedureDefData {
                                    name: recurse_proc_name,
                                    params: must_store
                                        .iter()
                                        .map(|var| var.get_raw_var_name(None))
                                        .collect(),
                                    warp: true,
                                },
                            )]);

                            let saved_available_params = std::mem::replace(&mut bctx.available_params, must_store);
                            let saved_available_param_sizes = std::mem::replace(&mut bctx.available_param_sizes, must_store_sizes);
                            bctx.code = new_code;

                            let (pre_call, post_call, _) = trans_call_instr(call, &mut ctx, &mut bctx, None)?;
                            bctx.code.add(pre_call);
                            bctx.code.add(post_call);
                            if let Some(check) = longjmp_propagation_check(&ctx) {
                                bctx.code.add(check);
                            }

                            bctx.available_params = saved_available_params;
                            bctx.available_param_sizes = saved_available_param_sizes;
                        } else {
                            if do_recursive {
                                // Save live variables on the local stack before the call.
                                let total_size: usize = must_store_sizes.iter().sum();
                                if total_size != 0 {
                                    bctx.code.add_block(Block::EditVar(EditVarData {
                                        op: VarOp::Change,
                                        name: ctx.cfg.local_stack_size_var.clone(),
                                        value: Value::Known(KnownVal::Num(total_size as f64)),
                                    }));

                                    let mut offset = 0usize;
                                    for (var, size) in must_store.iter().zip(&must_store_sizes) {
                                        let val = if *size == 1 {
                                            var.get_value(None)
                                        } else {
                                            var.get_all_values(*size).vals.into_iter().next().unwrap_or(var.get_value(None))
                                        };
                                        bctx.code.add(store_on_stack(
                                            &ctx.cfg.local_stack_var,
                                            &ctx.cfg.local_stack_size_var,
                                            -(offset as i64 + *size as i64 - 1),
                                            *size,
                                            &val,
                                        ));
                                        offset += *size;
                                    }
                                }
                            }

                            if returns_to_address {
                                let return_proc_name = localize_call_id(bctx.next_call_id, &label, fn_name, false);
                                let return_addr_id = if takes_return_address {
                                    Some(return_addresses.iter().position(|r| r == &return_proc_name).ok_or_else(|| {
                                        CompException(format!(
                                            "Could not find return address {} in callee return addresses",
                                            return_proc_name
                                        ))
                                    })?)
                                } else {
                                    None
                                };

                                let (pre_call, post_call, _) = trans_call_instr(call, &mut ctx, &mut bctx, return_addr_id)?;
                                bctx.code.add(pre_call);

                                // Finish the current procedure; the callee will return to the new target.
                                ctx.proj.code.push(bctx.code);

                                // Start the return-address target procedure.
                                let mut new_code = BlockList::from_blocks(vec![Block::ProcedureDef(
                                    scratch::ast::ProcedureDefData {
                                        name: return_proc_name,
                                        params: Vec::new(),
                                        warp: true,
                                    },
                                )]);
                                new_code.add_block(Block::EditCounter(CounterOp::Increment));
                                new_code.add(post_call);
                                if let Some(check) = longjmp_propagation_check(&ctx) {
                                    new_code.add(check);
                                }

                                if do_recursive {
                                    // Restore live variables after the call returns.
                                    let total_size: usize = must_store_sizes.iter().sum();
                                    let mut offset = 0usize;
                                    for (var, size) in must_store.iter().zip(&must_store_sizes) {
                                        let loaded = load_from_stack(
                                            &ctx.cfg.local_stack_var,
                                            &ctx.cfg.local_stack_size_var,
                                            -(offset as i64 + *size as i64 - 1),
                                        );
                                        if *size == 1 {
                                            new_code.add_block(var.set_value(loaded, VarOp::Set, None)?);
                                        } else {
                                            new_code.add_block(var.set_value(loaded, VarOp::Set, None)?);
                                        }
                                        offset += *size;
                                    }
                                    if total_size != 0 {
                                        new_code.add_block(Block::EditVar(EditVarData {
                                            op: VarOp::Change,
                                            name: ctx.cfg.local_stack_size_var.clone(),
                                            value: Value::Known(KnownVal::Num(-(total_size as f64))),
                                        }));
                                    }
                                }

                                bctx.code = new_code;
                                bctx.available_params = Vec::new();
                                bctx.available_param_sizes = Vec::new();
                            } else {
                                let (pre_call, post_call, _) = trans_call_instr(call, &mut ctx, &mut bctx, None)?;
                                bctx.code.add(pre_call);
                                bctx.code.add(post_call);
                                if let Some(check) = longjmp_propagation_check(&ctx) {
                                    bctx.code.add(check);
                                }

                                if do_recursive {
                                    // Restore live variables after the call returns.
                                    let total_size: usize = must_store_sizes.iter().sum();
                                    let mut offset = 0usize;
                                    for (var, size) in must_store.iter().zip(&must_store_sizes) {
                                        let loaded = load_from_stack(
                                            &ctx.cfg.local_stack_var,
                                            &ctx.cfg.local_stack_size_var,
                                            -(offset as i64 + *size as i64 - 1),
                                        );
                                        if *size == 1 {
                                            bctx.code.add_block(var.set_value(loaded, VarOp::Set, None)?);
                                        } else {
                                            bctx.code.add_block(var.set_value(loaded, VarOp::Set, None)?);
                                        }
                                        offset += *size;
                                    }
                                    if total_size != 0 {
                                        bctx.code.add_block(Block::EditVar(EditVarData {
                                            op: VarOp::Change,
                                            name: ctx.cfg.local_stack_size_var.clone(),
                                            value: Value::Known(KnownVal::Num(-(total_size as f64))),
                                        }));
                                    }
                                }
                            }
                        }
                        bctx.next_call_id += 1;
                    } else {
                        // Intrinsic call: handle normally.
                        let instr_code = trans_instr(instr, &mut ctx, &mut bctx)?;
                        bctx.code.add(instr_code);
                        bctx.next_call_id += 1;
                    }
                } else {
                    let instr_code = trans_instr(instr, &mut ctx, &mut bctx)?;
                    bctx.code.add(instr_code);
                }

                instr_idx += 1;
            }

            if let Some(terminator) = block.instrs.last() {
                if terminator.is_terminator() {
                    let terminator_code = trans_terminator_instr(terminator, &mut ctx, &mut bctx)?;
                    bctx.code.add(terminator_code);
                }
            }

            ctx.proj.code.push(bctx.code);

            is_first_block = false;
            if info.total_alloca_size.is_none() {
                total_fn_allocated = bctx.allocated;
            }
        }
    }

    Ok(ctx)
}

fn trans_func_ptr_sigs(ctx: &mut Context) -> Result<(), CompException> {
    let mut addr = ctx.min_func_ptr_addr;
    for (signature_id, (signature, could_call)) in ctx.fn_ptr_sigs.iter().enumerate() {
        // If only one possible target, the call is handled as a direct call.
        if could_call.len() == 1 {
            addr += could_call.len();
            continue;
        }

        let info = ctx.fn_ptr_sig_info.get(signature_id).cloned()
            .ok_or_else(|| CompException(format!("Missing fn_ptr_sig_info for {}", signature_id)))?;

        let sig_name = localize_func_ptr_sig(signature_id);

        let mut arg_count: usize = 0;
        for arg_ty in &signature.params {
            arg_count += memory::get_size_of(arg_ty, false)?;
        }
        let arguments: Vec<String> = (0..arg_count).map(|n| format!("%{}", n)).collect();

        let return_address_name = localize_param(&ctx.cfg.return_address_local);
        let vararg_ptr_name = localize_param(&ctx.cfg.vararg_ptr_local);
        let func_ptr_addr_name = localize_param(&ctx.cfg.func_ptr_parameter);

        let mut params = vec![ctx.cfg.func_ptr_parameter.clone()];
        params.extend(arguments.clone());
        if info.is_variadic {
            params.push(ctx.cfg.vararg_ptr_local.clone());
        }
        if info.takes_return_address {
            params.push(ctx.cfg.return_address_local.clone());
        }

        let mut blocks = BlockList::from_blocks(vec![
            Block::ProcedureDef(scratch::ast::ProcedureDefData {
                name: sig_name.clone(),
                params,
                warp: true,
            }),
        ]);

        let mut return_addr_val: Option<Value> = None;
        if info.returns_to_address {
            blocks.add_block(Block::EditCounter(CounterOp::Increment));
            let return_addr_var = Variable {
                var_name: ctx.cfg.return_address_local.clone(),
                var_type: VarType::Var,
                fn_name: Some(sig_name.clone()),
            };
            if info.takes_return_address {
                if !info.could_recurse {
                    blocks.add_block(return_addr_var.set_value(
                        Value::GetParam { name: return_address_name.clone() },
                        VarOp::Set,
                        None,
                    )?);
                } else {
                    blocks.add_block(Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: ctx.cfg.local_stack_size_var.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }));
                    blocks.add(store_on_stack(
                        &ctx.cfg.local_stack_var,
                        &ctx.cfg.local_stack_size_var,
                        0,
                        1,
                        &Value::GetParam { name: return_address_name.clone() },
                    ));
                }
            }
            return_addr_val = Some(return_addr_var.get_value(None));
        }

        let callback = localize_func_ptr_sig_callback(signature_id);

        let mut branches: BTreeMap<usize, BlockList> = BTreeMap::new();
        for name in could_call {
            let callee_info = ctx.fn_info.get(name).cloned()
                .ok_or_else(|| CompException(format!("Could not find function {}", name)))?;
            if info.is_variadic != callee_info.is_variadic {
                return Err(CompException(format!(
                    "Function pointer signature variadic mismatch for {}", name
                )));
            }

            let mut args: Vec<Value> = arguments.iter()
                .map(|arg| Value::GetParam { name: arg.clone() })
                .collect();
            if info.is_variadic {
                args.push(Value::GetParam { name: vararg_ptr_name.clone() });
            }
            if callee_info.takes_return_address {
                let callee_return_addr = callee_info.return_addresses.iter()
                    .position(|r| r == &callback)
                    .ok_or_else(|| CompException(format!(
                        "Could not find callback {} in callee {} return addresses",
                        callback, name
                    )))?;
                args.push(Value::Known(KnownVal::Num(callee_return_addr as f64)));
            }

            let mut branch = BlockList::from_blocks(vec![
                Block::ProcedureCall(scratch::ast::ProcedureCallData {
                    name: name.clone(),
                    args,
                    run_without_refresh: false,
                }),
            ]);

            if info.returns_to_address && !callee_info.returns_to_address {
                branch.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                    name: callback.clone(),
                    args: Vec::new(),
                    run_without_refresh: false,
                }));
            }

            branches.insert(addr, branch);
            addr += 1;
        }

        let func_ptr_addr_val = Value::GetParam {
            name: func_ptr_addr_name.clone(),
        };
        let jump_table_blocks = binary_search_jump_table(
            func_ptr_addr_val,
            branches,
            None,
            None,
            None,
            false,
            0,
            could_call.len().saturating_sub(1).max(1),
        );
        blocks.add(jump_table_blocks);

        ctx.proj.code.push(blocks);

        if info.returns_to_address {
            let mut callback_blocks = BlockList::from_blocks(vec![
                Block::ProcedureDef(scratch::ast::ProcedureDefData {
                    name: callback.clone(),
                    params: Vec::new(),
                    warp: true,
                }),
                Block::EditCounter(CounterOp::Increment),
            ]);

            let return_addr = if info.could_recurse && info.takes_return_address {
                callback_blocks.add_block(Block::EditVar(EditVarData {
                    op: VarOp::Change,
                    name: ctx.cfg.local_stack_size_var.clone(),
                    value: Value::Known(KnownVal::Num(-1.0)),
                }));
                load_from_stack(
                    &ctx.cfg.local_stack_var,
                    &ctx.cfg.local_stack_size_var,
                    1,
                )
            } else {
                return_addr_val.ok_or_else(|| CompException(
                    "Missing return address value for function pointer callback".to_string()
                ))?
            };

            callback_blocks.add(trans_return_addr(&return_addr, &info, ctx)?);
            ctx.proj.code.push(callback_blocks);
        }
    }
    Ok(())
}

fn get_value_width(val: &ir::Value) -> usize {
    get_type_width(val.type_())
}

fn get_type_width(ty: &ir::Type) -> usize {
    match ty {
        ir::Type::Integer(int_ty) => int_ty.width as usize,
        ir::Type::Half => 16,
        ir::Type::Float => 32,
        ir::Type::Double => 64,
        ir::Type::Fp128 => 128,
        ir::Type::Pointer(_) => super::config::PTR_WIDTH_BITS,
        _ => 32,
    }
}

fn build_lookup_table_comptime(kind: BinopKind, ctx: &mut Context) {
    let name = binop::lookup_table_name(&kind, &ctx.cfg);
    let lookup_size = 1usize << super::config::BINOP_LOOKUP_BITS;
    let mut values = Vec::with_capacity(lookup_size * lookup_size - 1);
    for l in 0..lookup_size {
        for r in 0..lookup_size {
            let v = match kind {
                BinopKind::And => (l & r) as f64,
                BinopKind::Or => (l | r) as f64,
                BinopKind::Xor => (l ^ r) as f64,
            };
            values.push(KnownVal::Num(v));
        }
    }
    // Scratch lists are 1-indexed; drop the first element so lookup index 0 maps to list position 1.
    values.remove(0);
    ctx.proj.lists.insert(name, values);
}

fn init_lookup_tables(ctx: &mut Context) -> Result<BlockList, CompException> {
    if ctx.cfg.gen_lut_runtime {
        // TODO: implement runtime lookup table generation to match Python.
        return Ok(BlockList::new());
    }
    if ctx.needs_and_lut {
        build_lookup_table_comptime(BinopKind::And, ctx);
    }
    if ctx.needs_or_lut {
        build_lookup_table_comptime(BinopKind::Or, ctx);
    }
    if ctx.needs_xor_lut {
        build_lookup_table_comptime(BinopKind::Xor, ctx);
    }
    Ok(BlockList::new())
}

/// Replace calls to branch procedures with assignments to the branch jump table
/// variable, recursing into control flow bodies.
fn replace_branch_calls(bl: &mut BlockList, replacements: &HashMap<String, Block>) -> Result<(), CompException> {
    for block in &mut bl.blocks {
        match block {
            Block::ProcedureCall(pc) if replacements.contains_key(&pc.name) => {
                if !pc.args.is_empty() {
                    return Err(CompException(format!(
                        "branch jump table replacement expects no args, but {} has {} args",
                        pc.name, pc.args.len()
                    )));
                }
                *block = replacements[&pc.name].clone();
            }
            Block::ControlFlow(cf) => {
                if let Some(body) = &mut cf.body {
                    replace_branch_calls(body, replacements)?;
                }
                if let Some(else_body) = &mut cf.else_body {
                    replace_branch_calls(else_body, replacements)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Post-optimization transformation that converts inter-block procedure calls
/// into a per-function branch jump table. Matches Python's `postOptTransform`.
///
/// Returns `true` if any transformation was applied.
pub fn post_opt_transform(
    proj: &mut Project,
    functions: &HashMap<String, crate::ir::instructions::Function>,
    cfg: &CompilerConfig,
) -> Result<bool, CompException> {
    if !cfg.use_branch_jump_table {
        return Ok(false);
    }

    // Map procedure names to their index in the project code.
    let mut procs: HashMap<String, usize> = HashMap::new();
    for (i, bl) in proj.code.iter().enumerate() {
        if let Some(Block::ProcedureDef(pd)) = bl.blocks.first() {
            procs.insert(pd.name.clone(), i);
        }
    }

    let mut to_remove: Vec<usize> = Vec::new();
    let mut did_transform = false;

    for func in functions.values() {
        if func.blocks.len() <= 1 {
            continue;
        }

        let fn_name = &func.name;
        let needs_call_replacement: Vec<String> = func
            .blocks
            .values()
            .skip(1)
            .map(|b| localize_label(&b.label, fn_name))
            .collect();
        if needs_call_replacement.is_empty() {
            continue;
        }

        let branch_id_var_name =
            localize_var(&cfg.branch_jump_table_addr_local, false, Some(fn_name), false);

        let mut replacements: HashMap<String, Block> = HashMap::new();
        for (i, name) in needs_call_replacement.iter().enumerate() {
            replacements.insert(
                name.clone(),
                Block::EditVar(EditVarData {
                    op: VarOp::Set,
                    name: branch_id_var_name.clone(),
                    value: Value::Known(KnownVal::Num(i as f64)),
                }),
            );
        }

        for name in std::iter::once(fn_name).chain(needs_call_replacement.iter()) {
            if let Some(&idx) = procs.get(name) {
                replace_branch_calls(&mut proj.code[idx], &replacements)?;
            }
        }

        let fn_proc_idx = *procs
            .get(fn_name)
            .ok_or_else(|| CompException(format!("Could not find procedure for function {}", fn_name)))?;

        {
            let first_proc = &mut proj.code[fn_proc_idx];
            if let Some(last) = first_proc.blocks.last() {
                if last.is_end() {
                    return Err(CompException(format!(
                        "Cannot append branch jump table forever loop after ending block in {}",
                        fn_name
                    )));
                }
            }
            first_proc.end = false;
        }

        let mut jump_table: BTreeMap<usize, BlockList> = BTreeMap::new();
        for (i, name) in needs_call_replacement.iter().enumerate() {
            let idx = procs[name];
            let body_blocks: Vec<Block> = proj.code[idx].blocks.iter().skip(1).cloned().collect();
            jump_table.insert(i, BlockList::from_blocks(body_blocks));
        }

        let mut get_branch_id = Value::GetVar {
            name: branch_id_var_name.clone(),
        };
        if cfg.opt_target.exec.compiler_type_hints {
            get_branch_id = Value::Op(Op::StrToFloat(Box::new(get_branch_id)));
        }

        let hi = needs_call_replacement.len().saturating_sub(1).max(0);
        let dispatch = binary_search_jump_table(
            get_branch_id,
            jump_table,
            None,
            None,
            None,
            false,
            0,
            hi,
        );

        let mut forever_body = BlockList::new();
        forever_body.add(dispatch);
        let forever = Block::ControlFlow(ControlFlow {
            op: ControlOp::Forever,
            condition: None,
            var: None,
            body: Some(forever_body),
            else_body: None,
        });
        proj.code[fn_proc_idx].add_block(forever);

        to_remove.extend(
            needs_call_replacement
                .iter()
                .filter_map(|n| procs.get(n).copied()),
        );
        did_transform = true;
    }

    to_remove.sort_unstable_by(|a, b| b.cmp(a));
    to_remove.dedup();
    for idx in to_remove {
        proj.code.remove(idx);
    }

    Ok(did_transform)
}

fn binary_search_jump_table(
    value: Value,
    branches: BTreeMap<usize, BlockList>,
    default_branch: Option<BlockList>,
    min_poss_value: Option<usize>,
    max_poss_value: Option<usize>,
    _are_branches_sorted: bool,
    lo: usize,
    hi: usize,
) -> BlockList {
    if branches.is_empty() {
        return default_branch.unwrap_or_else(BlockList::new);
    }

    let keys: Vec<usize> = branches.keys().copied().collect();
    let mid = (lo + hi) / 2;
    if mid >= keys.len() {
        return default_branch.unwrap_or_else(BlockList::new);
    }
    let mid_val = keys[mid];

    if lo == hi {
        let body = branches.values().nth(mid).cloned().unwrap_or_else(BlockList::new);
        if let Some(default) = default_branch {
            let skip_check = min_poss_value == Some(mid_val) && max_poss_value == Some(mid_val);
            if !skip_check {
                return BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
                    op: ControlOp::IfElse,
                    condition: Some(Value::BoolOp(BoolOp::Eq(
                        Box::new(value),
                        Box::new(Value::Known(KnownVal::Num(mid_val as f64))),
                    ))),
                    var: None,
                    body: Some(body),
                    else_body: Some(default),
                })]);
            }
        }
        return body;
    }

    let true_branch = binary_search_jump_table(
        value.clone(),
        branches.clone(),
        default_branch.clone(),
        Some(mid_val + 1),
        max_poss_value,
        true,
        mid + 1,
        hi,
    );
    let false_branch = binary_search_jump_table(
        value.clone(),
        branches.clone(),
        default_branch,
        min_poss_value,
        Some(mid_val),
        true,
        lo,
        mid,
    );

    BlockList::from_blocks(vec![Block::ControlFlow(ControlFlow {
        op: ControlOp::IfElse,
        condition: Some(Value::BoolOp(BoolOp::Gt(
            Box::new(value),
            Box::new(Value::Known(KnownVal::Num(mid_val as f64))),
        ))),
        var: None,
        body: Some(true_branch),
        else_body: Some(false_branch),
    })])
}

fn trans_entrypoint_call(ctx: &mut Context) -> Result<BlockList, CompException> {
    let mut blocks = BlockList::new();
    if let Some(info) = ctx.fn_info.get(&ctx.cfg.entrypoint).cloned() {
        let main_params_len: usize = info.param_sizes.iter().sum();
        let args: Vec<Value> = (0..main_params_len)
            .map(|_| Value::Known(KnownVal::Num(0.0)))
            .collect();

        if !info.returns_to_address {
            blocks.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: info.name.clone(),
                args,
                run_without_refresh: false,
            }));
        } else {
            let mut jump_table: BTreeMap<usize, BlockList> = BTreeMap::new();
            jump_table.insert(
                super::config::EXIT_CALL_ID,
                BlockList::from_blocks(vec![Block::StopScript(StopOption::This)]),
            );
            jump_table.insert(
                super::config::ENTRY_CALL_ID,
                BlockList::from_blocks(vec![Block::ProcedureCall(scratch::ast::ProcedureCallData {
                    name: info.name.clone(),
                    args,
                    run_without_refresh: false,
                })]),
            );

            for (id_offset, (fn_name, block_label)) in ctx.all_check_locations.iter().enumerate() {
                let branch_proc_name = localize_label(block_label, fn_name);
                jump_table.insert(
                    super::config::START_STACK_RESET_ID + id_offset,
                    BlockList::from_blocks(vec![Block::ProcedureCall(scratch::ast::ProcedureCallData {
                        name: branch_proc_name,
                        args: Vec::new(),
                        run_without_refresh: false,
                    })]),
                );
            }

            // Longjmp targets: resume at the continuation procedure for each
            // setjmp site. The dispatch id is stored in jmp_buf by setjmp.
            for site in &ctx.setjmp_sites {
                jump_table.insert(
                    site.dispatch_id,
                    BlockList::from_blocks(vec![Block::ProcedureCall(scratch::ast::ProcedureCallData {
                        name: site.continuation_name.clone(),
                        args: Vec::new(),
                        run_without_refresh: false,
                    })]),
                );
            }

            let jump_table_value = Value::GetVar {
                name: ctx.cfg.jump_table_id_var.clone(),
            };
            let jump_table_blocks = binary_search_jump_table(
                jump_table_value,
                jump_table,
                None,
                None,
                None,
                false,
                0,
                ctx.all_check_locations.len() + ctx.setjmp_sites.len() + 1,
            );

            let mut forever_body = BlockList::from_blocks(vec![Block::EditCounter(CounterOp::Reset)]);
            forever_body.add(jump_table_blocks);

            let jump_table_fn_blocks = BlockList::from_blocks(vec![
                Block::ProcedureDef(scratch::ast::ProcedureDefData {
                    name: "!jump table".to_string(),
                    params: Vec::new(),
                    warp: true,
                }),
                Block::ControlFlow(ControlFlow {
                    op: ControlOp::Forever,
                    condition: None,
                    var: None,
                    body: Some(forever_body),
                    else_body: None,
                }),
            ]);
            ctx.proj.code.push(jump_table_fn_blocks);

            blocks.add_block(Block::EditVar(EditVarData {
                op: VarOp::Set,
                name: ctx.cfg.jump_table_id_var.clone(),
                value: Value::Known(KnownVal::Num(super::config::ENTRY_CALL_ID as f64)),
            }));
            blocks.add_block(Block::ProcedureCall(scratch::ast::ProcedureCallData {
                name: "!jump table".to_string(),
                args: Vec::new(),
                run_without_refresh: false,
            }));
        }
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::values::{KnownFloatVal, KnownIntVal, NullPtrVal, UndefVal};

    #[test]
    fn test_localize_var_global() {
        assert_eq!(localize_var("x", true, None, false), "@x");
    }

    #[test]
    fn test_localize_var_local() {
        assert_eq!(localize_var("y", false, None, false), "%y");
    }

    #[test]
    fn test_localize_var_local_with_fn() {
        assert_eq!(localize_var("z", false, Some("main"), false), "%main:z");
    }

    #[test]
    fn test_localize_var_param() {
        assert_eq!(localize_var("p", false, Some("main"), true), "%p");
    }

    #[test]
    fn test_localize_label() {
        assert_eq!(localize_label("then", "main"), "main:then");
    }

    #[test]
    fn test_localize_call_id() {
        assert_eq!(localize_call_id(0, "entry", "foo", false), "foo:entry:return addr 0");
        assert_eq!(localize_call_id(1, "loop", "bar", true), "bar:loop:recursive call 1");
    }

    #[test]
    fn test_localize_param() {
        assert_eq!(localize_param("x"), "%x");
    }

    #[test]
    fn test_gen_temp_var() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let v1 = gen_temp_var(&mut ctx);
        let v2 = gen_temp_var(&mut ctx);
        assert!(v1.starts_with("%!tmp:"));
        assert!(v2.starts_with("%!tmp:"));
        assert_ne!(v1, v2);
    }

    #[test]
    fn test_trans_value_known_int() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let val = ir::Value::KnownInt(KnownIntVal {
            type_: ir::Type::Integer(ir::types::IntegerTy { width: 32 }),
            value: num_bigint::BigUint::from(42u32),
            width: 32,
        });
        let result = trans_value(&val, &mut ctx, None).unwrap();
        assert_eq!(result, InferredValue::Single(Value::Known(KnownVal::Num(42.0))));
    }

    #[test]
    fn test_trans_value_known_float() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let val = ir::Value::KnownFloat(KnownFloatVal {
            type_: ir::Type::Float,
            value: 3.14,
        });
        let result = trans_value(&val, &mut ctx, None).unwrap();
        assert_eq!(result, InferredValue::Single(Value::Known(KnownVal::Num(3.14))));
    }

    #[test]
    fn test_trans_value_null_ptr() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let val = ir::Value::NullPtr(NullPtrVal {
            type_: ir::Type::Pointer(ir::types::PointerTy { addrspace: ir::types::AddrSpace::Default }),
        });
        let result = trans_value(&val, &mut ctx, None).unwrap();
        assert_eq!(result, InferredValue::Single(Value::Known(KnownVal::Num(0.0))));
    }

    #[test]
    fn test_trans_value_undef() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let val = ir::Value::Undef(UndefVal {
            type_: ir::Type::Integer(ir::types::IntegerTy { width: 32 }),
        });
        let result = trans_value(&val, &mut ctx, None).unwrap();
        assert_eq!(result, InferredValue::Single(Value::Known(KnownVal::Num(0.0))));
    }

    #[test]
    fn test_context_new() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let ctx = Context::new(proj, cfg);
        assert!(ctx.fn_info.is_empty());
        assert!(ctx.globvar_to_ptr.is_empty());
        assert_eq!(ctx.next_fn_id, 0);
    }

    #[test]
    fn test_init_local_stack() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let ctx = Context::new(proj, cfg);
        let blocks = init_local_stack(&ctx);
        assert!(!blocks.is_empty());
    }

    #[test]
    fn test_trans_load_single() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let result = Variable {
            var_name: "x".to_string(),
            var_type: VarType::Var,
            fn_name: None,
        };
        let addr = Value::Known(KnownVal::Num(10.0));
        let blocks = trans_load(&result, addr, &ir::Type::Integer(ir::types::IntegerTy { width: 32 }), &mut ctx).unwrap();
        assert!(!blocks.is_empty());
    }

    #[test]
    fn test_trans_store_single() {
        let proj = Project::new(scratch::ast::ScratchConfig::default());
        let cfg = CompilerConfig::default();
        let mut ctx = Context::new(proj, cfg);
        let val = Value::Known(KnownVal::Num(42.0));
        let addr = Value::Known(KnownVal::Num(10.0));
        let blocks = trans_store(InferredValue::Single(val), addr, &ir::Type::Integer(ir::types::IntegerTy { width: 32 }), &mut ctx).unwrap();
        assert!(!blocks.is_empty());
    }
}