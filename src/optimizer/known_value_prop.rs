//! Known value propagation optimizer.
//!
//! Faithful Rust port of `llvm2scratch/optimizer.py` (`partialSimplifyValue`,
//! `simplifyValue`, `knownValuePropagationBlock`, `knownValuePropagation`).
//!
//! Behaviour notes (preserved verbatim from the Python authoritative reference):
//! - `KnownBool` is treated as a `Known` (anywhere the Python code tests
//!   `isinstance(x, Known)`, `KnownBool` matches too).
//! - `GetVar` is *not* simplified through a lookup function. Only `GetOfList`
//!   uses the `list_lookup` callback (which mirrors Python's `lookup_func`).
//! - The `Op` match only folds `add / sub / mul / div / mod / bool_to_float /
//!   str_to_float / abs / floor / ceiling`. The `case _:` branch resets
//!   `did_opti_total = False` even when children changed — this is preserved
//!   exactly (it is a quirk of the Python source).
//! - The `case _: did_opti_total |= False` for unmatched top-level values is
//!   also preserved.
//! - `inverted = block == "while"` in Python is a bug (compares a Block to a
//!   str, always False); ported as `inverted = false`.
//! - `value = unknown` (when `combined_known == 0`) inside the constant-merge
//!   rule uses `unknown` rather than `inner_unknown`; this is also a Python
//!   quirk and is preserved.

use crate::scratch::{Block, BlockList, Project, Value};
use crate::scratch::ast::{BoolOp, ControlFlow, ControlOp, EditVarData, GetOfList, KnownVal, Op};
#[cfg(test)]
use crate::scratch::ast::VarOp;

/// List lookup callback. Mirrors Python's `lookup_func(list_name, index_value)`.
pub type ListLookup<'a> = &'a dyn Fn(&str, &Value) -> Option<Value>;

// ---------------------------------------------------------------------------
// Helper predicates
// ---------------------------------------------------------------------------

#[inline]
fn is_known(v: &Value) -> bool {
    // Python: `isinstance(x, Known)` — KnownBool is a subclass of Known.
    matches!(v, Value::Known(_) | Value::KnownBool(_))
}

#[inline]
fn is_known_bool(v: &Value) -> bool {
    matches!(v, Value::KnownBool(_))
}

// ---------------------------------------------------------------------------
// Cast helpers (mirror llvm2scratch/scratch.py)
// ---------------------------------------------------------------------------

/// `scratchCastToNum` — NaN produced by parse failure is replaced with 0.0.
fn known_to_num(v: &Value) -> f64 {
    match v {
        Value::Known(KnownVal::Num(n)) => *n,
        Value::Known(KnownVal::Str(s)) => s.parse::<f64>().unwrap_or(0.0),
        Value::Known(KnownVal::Bool(b)) => if *b { 1.0 } else { 0.0 },
        Value::KnownBool(b) => if *b { 1.0 } else { 0.0 },
        _ => 0.0,
    }
}

/// `scratchCastToBool`.
fn known_to_bool(v: &Value) -> bool {
    match v {
        Value::Known(KnownVal::Str(s)) => {
            let lower = s.to_lowercase();
            !lower.is_empty() && lower != "0" && lower != "false"
        }
        Value::Known(KnownVal::Num(n)) => *n != 0.0 && !n.is_nan(),
        Value::Known(KnownVal::Bool(b)) => *b,
        Value::KnownBool(b) => *b,
        _ => false,
    }
}

/// Numeric view of a Known, preserving NaN (used by `scratchCompare`).
fn value_to_num_opt(v: &Value) -> Option<f64> {
    match v {
        Value::Known(KnownVal::Num(n)) => Some(*n),
        Value::Known(KnownVal::Str(s)) => s.parse::<f64>().ok(),
        Value::Known(KnownVal::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        Value::KnownBool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// `scratchCastToStr` (used by scratchCompare fallback).
fn known_to_str(v: &Value) -> String {
    match v {
        Value::Known(KnownVal::Str(s)) => s.clone(),
        Value::Known(KnownVal::Num(n)) => num_to_pystr(*n),
        Value::Known(KnownVal::Bool(b)) => if *b { "true".to_string() } else { "false".to_string() },
        Value::KnownBool(b) => if *b { "true".to_string() } else { "false".to_string() },
        _ => String::new(),
    }
}

/// Lowercased string view (used by scratchCompare fallback and bool_to_float
/// comparison rule). Matches Python `scratchCastToStr(x).lower()`.
fn value_to_str_lower(v: &Value) -> String {
    known_to_str(v).to_lowercase()
}

/// Format an f64 the way Python `str(float)` would, for comparison purposes.
/// We only need a stable, faithful representation for `scratchCompare`'s string
/// fallback, so a normal Rust float→string is sufficient here (the values
/// originate from `Known` numerics).
fn num_to_pystr(n: f64) -> String {
    if n.is_nan() {
        "nan".to_string()
    } else if n.is_infinite() {
        if n.is_sign_positive() { "inf".to_string() } else { "-inf".to_string() }
    } else if n.fract() == 0.0 && n.abs() < 1e16 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/// `scratchCompare` — returns negative if left<right, 0 if equal, positive otherwise.
fn known_compare(left: &Value, right: &Value) -> f64 {
    match (value_to_num_opt(left), value_to_num_opt(right)) {
        (Some(l), Some(r)) => {
            if l.is_infinite() && r.is_infinite()
                && ((l.is_sign_positive() && r.is_sign_positive())
                    || (l.is_sign_negative() && r.is_sign_negative()))
            {
                0.0
            } else {
                l - r
            }
        }
        _ => {
            let ls = value_to_str_lower(left);
            let rs = value_to_str_lower(right);
            if ls == rs { 0.0 } else if ls < rs { -1.0 } else { 1.0 }
        }
    }
}

/// `get_sign = lambda x: (-1, 1)[str(float(x))[0] == "-"]` from Python's div rule.
fn get_sign(x: f64) -> f64 {
    // Python `str(float(x))[0] == "-"` — sign based on the string repr.
    // For -0.0 this yields "-0.0" so sign is -1; for +0.0 it yields "0.0" → +1.
    // For NaN Python produces "nan" (no sign) → +1.
    let s = num_to_pystr(x);
    if s.starts_with('-') { -1.0 } else { 1.0 }
}

/// Scratch's modulo uses floor division semantics.
fn scratch_mod(left: f64, right: f64) -> f64 {
    if right == 0.0 {
        return f64::NAN;
    }
    let div = (left / right).floor();
    left - right * div
}

// ---------------------------------------------------------------------------
// getKnownAndUnknown — returns (known, unknown, left_is_known)
// ---------------------------------------------------------------------------

fn op_right_opt(op: &Op) -> Option<&Value> {
    op.right_opt()
}

fn boolop_right_opt(bop: &BoolOp) -> Option<&Value> {
    match bop {
        BoolOp::And(_, _) | BoolOp::Or(_, _) | BoolOp::Eq(_, _) |
        BoolOp::Lt(_, _) | BoolOp::Gt(_, _) => Some(bop.right()),
        BoolOp::Not(_) => None,
    }
}

/// Returns `(known, unknown, left_is_known)` if exactly one side is Known.
fn get_known_and_unknown_op(op: &Op) -> Option<(Value, Value, bool)> {
    if op_right_opt(op).is_none() {
        return None;
    }
    let left = op.left();
    let right = op.right();
    if is_known(left) {
        Some((left.clone(), right.clone(), true))
    } else if is_known(right) {
        Some((right.clone(), left.clone(), false))
    } else {
        None
    }
}

fn get_known_and_unknown_boolop(bop: &BoolOp) -> Option<(Value, Value, bool)> {
    if boolop_right_opt(bop).is_none() {
        return None;
    }
    let left = bop.left();
    let right = bop.right();
    if is_known(left) {
        Some((left.clone(), right.clone(), true))
    } else if is_known(right) {
        Some((right.clone(), left.clone(), false))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// partialSimplifyValue
// ---------------------------------------------------------------------------

pub fn partial_simplify_value(value: &Value, list_lookup: ListLookup<'_>) -> (Value, bool) {
    match value {
        Value::BoolOp(bop) => simplify_bool_op(bop, list_lookup),
        Value::Op(op) => simplify_op(op, list_lookup),
        Value::GetOfList(gol) => {
            let (new_inner, did_inner) = partial_simplify_value(&gol.value, list_lookup);
            let mut did_opti_total = did_inner;
            let mut result_val = value.clone();
            if did_inner {
                result_val = Value::GetOfList(GetOfList {
                    op: gol.op,
                    name: gol.name.clone(),
                    value: Box::new(new_inner),
                });
            }
            // Simplify lookup tables — only when the index is Known
            // (Python: `isinstance(value.value, sb3.Known)` — KnownBool counts).
            if let Value::GetOfList(g) = &result_val {
                if is_known(&g.value) {
                    if let Some(looked_up) = list_lookup(&g.name, &g.value) {
                        result_val = looked_up;
                        did_opti_total = true;
                    }
                }
            }
            (result_val, did_opti_total)
        }
        _ => (value.clone(), false),
    }
}

fn simplify_bool_op(bop: &BoolOp, list_lookup: ListLookup<'_>) -> (Value, bool) {
    let (left, did_1) = partial_simplify_value(bop.left(), list_lookup);
    let mut did_2 = false;
    let mut right = bop.right().clone();
    if boolop_right_opt(bop).is_some() {
        let (r, d) = partial_simplify_value(bop.right(), list_lookup);
        right = r;
        did_2 = d;
    }
    let did_opti_total = did_1 || did_2;

    // Rule A: `not`
    if let BoolOp::Not(_) = bop {
        if is_known_bool(&left) {
            return (Value::KnownBool(!known_to_bool(&left)), true);
        }
        if let Value::BoolOp(BoolOp::Not(inner)) = &left {
            return ((**inner).clone(), true);
        }
    }

    // Rule B: both sides Known — compare / and-or evaluate
    if is_known(&left) && is_known(&right) {
        match bop {
            BoolOp::Eq(_, _) | BoolOp::Lt(_, _) | BoolOp::Gt(_, _) => {
                let cmp = known_compare(&left, &right);
                let result = match bop {
                    BoolOp::Lt(_, _) => cmp < 0.0,
                    BoolOp::Gt(_, _) => cmp > 0.0,
                    BoolOp::Eq(_, _) => cmp == 0.0,
                    _ => unreachable!(),
                };
                return (Value::KnownBool(result), true);
            }
            BoolOp::And(_, _) | BoolOp::Or(_, _) => {
                let l = known_to_bool(&left);
                let r = known_to_bool(&right);
                let result = match bop {
                    BoolOp::And(_, _) => l && r,
                    BoolOp::Or(_, _) => l || r,
                    _ => unreachable!(),
                };
                return (Value::KnownBool(result), true);
            }
            _ => {}
        }
    }

    // Rule C: and/or with one Known side — short-circuit
    if matches!(bop, BoolOp::And(_, _) | BoolOp::Or(_, _))
        && (is_known(&left) || is_known(&right))
        && !(is_known(&left) && is_known(&right))
    {
        let (known, unknown) = if is_known(&left) {
            (left.clone(), right.clone())
        } else {
            (right.clone(), left.clone())
        };
        let known_val = known_to_bool(&known);
        let result = match bop {
            BoolOp::And(_, _) => {
                if !known_val { Value::KnownBool(false) } else { unknown }
            }
            BoolOp::Or(_, _) => {
                if known_val { Value::KnownBool(true) } else { unknown }
            }
            _ => unreachable!(),
        };
        return (result, true);
    }

    // Rule D: comparison + (add/sub) + Known — move known across both sides
    if matches!(bop, BoolOp::Eq(_, _) | BoolOp::Lt(_, _) | BoolOp::Gt(_, _))
        && (is_known(&left) || is_known(&right))
        && (
            (matches!(&left, Value::Op(o) if matches!(o, Op::Add(..) | Op::Sub(..)))
                ^ matches!(&right, Value::Op(o) if matches!(o, Op::Add(..) | Op::Sub(..))))
        )
    {
        if let Some((known_comp, unknown_comp, known_comp_is_left)) = get_known_and_unknown_boolop(bop) {
            if let Value::Op(unknown_op) = &unknown_comp {
                if let Some((known_add, unknown_add, known_add_is_left)) = get_known_and_unknown_op(unknown_op) {
                    let known_comp_is_numlike = matches!(&known_comp,
                        Value::Known(KnownVal::Num(_)) | Value::Known(KnownVal::Bool(_)) | Value::KnownBool(_));
                    let known_add_is_numlike = matches!(&known_add,
                        Value::Known(KnownVal::Num(_)) | Value::Known(KnownVal::Bool(_)) | Value::KnownBool(_));
                    let unknown_op_is_sub = matches!(unknown_op, Op::Sub(..));

                    if known_comp_is_numlike && known_add_is_numlike
                        && !(known_add_is_left && unknown_op_is_sub)
                    {
                        let value_added = if matches!(unknown_op, Op::Add(..)) {
                            known_to_num(&known_add)
                        } else {
                            -known_to_num(&known_add)
                        };
                        let new_known_comp = Value::Known(KnownVal::Num(known_to_num(&known_comp) - value_added));
                        let (l, r) = if known_comp_is_left {
                            (new_known_comp, unknown_add.clone())
                        } else {
                            (unknown_add.clone(), new_known_comp)
                        };
                        let new_bop = match bop {
                            BoolOp::Eq(_, _) => BoolOp::Eq(Box::new(l), Box::new(r)),
                            BoolOp::Lt(_, _) => BoolOp::Lt(Box::new(l), Box::new(r)),
                            BoolOp::Gt(_, _) => BoolOp::Gt(Box::new(l), Box::new(r)),
                            _ => unreachable!(),
                        };
                        return (Value::BoolOp(new_bop), true);
                    }
                }
            }
        }
    }

    // Rule E: comparison + bool_to_float + Known
    if matches!(bop, BoolOp::Eq(_, _) | BoolOp::Lt(_, _) | BoolOp::Gt(_, _))
        && (matches!(&left, Value::Op(o) if matches!(o, Op::BoolToFloat(..)))
            ^ matches!(&right, Value::Op(o) if matches!(o, Op::BoolToFloat(..))))
        && (is_known(&left) ^ is_known(&right))
    {
        let mut op_str = match bop {
            BoolOp::Eq(_, _) => "=",
            BoolOp::Lt(_, _) => "<",
            BoolOp::Gt(_, _) => ">",
            _ => unreachable!(),
        };
        let (unknown, known) = if matches!(&right, Value::Op(o) if matches!(o, Op::BoolToFloat(..))) {
            match op_str {
                "<" => op_str = ">=",
                ">" => op_str = "<=",
                _ => {}
            }
            (right.clone(), left.clone())
        } else {
            (left.clone(), right.clone())
        };

        let known_val_lowered: Value = if let Value::Known(KnownVal::Str(s)) = &known {
            Value::Known(KnownVal::Str(s.to_lowercase()))
        } else {
            known.clone()
        };

        let (value_true, value_false): (Value, Value) = if matches!(known_val_lowered, Value::Known(KnownVal::Str(_))) {
            (Value::Known(KnownVal::Str("true".to_string())), Value::Known(KnownVal::Str("false".to_string())))
        } else {
            (Value::Known(KnownVal::Num(1.0)), Value::Known(KnownVal::Num(0.0)))
        };

        let result_true = match op_str {
            "=" => known_compare(&value_true, &known_val_lowered) == 0.0,
            ">" => known_compare(&value_true, &known_val_lowered) > 0.0,
            "<" => known_compare(&value_true, &known_val_lowered) < 0.0,
            ">=" => known_compare(&value_true, &known_val_lowered) >= 0.0,
            "<=" => known_compare(&value_true, &known_val_lowered) <= 0.0,
            _ => unreachable!(),
        };
        let result_false = match op_str {
            "=" => known_compare(&value_false, &known_val_lowered) == 0.0,
            ">" => known_compare(&value_false, &known_val_lowered) > 0.0,
            "<" => known_compare(&value_false, &known_val_lowered) < 0.0,
            ">=" => known_compare(&value_false, &known_val_lowered) >= 0.0,
            "<=" => known_compare(&value_false, &known_val_lowered) <= 0.0,
            _ => unreachable!(),
        };

        let unknown_inner = if let Value::Op(Op::BoolToFloat(inner)) = &unknown {
            (**inner).clone()
        } else {
            unreachable!()
        };

        let new_value = if result_true == result_false {
            Value::KnownBool(result_true)
        } else if result_true {
            unknown_inner
        } else {
            Value::BoolOp(BoolOp::Not(Box::new(unknown_inner)))
        };
        return (new_value, true);
    }

    // No structural change — rebuild if children changed.
    if did_opti_total {
        (Value::BoolOp(bop.with_values(left, right)), true)
    } else {
        (Value::BoolOp(bop.clone()), false)
    }
}

fn simplify_op(op: &Op, list_lookup: ListLookup<'_>) -> (Value, bool) {
    let (left, did_1) = partial_simplify_value(op.left(), list_lookup);
    let mut did_2 = false;
    let right = if op_right_opt(op).is_some() {
        let (r, d) = partial_simplify_value(op.right(), list_lookup);
        did_2 = d;
        r
    } else {
        op.right().clone()
    };
    let mut did_opti_total = did_1 || did_2;

    let has_right = op_right_opt(op).is_some();

    // Rule 1: left Known and (right Known or unary) — evaluate.
    if is_known(&left) && (is_known(&right) || !has_right) {
        let l = known_to_num(&left);
        let r = if has_right { known_to_num(&right) } else { 0.0 };
        did_opti_total = true;

        let result: Option<Value> = match op {
            Op::Add(_, _) => Some(Value::Known(KnownVal::Num(l + r))),
            Op::Sub(_, _) => Some(Value::Known(KnownVal::Num(l - r))),
            Op::Mul(_, _) => Some(Value::Known(KnownVal::Num(l * r))),
            Op::Div(_, _) => {
                if r != 0.0 {
                    Some(Value::Known(KnownVal::Num(l / r)))
                } else if l == 0.0 {
                    Some(Value::Known(KnownVal::Num(f64::NAN)))
                } else {
                    Some(Value::Known(KnownVal::Num(f64::INFINITY * get_sign(l) * get_sign(r))))
                }
            }
            Op::Mod(_, _) => Some(Value::Known(KnownVal::Num(scratch_mod(l, r)))),
            Op::BoolToFloat(_) | Op::StrToFloat(_) => Some(Value::Known(KnownVal::Num(l))),
            Op::Abs(_) => Some(Value::Known(KnownVal::Num(l.abs()))),
            Op::Floor(_) => Some(Value::Known(KnownVal::Num(l.floor()))),
            Op::Ceiling(_) => Some(Value::Known(KnownVal::Num(l.ceil()))),
            _ => {
                // Python `case _: did_opti_total = False`
                did_opti_total = false;
                None
            }
        };
        if let Some(v) = result {
            return (v, true);
        }
    }

    // Rule 2 (Python's second `elif`): both sides Known, both present, mul → 0.
    if has_right && is_known(&left) && is_known(&right) {
        let l = known_to_num(&left);
        let r = known_to_num(&right);
        if matches!(op, Op::Mul(_, _)) {
            if l == 0.0 || r == 0.0 {
                return (Value::Known(KnownVal::Num(0.0)), true);
            }
            // (did_opti_total = True is set unconditionally by Python here, but
            // since the previous branch already returned for mul with two knowns,
            // this case only fires when neither is 0 — in which case no value
            // change occurs. Preserve `did_opti_total = True` quirk.)
            did_opti_total = true;
        }
    }

    // Rule 3: `if not did_opti_total:` — add/sub 0 simplification + constant combine
    if !did_opti_total {
        if let Some((known, unknown, left_is_known)) = get_known_and_unknown_op(op) {
            let known_num = known_to_num(&known);

            if matches!(op, Op::Add(_, _) | Op::Sub(_, _)) {
                if known_num == 0.0 && !(matches!(op, Op::Sub(_, _)) && left_is_known) {
                    return (unknown, true);
                }
                if let Value::Op(inner_op) = &unknown {
                    if matches!(inner_op, Op::Add(..) | Op::Sub(..))
                        && get_known_and_unknown_op(inner_op).is_some()
                    {
                        let (inner_known, inner_unknown, inner_left_is_known) =
                            get_known_and_unknown_op(inner_op).unwrap();
                        let inner_known_num = known_to_num(&inner_known);

                        let outer_is_sub = matches!(op, Op::Sub(_, _));
                        let inner_is_sub = matches!(inner_op, Op::Sub(..));

                        // sub_inner_value: True if the unknown ends up on the right
                        // of a subtraction after combining.
                        let sub_inner_value =
                            (outer_is_sub && left_is_known) ^ (inner_is_sub && inner_left_is_known);

                        let combined_known =
                            known_num * (if outer_is_sub && !left_is_known { -1.0 } else { 1.0 })
                            + inner_known_num
                                * (if inner_is_sub && !inner_left_is_known { -1.0 } else { 1.0 })
                                * (if outer_is_sub && left_is_known { -1.0 } else { 1.0 });

                        let new_value = if !sub_inner_value {
                            if combined_known == 0.0 {
                                // Python quirk: `value = unknown` (the outer unknown Op),
                                // NOT `inner_unknown`. Preserved verbatim.
                                unknown.clone()
                            } else if combined_known > 0.0 {
                                Value::Op(Op::Add(
                                    Box::new(inner_unknown.clone()),
                                    Box::new(Value::Known(KnownVal::Num(combined_known))),
                                ))
                            } else {
                                Value::Op(Op::Sub(
                                    Box::new(inner_unknown.clone()),
                                    Box::new(Value::Known(KnownVal::Num(-combined_known))),
                                ))
                            }
                        } else {
                            Value::Op(Op::Sub(
                                Box::new(Value::Known(KnownVal::Num(combined_known))),
                                Box::new(inner_unknown.clone()),
                            ))
                        };

                        return (new_value, true);
                    }
                }
            }
        }
    }

    if did_opti_total {
        (Value::Op(op.with_values(left, right)), true)
    } else {
        (Value::Op(op.clone()), false)
    }
}

// ---------------------------------------------------------------------------
// simplifyValue — iterate partial_simplify_value until fixpoint.
// ---------------------------------------------------------------------------

/// Translation-time `simplifyValue` with no list lookup (matches Python default).
pub fn simplify_value(value: &Value) -> Value {
    let no_lookup = |_: &str, _: &Value| None;
    simplify_value_with(value, &no_lookup)
}

pub fn simplify_value_with(value: &Value, list_lookup: ListLookup<'_>) -> Value {
    let mut current = value.clone();
    loop {
        let (simplified, changed) = partial_simplify_value(&current, list_lookup);
        if !changed {
            return simplified;
        }
        current = simplified;
    }
}

// ---------------------------------------------------------------------------
// getInputs / setInputs — only block-level direct inputs (no nested bodies)
// ---------------------------------------------------------------------------

fn get_inputs(block: &Block) -> Vec<Value> {
    match block {
        Block::Say { value }
        | Block::EditVar(EditVarData { value, .. })
        | Block::Broadcast { value, .. }
        | Block::SwitchCostume { value }
        | Block::EditVolume { value, .. }
        | Block::Ask { value, .. }
        | Block::Wait { value } => {
            vec![value.clone()]
        }
        Block::ControlFlow(cf) => {
            if let Some(cond) = &cf.condition { vec![cond.clone()] } else { vec![] }
        }
        Block::EditList(data) => {
            let mut v = Vec::new();
            if let Some(val) = &data.value { v.push(val.clone()); }
            if let Some(idx) = &data.index { v.push(idx.clone()); }
            v
        }
        Block::ProcedureCall(data) => data.args.clone(),
        _ => Vec::new(),
    }
}

fn set_inputs(block: Block, inputs: Vec<Value>) -> Block {
    let mut blk = block;
    match &mut blk {
        Block::Say { value }
        | Block::EditVar(EditVarData { value, .. })
        | Block::Broadcast { value, .. }
        | Block::SwitchCostume { value }
        | Block::EditVolume { value, .. }
        | Block::Ask { value, .. }
        | Block::Wait { value } => {
            debug_assert_eq!(inputs.len(), 1);
            *value = inputs.into_iter().next().unwrap();
        }
        Block::ControlFlow(cf) => {
            if cf.op != ControlOp::Forever {
                debug_assert_eq!(inputs.len(), 1);
                cf.condition = Some(inputs.into_iter().next().unwrap());
            } else {
                debug_assert!(inputs.is_empty());
            }
        }
        Block::EditList(data) => {
            let mut iter = inputs.into_iter();
            if data.value.is_some() { data.value = iter.next(); }
            if data.index.is_some() { data.index = iter.next(); }
        }
        Block::ProcedureCall(data) => {
            data.args = inputs;
        }
        _ => {}
    }
    blk
}

// ---------------------------------------------------------------------------
// knownValuePropagationBlock
// ---------------------------------------------------------------------------

fn is_end_blocklist(blocks: &BlockList) -> bool {
    blocks.blocks.last().map(|b| b.is_end()).unwrap_or(false)
}

pub fn known_value_propagation_block(
    blocklist: &BlockList,
    list_lookup: ListLookup<'_>,
) -> (BlockList, bool) {
    let mut did_opti_total = false;
    let mut new_blocklist = BlockList::new();

    for block in &blocklist.blocks {
        // 1) Optimize the direct inputs of the block.
        let mut block = block.clone();
        loop {
            let mut did_opti = false;
            let inputs = get_inputs(&block);
            let mut new_inputs = Vec::with_capacity(inputs.len());
            for value in inputs {
                let (v, did_v) = partial_simplify_value(&value, list_lookup);
                new_inputs.push(v);
                did_opti |= did_v;
            }
            block = set_inputs(block, new_inputs);
            did_opti_total |= did_opti;
            if !did_opti { break; }
        }

        // 2) Recurse into sub-blocklists.
        if let Block::ControlFlow(cf) = &mut block {
            let mut did_1 = false;
            let mut did_2 = false;
            if let Some(body) = cf.body.as_ref() {
                let (nb, d) = known_value_propagation_block(body, list_lookup);
                cf.body = Some(nb);
                did_1 = d;
            }
            if let Some(else_body) = cf.else_body.as_ref() {
                let (nb, d) = known_value_propagation_block(else_body, list_lookup);
                cf.else_body = Some(nb);
                did_2 = d;
            }
            did_opti_total |= did_1 || did_2;
        }

        // 3) Block-level simplification (dead-branch elimination, not-swap).
        let mut add_block = true;
        let mut did_opti = false;

        if let Block::ControlFlow(cf) = &block {
            if let Some(cond) = &cf.condition {
                if is_known(cond) {
                    match cf.op {
                        ControlOp::If | ControlOp::IfElse => {
                            did_opti = true;
                            add_block = false;
                            let truthy = known_to_bool(cond);
                            let take_else = !truthy;
                            let take_else = take_else && cf.op == ControlOp::IfElse;
                            if truthy {
                                if let Some(body) = &cf.body {
                                    if !body.is_empty() {
                                        new_blocklist.add(body.clone());
                                    }
                                    if is_end_blocklist(body) { break; }
                                }
                            } else if take_else {
                                if let Some(else_body) = &cf.else_body {
                                    if !else_body.is_empty() {
                                        new_blocklist.add(else_body.clone());
                                    }
                                    if is_end_blocklist(else_body) { break; }
                                }
                            }
                        }
                        ControlOp::Until | ControlOp::While => {
                            did_opti = true;
                            // Python: `inverted = block == "while"` — bug, always False.
                            let inverted = false;
                            let is_forever = inverted ^ !known_to_bool(cond);
                            if is_forever {
                                let new_cf = ControlFlow {
                                    op: ControlOp::Forever,
                                    condition: None,
                                    var: cf.var.clone(),
                                    body: cf.body.clone(),
                                    else_body: cf.else_body.clone(),
                                };
                                new_blocklist.add_block(Block::ControlFlow(new_cf));
                                break;
                            } else {
                                add_block = false;
                            }
                        }
                        _ => {}
                    }
                } else if let Value::BoolOp(BoolOp::Not(inner)) = cond {
                    match cf.op {
                        ControlOp::IfElse => {
                            did_opti = true;
                            let new_cf = ControlFlow {
                                op: ControlOp::IfElse,
                                condition: Some((**inner).clone()),
                                var: cf.var.clone(),
                                body: cf.else_body.clone(),
                                else_body: cf.body.clone(),
                            };
                            block = Block::ControlFlow(new_cf);
                        }
                        ControlOp::Until | ControlOp::While => {
                            did_opti = true;
                            let new_op = if cf.op == ControlOp::While { ControlOp::Until } else { ControlOp::While };
                            let new_cf = ControlFlow {
                                op: new_op,
                                condition: Some((**inner).clone()),
                                var: cf.var.clone(),
                                body: cf.body.clone(),
                                else_body: cf.else_body.clone(),
                            };
                            block = Block::ControlFlow(new_cf);
                        }
                        _ => {}
                    }
                }
            }
        }

        did_opti_total |= did_opti;

        if add_block {
            new_blocklist.add_block(block);
        }
    }

    (new_blocklist, did_opti_total)
}

// ---------------------------------------------------------------------------
// knownValuePropagation
// ---------------------------------------------------------------------------

pub fn known_value_propagation(
    proj: &Project,
    list_lookup: Option<ListLookup<'_>>,
) -> (Project, bool) {
    let no_lookup = |_: &str, _: &Value| None;
    let lookup: ListLookup<'_> = list_lookup.unwrap_or(&no_lookup);

    let mut new_proj = proj.clone();
    let mut did_total_opti = false;

    for code in new_proj.code.iter_mut() {
        loop {
            let (new_code, did) = known_value_propagation_block(code, lookup);
            *code = new_code;
            did_total_opti |= did;
            if !did { break; }
        }
    }

    (new_proj, did_total_opti)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ScratchConfig;
    use crate::scratch::ast::{BlockList, EditListData, ListEditOp, ListOp};

    fn no_lookup() -> impl Fn(&str, &Value) -> Option<Value> {
        |_: &str, _: &Value| None
    }

    #[test]
    fn test_add_knowns() {
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(3.0))),
            Box::new(Value::Known(KnownVal::Num(4.0))),
        ));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_sub_knowns() {
        let val = Value::Op(Op::Sub(
            Box::new(Value::Known(KnownVal::Num(10.0))),
            Box::new(Value::Known(KnownVal::Num(3.0))),
        ));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_mul_knowns() {
        let val = Value::Op(Op::Mul(
            Box::new(Value::Known(KnownVal::Num(6.0))),
            Box::new(Value::Known(KnownVal::Num(7.0))),
        ));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(42.0)));
    }

    #[test]
    fn test_add_zero_left() {
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(0.0))),
            Box::new(Value::GetVar { name: "ptr".to_string() }),
        ));
        assert_eq!(simplify_value(&val), Value::GetVar { name: "ptr".to_string() });
    }

    #[test]
    fn test_zero_add_right() {
        let val = Value::Op(Op::Add(
            Box::new(Value::GetVar { name: "ptr".to_string() }),
            Box::new(Value::Known(KnownVal::Num(0.0))),
        ));
        assert_eq!(simplify_value(&val), Value::GetVar { name: "ptr".to_string() });
    }

    #[test]
    fn test_sub_zero_right() {
        // (ptr) - 0  ->  ptr  (left_is_known=False, so not blocked)
        let val = Value::Op(Op::Sub(
            Box::new(Value::GetVar { name: "ptr".to_string() }),
            Box::new(Value::Known(KnownVal::Num(0.0))),
        ));
        assert_eq!(simplify_value(&val), Value::GetVar { name: "ptr".to_string() });
    }

    #[test]
    fn test_sub_zero_left_not_simplified() {
        // 0 - ptr should NOT simplify to ptr (Python guards against this).
        let val = Value::Op(Op::Sub(
            Box::new(Value::Known(KnownVal::Num(0.0))),
            Box::new(Value::GetVar { name: "ptr".to_string() }),
        ));
        let result = simplify_value(&val);
        assert_eq!(result, val);
    }

    #[test]
    fn test_const_combine() {
        // (1) + ((ptr) + (2))  ->  (ptr) + (3)
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(1.0))),
            Box::new(Value::Op(Op::Add(
                Box::new(Value::GetVar { name: "ptr".to_string() }),
                Box::new(Value::Known(KnownVal::Num(2.0))),
            ))),
        ));
        let result = simplify_value(&val);
        assert_eq!(result, Value::Op(Op::Add(
            Box::new(Value::GetVar { name: "ptr".to_string() }),
            Box::new(Value::Known(KnownVal::Num(3.0))),
        )));
    }

    #[test]
    fn test_bool_to_float_eq_known() {
        // (bool_to_float X) = (1)  ->  X
        let val = Value::BoolOp(BoolOp::Eq(
            Box::new(Value::Op(Op::BoolToFloat(Box::new(Value::GetVar { name: "x".to_string() })))),
            Box::new(Value::Known(KnownVal::Num(1.0))),
        ));
        let result = simplify_value(&val);
        assert_eq!(result, Value::GetVar { name: "x".to_string() });
    }

    #[test]
    fn test_bool_to_float_eq_zero() {
        // (bool_to_float X) = (0)  ->  not X
        let val = Value::BoolOp(BoolOp::Eq(
            Box::new(Value::Op(Op::BoolToFloat(Box::new(Value::GetVar { name: "x".to_string() })))),
            Box::new(Value::Known(KnownVal::Num(0.0))),
        ));
        let result = simplify_value(&val);
        assert_eq!(result, Value::BoolOp(BoolOp::Not(Box::new(Value::GetVar { name: "x".to_string() }))));
    }

    #[test]
    fn test_not_knownbool() {
        let val = Value::BoolOp(BoolOp::Not(Box::new(Value::KnownBool(true))));
        assert_eq!(simplify_value(&val), Value::KnownBool(false));
    }

    #[test]
    fn test_not_not() {
        let val = Value::BoolOp(BoolOp::Not(Box::new(Value::BoolOp(BoolOp::Not(
            Box::new(Value::GetVar { name: "x".to_string() }),
        )))));
        assert_eq!(simplify_value(&val), Value::GetVar { name: "x".to_string() });
    }

    #[test]
    fn test_and_known_false() {
        let val = Value::BoolOp(BoolOp::And(
            Box::new(Value::KnownBool(false)),
            Box::new(Value::GetVar { name: "x".to_string() }),
        ));
        assert_eq!(simplify_value(&val), Value::KnownBool(false));
    }

    #[test]
    fn test_or_known_true() {
        let val = Value::BoolOp(BoolOp::Or(
            Box::new(Value::KnownBool(true)),
            Box::new(Value::GetVar { name: "x".to_string() }),
        ));
        assert_eq!(simplify_value(&val), Value::KnownBool(true));
    }

    #[test]
    fn test_comparison_add_sub() {
        // (ptr + 5) > (3)  ->  ptr > -2
        let val = Value::BoolOp(BoolOp::Gt(
            Box::new(Value::Op(Op::Add(
                Box::new(Value::GetVar { name: "ptr".to_string() }),
                Box::new(Value::Known(KnownVal::Num(5.0))),
            ))),
            Box::new(Value::Known(KnownVal::Num(3.0))),
        ));
        let result = simplify_value(&val);
        assert_eq!(result, Value::BoolOp(BoolOp::Gt(
            Box::new(Value::GetVar { name: "ptr".to_string() }),
            Box::new(Value::Known(KnownVal::Num(-2.0))),
        )));
    }

    #[test]
    fn test_abs() {
        let val = Value::Op(Op::Abs(Box::new(Value::Known(KnownVal::Num(-5.0)))));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(5.0)));
    }

    #[test]
    fn test_floor() {
        let val = Value::Op(Op::Floor(Box::new(Value::Known(KnownVal::Num(3.7)))));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(3.0)));
    }

    #[test]
    fn test_ceiling() {
        let val = Value::Op(Op::Ceiling(Box::new(Value::Known(KnownVal::Num(3.2)))));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(4.0)));
    }

    #[test]
    fn test_bool_to_float() {
        let val = Value::Op(Op::BoolToFloat(Box::new(Value::Known(KnownVal::Num(7.0)))));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_str_to_float() {
        let val = Value::Op(Op::StrToFloat(Box::new(Value::Known(KnownVal::Num(7.0)))));
        assert_eq!(simplify_value(&val), Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_sqrt_not_folded() {
        // Python doesn't fold sqrt in partialSimplifyValue (no `sqrt` case).
        let val = Value::Op(Op::Sqrt(Box::new(Value::Known(KnownVal::Num(16.0)))));
        let result = simplify_value(&val);
        assert_eq!(result, val);
    }

    #[test]
    fn test_getvar_not_simplified() {
        // GetVar should NOT be simplified (no lookup_func for GetVar).
        let val = Value::GetVar { name: "x".to_string() };
        assert_eq!(simplify_value(&val), val);
    }

    #[test]
    fn test_dead_branch_if_true() {
        let mut bl = BlockList::new();
        bl.add_block(Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::KnownBool(true)),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) })),
            else_body: None,
        }));
        bl.add_block(Block::Say { value: Value::Known(KnownVal::Num(2.0)) });

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        assert_eq!(result.blocks.len(), 2);
        assert!(matches!(result.blocks[0], Block::Say { .. }));
    }

    #[test]
    fn test_dead_branch_if_false() {
        let mut bl = BlockList::new();
        bl.add_block(Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::KnownBool(false)),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) })),
            else_body: None,
        }));
        bl.add_block(Block::Say { value: Value::Known(KnownVal::Num(2.0)) });

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        assert_eq!(result.blocks.len(), 1);
    }

    #[test]
    fn test_not_swap_if_else() {
        let mut bl = BlockList::new();
        bl.add_block(Block::ControlFlow(ControlFlow {
            op: ControlOp::IfElse,
            condition: Some(Value::BoolOp(BoolOp::Not(Box::new(Value::GetVar { name: "x".to_string() })))),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) })),
            else_body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(2.0)) })),
        }));

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        if let Block::ControlFlow(cf) = &result.blocks[0] {
            assert_eq!(cf.condition, Some(Value::GetVar { name: "x".to_string() }));
            // body should now be the old else_body (2.0)
            if let Some(body) = &cf.body {
                assert!(matches!(&body.blocks[0], Block::Say { value } if *value == Value::Known(KnownVal::Num(2.0))));
            } else {
                panic!("body missing");
            }
        } else {
            panic!("expected ControlFlow");
        }
    }

    #[test]
    fn test_dead_branch_until_known_true() {
        // repeat until (true) -> nothing (loop body never runs)
        let mut bl = BlockList::new();
        bl.add_block(Block::ControlFlow(ControlFlow {
            op: ControlOp::Until,
            condition: Some(Value::KnownBool(true)),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) })),
            else_body: None,
        }));
        bl.add_block(Block::Say { value: Value::Known(KnownVal::Num(2.0)) });

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        // The until block should be removed; only the trailing Say remains.
        assert_eq!(result.blocks.len(), 1);
        assert!(matches!(result.blocks[0], Block::Say { .. }));
    }

    #[test]
    fn test_dead_branch_until_known_false() {
        // repeat until (false) -> forever
        let mut bl = BlockList::new();
        bl.add_block(Block::ControlFlow(ControlFlow {
            op: ControlOp::Until,
            condition: Some(Value::KnownBool(false)),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) })),
            else_body: None,
        }));

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        if let Block::ControlFlow(cf) = &result.blocks[0] {
            assert_eq!(cf.op, ControlOp::Forever);
        } else {
            panic!("expected ControlFlow");
        }
    }

    #[test]
    fn test_edit_var_simplification() {
        let mut bl = BlockList::new();
        bl.add_block(Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "x".to_string(),
            value: Value::Op(Op::Add(
                Box::new(Value::Known(KnownVal::Num(1.0))),
                Box::new(Value::Known(KnownVal::Num(2.0))),
            )),
        }));

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        if let Block::EditVar(data) = &result.blocks[0] {
            assert_eq!(data.value, Value::Known(KnownVal::Num(3.0)));
        } else {
            panic!("expected EditVar");
        }
    }

    #[test]
    fn test_getoflist_with_lookup() {
        let gol = Value::GetOfList(GetOfList {
            op: ListOp::AtIndex,
            name: "mylist".to_string(),
            value: Box::new(Value::Known(KnownVal::Num(5.0))),
        });

        let lookup = |name: &str, idx: &Value| -> Option<Value> {
            if name == "mylist" {
                if let Value::Known(KnownVal::Num(n)) = idx {
                    return Some(Value::Known(KnownVal::Num(n + 100.0)));
                }
            }
            None
        };
        let result = simplify_value_with(&gol, &lookup);
        assert_eq!(result, Value::Known(KnownVal::Num(105.0)));
    }

    #[test]
    fn test_getoflist_no_lookup() {
        let gol = Value::GetOfList(GetOfList {
            op: ListOp::AtIndex,
            name: "mylist".to_string(),
            value: Box::new(Value::Known(KnownVal::Num(5.0))),
        });
        let result = simplify_value(&gol);
        assert_eq!(result, gol);
    }

    #[test]
    fn test_known_value_propagation_project_no_change() {
        let mut proj = Project::new(ScratchConfig::default());
        let mut bl = BlockList::new();
        bl.add_block(Block::Say { value: Value::Known(KnownVal::Num(42.0)) });
        proj.code.push(bl);

        let (_, changed) = known_value_propagation(&proj, None);
        assert!(!changed);
    }

    #[test]
    fn test_edit_list_input_simplification() {
        let mut bl = BlockList::new();
        bl.add_block(Block::EditList(EditListData {
            op: ListEditOp::ReplaceAt,
            name: "lst".to_string(),
            value: Some(Value::Op(Op::Add(
                Box::new(Value::Known(KnownVal::Num(1.0))),
                Box::new(Value::Known(KnownVal::Num(2.0))),
            ))),
            index: Some(Value::Known(KnownVal::Num(0.0))),
        }));

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        if let Block::EditList(data) = &result.blocks[0] {
            assert_eq!(data.value, Some(Value::Known(KnownVal::Num(3.0))));
        } else {
            panic!("expected EditList");
        }
    }

    #[test]
    fn test_nested_controlflow_recursion() {
        // if (true) { if (1+2 == 3) { say "yes" } }
        let inner_cf = Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::BoolOp(BoolOp::Eq(
                Box::new(Value::Op(Op::Add(
                    Box::new(Value::Known(KnownVal::Num(1.0))),
                    Box::new(Value::Known(KnownVal::Num(2.0))),
                ))),
                Box::new(Value::Known(KnownVal::Num(3.0))),
            ))),
            var: None,
            body: Some(BlockList::from_block(Block::Say { value: Value::Known(KnownVal::Str("yes".to_string())) })),
            else_body: None,
        });
        let outer_cf = Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::KnownBool(true)),
            var: None,
            body: Some(BlockList::from_block(inner_cf)),
            else_body: None,
        });
        let mut bl = BlockList::new();
        bl.add_block(outer_cf);

        let (result, changed) = known_value_propagation_block(&bl, &no_lookup());
        assert!(changed);
        // Both the outer if (cond true) and inner if (1+2==3 → true) collapse,
        // leaving just the Say block.
        assert_eq!(result.blocks.len(), 1);
        assert!(matches!(result.blocks[0], Block::Say { .. }));
    }
}
