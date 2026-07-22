use crate::scratch::{self, Block, BlockList, Value, VarOp};
use crate::scratch::ast::{BoolOp, Op};
use crate::target::TargetPerf;

use super::config::{CompException, IdxbleValue, VarType, Variable};

impl Variable {
    pub fn get_value(&self, index: Option<usize>) -> Value {
        let name = self.get_raw_var_name(index);
        match self.var_type {
            VarType::Param => scratch::Value::GetParam { name },
            _ => scratch::Value::GetVar { name },
        }
    }

    pub fn get_all_values(&self, value_len: usize) -> IdxbleValue {
        assert!(value_len > 1, "get_all_values: value_len must be > 1, got {}", value_len);
        let vals: Vec<Value> = (0..value_len).map(|i| self.get_value(Some(i))).collect();
        IdxbleValue { vals }
    }

    pub fn set_value(&self, value: Value, op: VarOp, index: Option<usize>) -> Result<Block, CompException> {
        if self.var_type == VarType::Param {
            return Err(CompException(format!("{} param is read only", self.var_name)));
        }
        Ok(Block::EditVar(scratch::ast::EditVarData {
            op,
            name: self.get_raw_var_name(index),
            value,
        }))
    }

    pub fn set_all_values(&self, values: &IdxbleValue) -> Result<BlockList, CompException> {
        if values.vals.len() <= 1 {
            return Err(CompException(format!(
                "set_all_values: expected at least 2 values, got {}", values.vals.len()
            )));
        }
        let mut blocks = BlockList::new();
        for (i, val) in values.vals.iter().enumerate() {
            blocks.add_block(self.set_value(val.clone(), VarOp::Set, Some(i))?);
        }
        Ok(blocks)
    }

    pub fn set_inferred_value(&self, value: InferredValue) -> Result<BlockList, CompException> {
        match value {
            InferredValue::Single(v) => {
                let block = self.set_value(v, VarOp::Set, None)?;
                Ok(BlockList::from_blocks(vec![block]))
            }
            InferredValue::Indexed(iv) => self.set_all_values(&iv),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InferredValue {
    Single(Value),
    Indexed(IdxbleValue),
}

impl InferredValue {
    pub fn into_single(self) -> Result<Value, super::config::CompException> {
        match self {
            InferredValue::Single(v) => Ok(v),
                InferredValue::Indexed(_) => {
                Err(super::config::CompException("Expected single value but got indexed value".to_string()))
            }
        }
    }

    pub fn into_indexed(self) -> Result<IdxbleValue, super::config::CompException> {
        match self {
            InferredValue::Indexed(iv) => Ok(iv),
            InferredValue::Single(_) => Err(super::config::CompException(
                "Expected indexed value but got single value".to_string(),
            )),
        }
    }

    pub fn as_single(&self) -> Option<&Value> {
        match self {
            InferredValue::Single(v) => Some(v),
            InferredValue::Indexed(_) => None,
        }
    }

    pub fn is_single(&self) -> bool {
        matches!(self, InferredValue::Single(_))
    }
}

impl IdxbleValue {
    pub fn stringify(&self, sb: bool) -> String {
        let items: Vec<String> = self.vals.iter().map(|v| value_stringify(v, sb)).collect();
        format!("{:?}", items)
    }
}

pub fn sum_value_parts(parts: &[Value], default: Option<f64>) -> Value {
    if parts.is_empty() {
        default.map_or_else(
            || panic!("sum_value_parts: empty parts with no default"),
            |d| Value::Known(scratch::ast::KnownVal::Num(d)),
        )
    } else {
        let mut res = parts[0].clone();
        for part in &parts[1..] {
            res = Value::Op(scratch::ast::Op::Add(Box::new(res), Box::new(part.clone())));
        }
        res
    }
}

pub fn get_value_cost(value: &Value, perf: &TargetPerf) -> f64 {
    match value {
        Value::Known(_) | Value::KnownBool(_) => 0.0,
        Value::GetVar { .. } => perf.get_var,
        Value::GetParam { .. } => 0.0,
        Value::Op(op) => {
            let op_cost = match op {
                Op::Add(..) => perf.add,
                Op::Sub(..) => perf.sub,
                Op::Mul(..) => perf.mul,
                Op::Div(..) => perf.div,
                Op::Mod(..) => perf.r#mod,
                _ => 1.0,
            };
            op_cost + get_value_cost(op.left(), perf) + get_value_cost(op.right(), perf)
        }
        Value::BoolOp(bop) => {
            let op_cost = match bop {
                BoolOp::Gt(..) => perf.gt,
                BoolOp::Lt(..) => perf.lt,
                BoolOp::Eq(..) => perf.eq,
                BoolOp::And(..) => perf.and_,
                BoolOp::Or(..) => perf.or_,
                BoolOp::Not(..) => perf.not_,
            };
            op_cost + get_value_cost(bop.left(), perf) + get_value_cost(bop.right(), perf)
        }
        Value::GetOfList(gol) => {
            let op_cost = match gol.op {
                crate::scratch::ast::ListOp::AtIndex | crate::scratch::ast::ListOp::LengthOf => perf.at_index,
                crate::scratch::ast::ListOp::IndexOf => perf.index_of,
                crate::scratch::ast::ListOp::Contains => perf.contains_str,
            };
            op_cost + get_value_cost(&gol.value, perf)
        }
        Value::GetList { .. } => perf.get_list,
        Value::GetListLength { .. } => perf.length_of_list,
        Value::CostumeInfo { .. } | Value::GetCounter | Value::GetAnswer | Value::DaysSince2000 => 0.0,
    }
}

pub fn combine_idxble_values(vals: &[InferredValue]) -> IdxbleValue {
    let mut combined: Vec<Value> = Vec::new();
    for val in vals {
        match val {
            InferredValue::Single(v) => combined.push(v.clone()),
            InferredValue::Indexed(iv) => combined.extend(iv.vals.iter().cloned()),
        }
    }
    IdxbleValue { vals: combined }
}

pub fn value_stringify(value: &Value, _sb: bool) -> String {
    match value {
        Value::Known(kv) => match kv {
            scratch::ast::KnownVal::Str(s) => format!("\"{}\"", s),
            scratch::ast::KnownVal::Num(n) => n.to_string(),
            scratch::ast::KnownVal::Bool(b) => b.to_string(),
        },
        Value::KnownBool(b) => b.to_string(),
        Value::GetVar { name } => format!("var:{}", name),
        Value::GetList { name } => format!("list:{}", name),
        Value::GetParam { name } => format!("param:{}", name),
        _ => format!("{:?}", value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ast::KnownVal;
    use crate::scratch::Op;

    #[test]
    fn test_variable_get_value_global() {
        let var = Variable { var_name: "g".to_string(), var_type: VarType::Global, fn_name: None };
        let val = var.get_value(None);
        assert!(matches!(val, Value::GetVar { .. }));
    }

    #[test]
    fn test_variable_get_value_param() {
        let var = Variable { var_name: "p".to_string(), var_type: VarType::Param, fn_name: None };
        let val = var.get_value(None);
        assert!(matches!(val, Value::GetParam { .. }));
    }

    #[test]
    fn test_variable_get_all_values() {
        let var = Variable { var_name: "x".to_string(), var_type: VarType::Var, fn_name: Some("f".to_string()) };
        let iv = var.get_all_values(3);
        assert_eq!(iv.vals.len(), 3);
    }

    #[test]
    fn test_variable_set_value() {
        let var = Variable { var_name: "x".to_string(), var_type: VarType::Var, fn_name: Some("f".to_string()) };
        let result = var.set_value(Value::Known(KnownVal::Num(42.0)), VarOp::Set, None);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert!(matches!(block, Block::EditVar(..)));
    }

    #[test]
    fn test_variable_set_value_param_readonly() {
        let var = Variable { var_name: "p".to_string(), var_type: VarType::Param, fn_name: None };
        let result = var.set_value(Value::Known(KnownVal::Num(1.0)), VarOp::Set, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_variable_set_all_values() {
        let var = Variable { var_name: "x".to_string(), var_type: VarType::Var, fn_name: Some("f".to_string()) };
        let iv = IdxbleValue {
            vals: vec![Value::Known(KnownVal::Num(1.0)), Value::Known(KnownVal::Num(2.0))],
        };
        let result = var.set_all_values(&iv);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().blocks.len(), 2);
    }

    #[test]
    fn test_idxble_value_stringify() {
        let iv = IdxbleValue {
            vals: vec![Value::Known(KnownVal::Num(1.0)), Value::Known(KnownVal::Num(2.0))],
        };
        let s = iv.stringify(false);
        assert!(!s.is_empty());
    }

    #[test]
    fn test_sum_value_parts_empty_with_default() {
        let result = sum_value_parts(&[], Some(0.0));
        assert_eq!(result, Value::Known(KnownVal::Num(0.0)));
    }

    #[test]
    fn test_sum_value_parts_single() {
        let parts = vec![Value::Known(KnownVal::Num(5.0))];
        let result = sum_value_parts(&parts, None);
        assert_eq!(result, Value::Known(KnownVal::Num(5.0)));
    }

    #[test]
    fn test_sum_value_parts_multiple() {
        let parts = vec![
            Value::Known(KnownVal::Num(3.0)),
            Value::Known(KnownVal::Num(4.0)),
        ];
        let result = sum_value_parts(&parts, None);
        assert!(matches!(result, Value::Op(Op::Add(_, _))));
    }

    #[test]
    fn test_combine_idxble_values() {
        let vals = vec![
            InferredValue::Single(Value::Known(KnownVal::Num(1.0))),
            InferredValue::Indexed(IdxbleValue {
                vals: vec![Value::Known(KnownVal::Num(2.0)), Value::Known(KnownVal::Num(3.0))],
            }),
        ];
        let result = combine_idxble_values(&vals);
        assert_eq!(result.vals.len(), 3);
    }
}