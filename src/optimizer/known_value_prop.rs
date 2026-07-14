use std::collections::HashMap;

use crate::scratch::{Block, BlockList, Project, Value};
use crate::scratch::ast::{BoolOp, KnownVal, Op};

pub fn partial_simplify_value(value: &Value, lookup_func: &dyn Fn(&Value) -> Option<Value>) -> (Value, bool) {
    match value {
        Value::Known(_) | Value::KnownBool(_) => (value.clone(), false),
        Value::Op(op) => simplify_op(op, lookup_func),
        Value::BoolOp(bop) => simplify_bool_op(bop, lookup_func),
        Value::GetVar { name: _ } => {
            if let Some(simplified) = lookup_func(value) {
                (simplified, true)
            } else {
                (value.clone(), false)
            }
        }
        Value::GetOfList(gol) => {
            let (new_val, changed) = partial_simplify_value(&gol.value, lookup_func);
            if changed {
                (Value::GetOfList(crate::scratch::ast::GetOfList {
                    op: gol.op,
                    name: gol.name.clone(),
                    value: Box::new(new_val),
                }), true)
            } else {
                (value.clone(), false)
            }
        }
        _ => (value.clone(), false),
    }
}

fn simplify_op(op: &Op, lookup_func: &dyn Fn(&Value) -> Option<Value>) -> (Value, bool) {
    let (left, left_changed) = partial_simplify_value(op.left(), lookup_func);
    let (right, right_changed) = partial_simplify_value(op.right(), lookup_func);

    let changed = left_changed || right_changed;

    if let Value::Known(KnownVal::Num(a)) = &left {
        let unary_result: Option<Value> = match op {
            Op::Abs(_) => Some(Value::Known(KnownVal::Num(a.abs()))),
            Op::Floor(_) => Some(Value::Known(KnownVal::Num(a.floor()))),
            Op::Ceiling(_) => Some(Value::Known(KnownVal::Num(a.ceil()))),
            Op::Sqrt(_) => {
                if *a >= 0.0 { Some(Value::Known(KnownVal::Num(a.sqrt()))) } else { None }
            }
            Op::Sin(_) => Some(Value::Known(KnownVal::Num(a.to_radians().sin()))),
            Op::Cos(_) => Some(Value::Known(KnownVal::Num(a.to_radians().cos()))),
            Op::Tan(_) => Some(Value::Known(KnownVal::Num(a.to_radians().tan()))),
            Op::Asin(_) => {
                if *a >= -1.0 && *a <= 1.0 { Some(Value::Known(KnownVal::Num(a.asin().to_degrees()))) } else { None }
            }
            Op::Acos(_) => {
                if *a >= -1.0 && *a <= 1.0 { Some(Value::Known(KnownVal::Num(a.acos().to_degrees()))) } else { None }
            }
            Op::Atan(_) => Some(Value::Known(KnownVal::Num(a.atan().to_degrees()))),
            Op::Ln(_) => {
                if *a > 0.0 { Some(Value::Known(KnownVal::Num(a.ln()))) } else { None }
            }
            Op::Log(_) => {
                if *a > 0.0 { Some(Value::Known(KnownVal::Num(a.log10()))) } else { None }
            }
            Op::Exp(_) => Some(Value::Known(KnownVal::Num(a.exp()))),
            Op::Exp10(_) => Some(Value::Known(KnownVal::Num(10f64.powf(*a)))),
            _ => None,
        };
        if let Some(result) = unary_result {
            return (result, true);
        }
    }

    if let (Value::Known(KnownVal::Num(a)), Value::Known(KnownVal::Num(b))) = (&left, &right) {
        let result = match op {
            Op::Add(_, _) => Value::Known(KnownVal::Num(a + b)),
            Op::Sub(_, _) => Value::Known(KnownVal::Num(a - b)),
            Op::Mul(_, _) => Value::Known(KnownVal::Num(a * b)),
            Op::Div(_, _) => {
                if *b != 0.0 {
                    Value::Known(KnownVal::Num(a / b))
                } else {
                    return (Value::Op(op.with_values(left, right)), changed);
                }
            }
            Op::Mod(_, _) => {
                if *b != 0.0 {
                    // Scratch's modulo uses floor division, matching Python's % operator,
                    // not Rust's truncated-division % operator.
                    let div = (a / b).floor();
                    let rem = a - b * div;
                    Value::Known(KnownVal::Num(rem))
                } else {
                    return (Value::Op(op.with_values(left, right)), changed);
                }
            }
            _ => return (Value::Op(op.with_values(left, right)), changed),
        };
        (result, true)
    } else if changed {
        (Value::Op(op.with_values(left, right)), true)
    } else {
        (Value::Op(op.clone()), false)
    }
}

fn simplify_bool_op(bop: &BoolOp, lookup_func: &dyn Fn(&Value) -> Option<Value>) -> (Value, bool) {
    let (left, left_changed) = partial_simplify_value(bop.left(), lookup_func);
    let (right, right_changed) = partial_simplify_value(bop.right(), lookup_func);

    let changed = left_changed || right_changed;

    if changed {
        (Value::BoolOp(bop.with_values(left, right)), true)
    } else {
        (Value::BoolOp(bop.clone()), false)
    }
}

pub fn simplify_value(value: &Value, lookup_func: Option<&dyn Fn(&Value) -> Option<Value>>) -> Value {
    let default_lookup = |_: &Value| None;
    let lookup = lookup_func.unwrap_or(&default_lookup);

    let mut current = value.clone();
    loop {
        let (simplified, changed) = partial_simplify_value(&current, lookup);
        if !changed {
            return simplified;
        }
        current = simplified;
    }
}

pub fn known_value_propagation_block(
    blocklist: &BlockList,
    lookup_func: &dyn Fn(&Value) -> Option<Value>,
) -> (BlockList, bool) {
    let mut known_values: HashMap<String, Value> = HashMap::new();
    let mut new_blocks = Vec::new();
    let mut any_changed = false;

    for block in &blocklist.blocks {
        let kv_snapshot = known_values.clone();
        let combined_lookup = |v: &Value| -> Option<Value> {
            if let Value::GetVar { name } = v
                && let Some(known) = kv_snapshot.get(name) {
                    return Some(known.clone());
                }
            lookup_func(v)
        };

        let (new_block, block_changed) = propagate_block(block, &combined_lookup);
        if block_changed {
            any_changed = true;
        }

        if let Block::EditVar(data) = &new_block {
            if data.op == crate::scratch::ast::VarOp::Set {
                if let Value::Known(_) | Value::KnownBool(_) = &data.value {
                    known_values.insert(data.name.clone(), data.value.clone());
                } else {
                    known_values.remove(&data.name);
                }
            } else {
                known_values.remove(&data.name);
            }
        }

        new_blocks.push(new_block);
    }

    let mut result = BlockList::new();
    for b in new_blocks {
        result.add_block(b);
    }

    (result, any_changed)
}

fn propagate_block(block: &Block, lookup_func: &dyn Fn(&Value) -> Option<Value>) -> (Block, bool) {
    match block {
        Block::EditVar(data) => {
            let (new_value, changed) = partial_simplify_value(&data.value, lookup_func);
            if changed {
                (Block::EditVar(crate::scratch::ast::EditVarData {
                    op: data.op,
                    name: data.name.clone(),
                    value: new_value,
                }), true)
            } else {
                (block.clone(), false)
            }
        }
        Block::Say { value } => {
            let (new_value, changed) = partial_simplify_value(value, lookup_func);
            if changed {
                (Block::Say { value: new_value }, true)
            } else {
                (block.clone(), false)
            }
        }
        _ => (block.clone(), false),
    }
}

pub fn known_value_propagation(
    proj: &Project,
    lookup_func: Option<&dyn Fn(&Value) -> Option<Value>>,
) -> (Project, bool) {
    let default_lookup = |_: &Value| None;
    let lookup = lookup_func.unwrap_or(&default_lookup);

    let mut new_proj = proj.clone();
    let mut any_changed = false;

    for code in &mut new_proj.code {
        let (new_code, changed) = known_value_propagation_block(code, lookup);
        if changed {
            any_changed = true;
            *code = new_code;
        }
    }

    (new_proj, any_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ast::VarOp;

    #[test]
    fn test_simplify_value_known() {
        let val = Value::Known(KnownVal::Num(42.0));
        let result = simplify_value(&val, None);
        assert_eq!(result, val);
    }

    #[test]
    fn test_simplify_value_add_knowns() {
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(3.0))),
            Box::new(Value::Known(KnownVal::Num(4.0))),
        ));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_simplify_value_sub_knowns() {
        let val = Value::Op(Op::Sub(
            Box::new(Value::Known(KnownVal::Num(10.0))),
            Box::new(Value::Known(KnownVal::Num(3.0))),
        ));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(7.0)));
    }

    #[test]
    fn test_simplify_value_mul_knowns() {
        let val = Value::Op(Op::Mul(
            Box::new(Value::Known(KnownVal::Num(6.0))),
            Box::new(Value::Known(KnownVal::Num(7.0))),
        ));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(42.0)));
    }

    #[test]
    fn test_simplify_value_with_lookup() {
        let val = Value::GetVar { name: "x".to_string() };
        let lookup = |v: &Value| -> Option<Value> {
            if let Value::GetVar { name } = v {
                if name == "x" {
                    return Some(Value::Known(KnownVal::Num(5.0)));
                }
            }
            None
        };
        let result = simplify_value(&val, Some(&lookup));
        assert_eq!(result, Value::Known(KnownVal::Num(5.0)));
    }

    #[test]
    fn test_known_value_propagation_block() {
        let mut bl = BlockList::new();
        bl.add_block(Block::EditVar(crate::scratch::ast::EditVarData {
            op: VarOp::Set,
            name: "x".to_string(),
            value: Value::Known(KnownVal::Num(5.0)),
        }));
        bl.add_block(Block::Say {
            value: Value::GetVar { name: "x".to_string() },
        });

        let no_lookup = |_: &Value| None;
        let (result, changed) = known_value_propagation_block(&bl, &no_lookup);
        assert!(changed);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_known_value_propagation_no_change() {
        let mut bl = BlockList::new();
        bl.add_block(Block::Say {
            value: Value::Known(KnownVal::Num(42.0)),
        });

        let no_lookup = |_: &Value| None;
        let (_, changed) = known_value_propagation_block(&bl, &no_lookup);
        assert!(!changed);
    }

    #[test]
    fn test_simplify_value_abs() {
        let val = Value::Op(Op::Abs(Box::new(Value::Known(KnownVal::Num(-5.0)))));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(5.0)));
    }

    #[test]
    fn test_simplify_value_sqrt() {
        let val = Value::Op(Op::Sqrt(Box::new(Value::Known(KnownVal::Num(16.0)))));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(4.0)));
    }

    #[test]
    fn test_simplify_value_floor() {
        let val = Value::Op(Op::Floor(Box::new(Value::Known(KnownVal::Num(3.7)))));
        let result = simplify_value(&val, None);
        assert_eq!(result, Value::Known(KnownVal::Num(3.0)));
    }

    #[test]
    fn test_simplify_value_exp() {
        let val = Value::Op(Op::Exp(Box::new(Value::Known(KnownVal::Num(0.0)))));
        let result = simplify_value(&val, None);
        if let Value::Known(KnownVal::Num(n)) = result {
            assert!((n - 1.0).abs() < 1e-10);
        } else {
            panic!("Expected Known(Num)");
        }
    }
}