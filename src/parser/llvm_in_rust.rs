use indexmap::IndexMap;

use llvm_ir::context::{ConstId, Context, GlobalId, TypeId, ValueRef};
use llvm_ir::function::Function as LlvmFunction;
use llvm_ir::instruction::{InstrKind, FloatPredicate, IntPredicate, TailCallKind};
use llvm_ir::module::Module;
use llvm_ir::types::{FloatKind, StructType, TypeData, FunctionType};
use llvm_ir::value::ConstantData;

use crate::ir::types::*;
use crate::ir::values::*;
use crate::ir::instructions::*;

type DecodedModule = crate::ir::instructions::Module;

pub fn convert_module(
    module: &Module,
    ctx: &Context,
) -> Result<DecodedModule, String> {
    let func_names: Vec<String> = module.functions.iter().map(|f| f.name.clone()).collect();

    let mut global_vars = IndexMap::new();
    for gv in &module.globals {
        let ty = convert_type_id(gv.ty, ctx);
        let init = gv.initializer
            .map(|cid| convert_constant(cid, ctx, module, &func_names))
            .unwrap_or(Value::Undef(UndefVal { type_: ty.clone() }));
        global_vars.insert(gv.name.clone(), GlobalVar {
            name: gv.name.clone(),
            type_: ty,
            is_constant: gv.is_constant,
            init,
        });
    }

    let mut functions = IndexMap::new();
    for func in &module.functions {
        if func.is_declaration {
            continue;
        }
        let (return_type, _params, variadic) = extract_fn_type(func.ty, ctx);

        let mut blocks = IndexMap::new();
        let mut block_names = Vec::new();
        for block in &func.blocks {
            block_names.push(block.name.clone());
            let mut instrs = Vec::new();
            for instr_id in block.instrs() {
                let instr = func.instr(instr_id);
                instrs.push(convert_instr(instr, ctx, func, &func_names, module));
            }
            blocks.insert(block.name.clone(), Block {
                label: block.name.clone(),
                instrs,
            });
        }
        if func.name == "factorial_recurse" || func.name == "sum_to_one_digit" || func.name == "numberize" {
            eprintln!("DEBUG llvm_in_rust {} blocks: {:?}", func.name, block_names);
        }

        let intrinsic = Intrinsic::from_name(&func.name);

        functions.insert(func.name.clone(), Function {
            name: func.name.clone(),
            return_type,
            params: func.args.iter().map(|a| {
                let arg_ty = convert_type_id(a.ty, ctx);
                ArgumentVal { type_: arg_ty, name: a.name.clone() }
            }).collect(),
            variadic,
            intrinsic,
            blocks,
        });
    }

    let name = module.source_filename.clone().unwrap_or_default();

    Ok(DecodedModule { name, functions, global_vars })
}

fn extract_fn_type(ty: TypeId, ctx: &Context) -> (Type, Vec<Type>, bool) {
    match ctx.get_type(ty) {
        TypeData::Function(FunctionType { ret, params, variadic }) => {
            let ret_ty = convert_type_id(*ret, ctx);
            let param_tys = params.iter().map(|p| convert_type_id(*p, ctx)).collect();
            (ret_ty, param_tys, *variadic)
        }
        _ => (Type::Void, vec![], false),
    }
}

fn convert_type_id(type_id: TypeId, ctx: &Context) -> Type {
    let td = ctx.get_type(type_id);
    convert_type_data(td, ctx)
}

fn convert_type_data(td: &TypeData, ctx: &Context) -> Type {
    match td {
        TypeData::Void => Type::Void,
        TypeData::Integer(bits) => Type::Integer(IntegerTy { width: *bits }),
        TypeData::Float(fk) => match fk {
            FloatKind::Half => Type::Half,
            FloatKind::Single => Type::Float,
            FloatKind::Double => Type::Double,
            FloatKind::Fp128 => Type::Fp128,
            _ => Type::Float,
        },
        TypeData::Pointer => Type::Pointer(PointerTy::new(AddrSpace::Default)),
        TypeData::Array { element, len } => {
            let inner = convert_type_id(*element, ctx);
            Type::Array(ArrayTy { inner: Box::new(inner), size: *len as u32 })
        }
        TypeData::Vector { element, len, scalable } => {
            if *scalable {
                let inner = convert_type_id(*element, ctx);
                Type::Array(ArrayTy { inner: Box::new(inner), size: *len })
            } else {
                let inner = convert_type_id(*element, ctx);
                Type::Vector(VecTy { inner: Box::new(inner), size: *len })
            }
        }
        TypeData::Struct(StructType { fields, packed, .. }) => {
            let members: Vec<Type> = fields.iter().map(|f| convert_type_id(*f, ctx)).collect();
            Type::Struct(StructTy { is_packed: *packed, members })
        }
        TypeData::Function(FunctionType { ret, params, variadic }) => {
            let ret_ty = convert_type_id(*ret, ctx);
            let param_tys: Vec<Type> = params.iter().map(|p| convert_type_id(*p, ctx)).collect();
            Type::Func(FuncTy {
                return_type: Box::new(ret_ty),
                params: param_tys,
                variadic: *variadic,
            })
        }
        TypeData::Label => Type::Label,
        TypeData::Metadata => Type::Metadata,
    }
}

/// Convert a ValueRef that appears inside a constant expression. Only constants
/// and global references are valid here.
fn convert_const_value_ref(vref: &ValueRef, ctx: &Context, module: &Module, func_names: &[String]) -> Value {
    match vref {
        ValueRef::Constant(id) => convert_constant(*id, ctx, module, func_names),
        ValueRef::Global(id) => {
            // Global references inside constant expressions are not expected,
            // but treat them as null pointers to avoid panics.
            let _ = id;
            Value::NullPtr(NullPtrVal { type_: Type::Pointer(PointerTy::new(AddrSpace::Default)) })
        }
        _ => panic!("unexpected non-constant value in constant expression: {:?}", vref),
    }
}

fn convert_constant(const_id: ConstId, ctx: &Context, module: &Module, func_names: &[String]) -> Value {
    let cd = ctx.get_const(const_id);
    match cd {
        ConstantData::Int { ty, val } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let width = get_int_width(*ty, ctx);
            Value::KnownInt(KnownIntVal { type_: proj_ty, value: *val as u128, width })
        }
        ConstantData::IntWide { ty, words } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let width = get_int_width(*ty, ctx);
            let mut val: u128 = 0;
            for (i, w) in words.iter().enumerate() {
                if i < 2 {
                    val |= (*w as u128) << (i * 64);
                }
            }
            Value::KnownInt(KnownIntVal { type_: proj_ty, value: val, width })
        }
        ConstantData::Float { ty, bits } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let f = f64::from_bits(*bits);
            Value::KnownFloat(KnownFloatVal { type_: proj_ty, value: f })
        }
        ConstantData::Null(ty) => {
            let proj_ty = convert_type_id(*ty, ctx);
            Value::NullPtr(NullPtrVal { type_: proj_ty })
        }
        ConstantData::Undef(ty) => {
            let proj_ty = convert_type_id(*ty, ctx);
            Value::Undef(UndefVal { type_: proj_ty })
        }
        ConstantData::Poison(ty) => {
            let proj_ty = convert_type_id(*ty, ctx);
            Value::Undef(UndefVal { type_: proj_ty })
        }
        ConstantData::ZeroInitializer(ty) => {
            let proj_ty = convert_type_id(*ty, ctx);
            get_zero_init_val(&proj_ty)
        }
        ConstantData::Array { ty, elements } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let vals: Vec<Value> = elements.iter().map(|e| convert_constant(*e, ctx, module, func_names)).collect();
            Value::KnownArr(KnownArrVal { type_: proj_ty, values: vals })
        }
        ConstantData::Struct { ty, fields } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let vals: Vec<Value> = fields.iter().map(|f| convert_constant(*f, ctx, module, func_names)).collect();
            Value::KnownStruct(KnownStructVal { type_: proj_ty, values: vals })
        }
        ConstantData::Vector { ty, elements } => {
            let proj_ty = convert_type_id(*ty, ctx);
            let vals: Vec<Value> = elements.iter().map(|e| convert_constant(*e, ctx, module, func_names)).collect();
            Value::KnownVec(KnownVecVal { type_: proj_ty, values: vals })
        }
        ConstantData::GlobalRef { ty, name, id } => {
            if func_names.contains(name) || *id == GlobalId(u32::MAX) {
                // Function reference (function pointer).
                if let Some(func) = module.functions.iter().find(|f| &f.name == name) {
                    let fn_ty = convert_type_id(func.ty, ctx);
                    Value::Function(FunctionVal { type_: fn_ty, name: name.clone() })
                } else {
                    // Forward declaration / unresolved function pointer.
                    let proj_ty = convert_type_id(*ty, ctx);
                    Value::GlobalPtr(GlobalPtrVal { type_: proj_ty, name: name.clone() })
                }
            } else {
                let proj_ty = convert_type_id(*ty, ctx);
                Value::GlobalPtr(GlobalPtrVal { type_: proj_ty, name: name.clone() })
            }
        }
        ConstantData::GetElementPtr { ty, inbounds, base_ty, ptr, indices } => {
            let proj_ty = convert_type_id(*ty, ctx);
            Value::ConstExpr(ConstExprVal {
                type_: proj_ty,
                expr: ConstExpr::GetElementPtr(Box::new(GetElementPtr {
                    result: ResultLocalVar::new(""),
                    base_ptr_type: convert_type_id(*base_ty, ctx),
                    base_ptr: convert_const_value_ref(ptr, ctx, module, func_names),
                    indices: indices.iter().map(|i| convert_const_value_ref(i, ctx, module, func_names)).collect(),
                    is_inbounds: *inbounds,
                    is_nusw: false,
                    is_nuw: false,
                })),
            })
        }
    }
}

fn get_int_width(ty: TypeId, ctx: &Context) -> u32 {
    match ctx.get_type(ty) {
        TypeData::Integer(bits) => *bits,
        _ => 0,
    }
}

fn get_zero_init_val(ty: &Type) -> Value {
    match ty {
        Type::Integer(IntegerTy { width }) => Value::KnownInt(KnownIntVal {
            type_: ty.clone(),
            value: 0,
            width: *width,
        }),
        Type::Float | Type::Double | Type::Half | Type::Fp128 => {
            Value::KnownFloat(KnownFloatVal { type_: ty.clone(), value: 0.0 })
        }
        Type::Pointer(_) => Value::NullPtr(NullPtrVal { type_: ty.clone() }),
        Type::Struct(StructTy { members, .. }) => {
            let vals: Vec<Value> = members.iter().map(|m| get_zero_init_val(m)).collect();
            Value::KnownStruct(KnownStructVal { type_: ty.clone(), values: vals })
        }
        Type::Array(ArrayTy { inner, size }) => {
            let vals: Vec<Value> = (0..*size).map(|_| get_zero_init_val(inner)).collect();
            Value::KnownArr(KnownArrVal { type_: ty.clone(), values: vals })
        }
        Type::Vector(VecTy { inner, size }) => {
            let vals: Vec<Value> = (0..*size).map(|_| get_zero_init_val(inner)).collect();
            Value::KnownVec(KnownVecVal { type_: ty.clone(), values: vals })
        }
        _ => Value::NullPtr(NullPtrVal { type_: ty.clone() }),
    }
}

fn convert_value_ref(vref: &ValueRef, ctx: &Context, func: &LlvmFunction, func_names: &[String], module: &Module) -> Value {
    match vref {
        ValueRef::Instruction(instr_id) => {
            let instr = func.instr(*instr_id);
            let ty = convert_type_id(instr.ty, ctx);
            let name = instr.name.clone().unwrap_or_default();
            Value::LocalVar(LocalVarVal { type_: ty, name })
        }
        ValueRef::Argument(arg_id) => {
            let arg = func.arg(*arg_id);
            let ty = convert_type_id(arg.ty, ctx);
            Value::Argument(ArgumentVal { type_: ty, name: arg.name.clone() })
        }
        ValueRef::Constant(const_id) => convert_constant(*const_id, ctx, module, func_names),
        ValueRef::Global(global_id) => {
            let gv = &module.globals[global_id.0 as usize];
            let name = gv.name.clone();
            let ty = convert_type_id(gv.ty, ctx);
            if func_names.contains(&name) {
                Value::Function(FunctionVal { type_: ty, name })
            } else {
                Value::GlobalPtr(GlobalPtrVal { type_: ty, name })
            }
        }
    }
}

fn convert_block_label(block_id: &llvm_ir::context::BlockId, func: &LlvmFunction) -> String {
    let block = func.block(*block_id);
    block.name.clone()
}

fn convert_instr(instr: &llvm_ir::instruction::Instruction, ctx: &Context, func: &LlvmFunction, func_names: &[String], module: &Module) -> Instr {
    let result = instr.name.as_ref().map(|n| ResultLocalVar::new(n.clone()));
    match &instr.kind {
        InstrKind::Ret { val } => {
            Instr::Ret(Ret {
                value: val.as_ref().map(|v| convert_value_ref(v, ctx, func, func_names, module)),
            })
        }
        InstrKind::Br { dest } => {
            Instr::UncondBr(UncondBr {
                branch: LabelVal { type_: Type::Label, label: convert_block_label(dest, func) },
            })
        }
        InstrKind::CondBr { cond, then_dest, else_dest } => {
            Instr::CondBr(CondBr {
                cond: convert_value_ref(cond, ctx, func, func_names, module),
                branch_true: LabelVal { type_: Type::Label, label: convert_block_label(then_dest, func) },
                branch_false: LabelVal { type_: Type::Label, label: convert_block_label(else_dest, func) },
            })
        }
        InstrKind::Switch { val, default, cases } => {
            Instr::Switch(Switch {
                cond: convert_value_ref(val, ctx, func, func_names, module),
                branch_default: LabelVal { type_: Type::Label, label: convert_block_label(default, func) },
                branch_table: cases.iter().map(|(v, b)| {
                    (convert_value_ref(v, ctx, func, func_names, module), LabelVal { type_: Type::Label, label: convert_block_label(b, func) })
                }).collect(),
            })
        }
        InstrKind::Unreachable => Instr::Unreachable,
        InstrKind::FNeg { operand, .. } => {
            Instr::UnaryOp(UnaryOp {
                result: result.unwrap(),
                opcode: UnaryOpcode::FNeg,
                operand: convert_value_ref(operand, ctx, func, func_names, module),
            })
        }
        InstrKind::Add { flags, lhs, rhs }
        | InstrKind::Sub { flags, lhs, rhs }
        | InstrKind::Mul { flags, lhs, rhs }
        | InstrKind::Shl { flags, lhs, rhs } => {
            let opcode = match &instr.kind {
                InstrKind::Add { .. } => BinaryOpcode::Add,
                InstrKind::Sub { .. } => BinaryOpcode::Sub,
                InstrKind::Mul { .. } => BinaryOpcode::Mul,
                InstrKind::Shl { .. } => BinaryOpcode::Shl,
                _ => unreachable!(),
            };
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: flags.nuw,
                is_nsw: flags.nsw,
                is_exact: false,
                is_disjoint: false,
            })
        }
        InstrKind::UDiv { exact, lhs, rhs }
        | InstrKind::SDiv { exact, lhs, rhs }
        | InstrKind::LShr { exact, lhs, rhs }
        | InstrKind::AShr { exact, lhs, rhs } => {
            let opcode = match &instr.kind {
                InstrKind::UDiv { .. } => BinaryOpcode::UDiv,
                InstrKind::SDiv { .. } => BinaryOpcode::SDiv,
                InstrKind::LShr { .. } => BinaryOpcode::LShr,
                InstrKind::AShr { .. } => BinaryOpcode::AShr,
                _ => unreachable!(),
            };
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false,
                is_nsw: false,
                is_exact: *exact,
                is_disjoint: false,
            })
        }
        InstrKind::URem { lhs, rhs } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::URem,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::SRem { lhs, rhs } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::SRem,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::And { lhs, rhs } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::And,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::Or { lhs, rhs } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::Or,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::Xor { lhs, rhs } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::Xor,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::FAdd { lhs, rhs, .. } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::FAdd,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::FSub { lhs, rhs, .. } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::FSub,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::FMul { lhs, rhs, .. } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::FMul,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::FDiv { lhs, rhs, .. } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::FDiv,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::FRem { lhs, rhs, .. } => {
            Instr::BinaryOp(BinaryOp {
                result: result.unwrap(),
                opcode: BinaryOpcode::FRem,
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_nuw: false, is_nsw: false, is_exact: false, is_disjoint: false,
            })
        }
        InstrKind::ExtractElement { vec, idx } => {
            Instr::ExtractElement(ExtractElement {
                result: result.unwrap(),
                agg: convert_value_ref(vec, ctx, func, func_names, module),
                index: convert_value_ref(idx, ctx, func, func_names, module),
            })
        }
        InstrKind::InsertElement { vec, val, idx } => {
            Instr::InsertElement(InsertElement {
                result: result.unwrap(),
                agg: convert_value_ref(vec, ctx, func, func_names, module),
                item: convert_value_ref(val, ctx, func, func_names, module),
                index: convert_value_ref(idx, ctx, func, func_names, module),
            })
        }
        InstrKind::ShuffleVector { v1, v2, mask } => {
            let mask_val = Value::KnownVec(KnownVecVal {
                type_: Type::Vector(VecTy { inner: Box::new(Type::Integer(IntegerTy { width: 32 })), size: mask.len() as u32 }),
                values: mask.iter().map(|&i| {
                    Value::KnownInt(KnownIntVal {
                        type_: Type::Integer(IntegerTy { width: 32 }),
                        value: i as u128,
                        width: 32,
                    })
                }).collect(),
            });
            Instr::ShuffleVector(ShuffleVector {
                result: result.unwrap(),
                fst_vector: convert_value_ref(v1, ctx, func, func_names, module),
                snd_vector: convert_value_ref(v2, ctx, func, func_names, module),
                mask_vector: mask_val,
            })
        }
        InstrKind::ExtractValue { aggregate, indices } => {
            Instr::ExtractValue(ExtractValue {
                result: result.unwrap(),
                agg: convert_value_ref(aggregate, ctx, func, func_names, module),
                indices: indices.clone(),
            })
        }
        InstrKind::InsertValue { aggregate, val, indices } => {
            Instr::InsertValue(InsertValue {
                result: result.unwrap(),
                agg: convert_value_ref(aggregate, ctx, func, func_names, module),
                element: convert_value_ref(val, ctx, func, func_names, module),
                indices: indices.clone(),
            })
        }
        InstrKind::Alloca { alloc_ty, num_elements, .. } => {
            let allocated_type = convert_type_id(*alloc_ty, ctx);
            let num_el = num_elements.as_ref()
                .map(|v| convert_value_ref(v, ctx, func, func_names, module))
                .unwrap_or(Value::KnownInt(KnownIntVal {
                    type_: Type::Integer(IntegerTy { width: 32 }),
                    value: 1,
                    width: 32,
                }));
            Instr::Alloca(Alloca {
                result: result.unwrap(),
                allocated_type,
                num_elements: num_el,
            })
        }
        InstrKind::Load { ty, ptr, .. } => {
            Instr::Load(Load {
                result: result.unwrap(),
                loaded_type: convert_type_id(*ty, ctx),
                address: convert_value_ref(ptr, ctx, func, func_names, module),
            })
        }
        InstrKind::Store { val, ptr, .. } => {
            Instr::Store(Store {
                value: convert_value_ref(val, ctx, func, func_names, module),
                address: convert_value_ref(ptr, ctx, func, func_names, module),
            })
        }
        InstrKind::GetElementPtr { inbounds, base_ty, ptr, indices } => {
            Instr::GetElementPtr(GetElementPtr {
                result: result.unwrap(),
                base_ptr_type: convert_type_id(*base_ty, ctx),
                base_ptr: convert_value_ref(ptr, ctx, func, func_names, module),
                indices: indices.iter().map(|i| convert_value_ref(i, ctx, func, func_names, module)).collect(),
                is_inbounds: *inbounds,
                is_nusw: false,
                is_nuw: false,
            })
        }
        InstrKind::Trunc { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(),
                opcode: ConvOpcode::Trunc,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::ZExt { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::ZExt,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::SExt { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::SExt,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::FPTrunc { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::FPTrunc,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::FPExt { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::FPExt,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::FPToUI { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::FPToUI,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::FPToSI { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::FPToSI,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::UIToFP { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::UIToFP,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::SIToFP { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::SIToFP,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::PtrToInt { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::PtrToInt,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::IntToPtr { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::IntToPtr,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::BitCast { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::BitCast,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::AddrSpaceCast { val, to } => {
            Instr::Conversion(Conversion {
                result: result.unwrap(), opcode: ConvOpcode::AddrSpaceCast,
                value: convert_value_ref(val, ctx, func, func_names, module),
                res_type: convert_type_id(*to, ctx),
                is_nuw: false, is_nsw: false,
            })
        }
        InstrKind::ICmp { pred, lhs, rhs } => {
            Instr::ICmp(ICmp {
                result: result.unwrap(),
                cond: convert_int_predicate(*pred),
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
                is_samesign: false,
            })
        }
        InstrKind::FCmp { pred, lhs, rhs, .. } => {
            Instr::FCmp(FCmp {
                result: result.unwrap(),
                cond: convert_float_predicate(*pred),
                left: convert_value_ref(lhs, ctx, func, func_names, module),
                right: convert_value_ref(rhs, ctx, func, func_names, module),
            })
        }
        InstrKind::Phi { incoming, .. } => {
            Instr::Phi(Phi {
                result: result.unwrap(),
                incoming: incoming.iter().map(|(v, b)| {
                    (convert_value_ref(v, ctx, func, func_names, module),
                     LabelVal { type_: Type::Label, label: convert_block_label(b, func) })
                }).collect(),
            })
        }
        InstrKind::Select { cond, then_val, else_val } => {
            Instr::Select(Select {
                result: result.unwrap(),
                cond: convert_value_ref(cond, ctx, func, func_names, module),
                true_value: convert_value_ref(then_val, ctx, func, func_names, module),
                false_value: convert_value_ref(else_val, ctx, func, func_names, module),
            })
        }
        InstrKind::Freeze { val } => {
            Instr::Freeze(Freeze {
                result: result.unwrap(),
                value: convert_value_ref(val, ctx, func, func_names, module),
            })
        }
        InstrKind::Call { callee_ty, callee, args, tail, .. } => {
            let (return_type, param_types, variadic) = extract_fn_type(*callee_ty, ctx);
            let mut callee_val = convert_value_ref(callee, ctx, func, func_names, module);
            // If the parser returned a GlobalPtr but it's actually a function,
            // convert it so the rest of the pipeline sees Value::Function.
            if let Value::GlobalPtr(ref gp) = callee_val {
                if func_names.contains(&gp.name) {
                    callee_val = Value::Function(FunctionVal {
                        type_: gp.type_.clone(),
                        name: gp.name.clone(),
                    });
                }
            }
            let arg_vals: Vec<Value> = args.iter().map(|a| convert_value_ref(a, ctx, func, func_names, module)).collect();
            let tail_kind = match tail {
                TailCallKind::None => CallTailKind::NoTail,
                TailCallKind::Tail => CallTailKind::Tail,
                TailCallKind::MustTail => CallTailKind::MustTail,
                TailCallKind::NoTail => CallTailKind::NoTail,
            };
            let fn_name = match callee_val {
                Value::Function(ref fv) => fv.name.clone(),
                _ => String::new(),
            };
            let intrinsic = Intrinsic::from_name(&fn_name);

            Instr::Call(Call {
                result: if return_type != Type::Void { result } else { None },
                func: callee_val,
                return_type,
                args: arg_vals,
                params: param_types,
                variadic,
                tail_kind,
                intrinsic,
                callees: vec![],
            })
        }
    }
}

fn convert_int_predicate(pred: IntPredicate) -> ICmpCond {
    match pred {
        IntPredicate::Eq => ICmpCond::Eq,
        IntPredicate::Ne => ICmpCond::Ne,
        IntPredicate::Ugt => ICmpCond::Ugt,
        IntPredicate::Uge => ICmpCond::Uge,
        IntPredicate::Ult => ICmpCond::Ult,
        IntPredicate::Ule => ICmpCond::Ule,
        IntPredicate::Sgt => ICmpCond::Sgt,
        IntPredicate::Sge => ICmpCond::Sge,
        IntPredicate::Slt => ICmpCond::Slt,
        IntPredicate::Sle => ICmpCond::Sle,
    }
}

fn convert_float_predicate(pred: FloatPredicate) -> FCmpCond {
    match pred {
        FloatPredicate::False => FCmpCond::FalseCond,
        FloatPredicate::Oeq => FCmpCond::Oeq,
        FloatPredicate::Ogt => FCmpCond::Ogt,
        FloatPredicate::Oge => FCmpCond::Oge,
        FloatPredicate::Olt => FCmpCond::Olt,
        FloatPredicate::Ole => FCmpCond::Ole,
        FloatPredicate::One => FCmpCond::One,
        FloatPredicate::Ord => FCmpCond::Ord,
        FloatPredicate::Ueq => FCmpCond::Ueq,
        FloatPredicate::Ugt => FCmpCond::Ugt,
        FloatPredicate::Uge => FCmpCond::Uge,
        FloatPredicate::Ult => FCmpCond::Ult,
        FloatPredicate::Ule => FCmpCond::Ule,
        FloatPredicate::Une => FCmpCond::Une,
        FloatPredicate::Uno => FCmpCond::Uno,
        FloatPredicate::True => FCmpCond::TrueCond,
    }
}
