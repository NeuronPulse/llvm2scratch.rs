use crate::scratch::{GetOfList, KnownVal, ListOp, Op, Value};

use super::config::{CompException, VARIABLE_MAX_BITS};

pub fn undo_twos_complement_with_offset(val: Value, width: usize) -> (Value, i64) {
    let half_pow = 2f64.powi(width as i32 - 1) + 1.0;
    let neg_pow = -2f64.powi(width as i32);
    let offset: i64 = if width <= 63 {
        (1i64 << (width - 1)) - 1
    } else {
        (1i64 << 62) - 1 + (1i64 << 62)
    };

    let inner = Value::Op(Op::Add(
        Box::new(val),
        Box::new(Value::Known(KnownVal::Num(half_pow))),
    ));
    let modded = Value::Op(Op::Mod(
        Box::new(inner),
        Box::new(Value::Known(KnownVal::Num(neg_pow))),
    ));

    (modded, offset)
}

pub fn undo_twos_complement(val: Value, width: usize) -> Value {
    let (value, offset) = undo_twos_complement_with_offset(val, width);
    Value::Op(Op::Add(
        Box::new(value),
        Box::new(Value::Known(KnownVal::Num(offset as f64))),
    ))
}

/// Reverse two's complement for signed comparisons.
/// Matches Python's `reverse_twos_complement`: `mod(val + 2^(width-1) + 1, -2^width)`.
pub fn reverse_twos_complement(val: Value, width: usize) -> Value {
    let half_pow = 2f64.powi(width as i32 - 1) + 1.0;
    let neg_pow = -2f64.powi(width as i32);
    Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Add(
            Box::new(val),
            Box::new(Value::Known(KnownVal::Num(half_pow))),
        ))),
        Box::new(Value::Known(KnownVal::Num(neg_pow))),
    ))
}

/// Variant used by Python's compiler_opt signed >=/<= shortcut.
/// Matches Python's `reverse_twos_complement_and_sub_half`:
/// `mod(val + 2^(width-1) + 0.5, -2^width)`.
pub fn reverse_twos_complement_and_sub_half(val: Value, width: usize) -> Value {
    let half_pow = 2f64.powi(width as i32 - 1) + 0.5;
    let neg_pow = -2f64.powi(width as i32);
    Value::Op(Op::Mod(
        Box::new(Value::Op(Op::Add(
            Box::new(val),
            Box::new(Value::Known(KnownVal::Num(half_pow))),
        ))),
        Box::new(Value::Known(KnownVal::Num(neg_pow))),
    ))
}

pub fn comptime_undo_twos_complement(val: f64, width: usize) -> f64 {
    if width >= 128 {
        return val;
    }
    let val_i128 = val as i128;
    let threshold: i128 = 1i128 << width;
    if val_i128 >= threshold / 2 {
        (val_i128 - threshold) as f64
    } else {
        val
    }
}

pub fn apply_twos_complement(val: Value, width: usize) -> Value {
    let pow2w = 2f64.powi(width as i32);
    Value::Op(Op::Mod(
        Box::new(val),
        Box::new(Value::Known(KnownVal::Num(pow2w))),
    ))
}

pub fn comptime_apply_twos_complement(val: f64, width: usize) -> f64 {
    let pow2w_i128 = 1i128 << width;
    let val_i128 = val as i128;
    let result = ((val_i128 % pow2w_i128) + pow2w_i128) % pow2w_i128;
    result as f64
}

pub fn int_pow2(val: &Value, manual_offset: i32) -> Result<Value, CompException> {
    match val {
        Value::Known(kv) => match kv {
            KnownVal::Num(n) => {
                let exp = *n as i32 + manual_offset;
                if !(0..=53).contains(&exp) {
                    Err(CompException("Cannot calculate pow2 of a known non-integer".to_string()))
                } else {
                    Ok(Value::Known(KnownVal::Num(2f64.powi(exp))))
                }
            }
            _ => Err(CompException("Cannot calculate pow2 of a known non-integer".to_string())),
        },
        _ => {
            // Matches Python's getPow2Offset(): offset = VARIABLE_MAX_BITS + 1
            // because Scratch lists are 1-indexed.
            let offset = VARIABLE_MAX_BITS as f64 + 1.0;
            let idx = Value::Op(Op::Add(
                Box::new(val.clone()),
                Box::new(Value::Known(KnownVal::Num(offset + manual_offset as f64))),
            ));
            Ok(Value::GetOfList(GetOfList {
                op: ListOp::AtIndex,
                name: "!POW2 lookup".to_string(),
                value: Box::new(idx),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comptime_undo_twos_complement_positive() {
        assert_eq!(comptime_undo_twos_complement(5.0, 8), 5.0);
    }

    #[test]
    fn test_comptime_undo_twos_complement_negative() {
        assert_eq!(comptime_undo_twos_complement(255.0, 8), -1.0);
        assert_eq!(comptime_undo_twos_complement(128.0, 8), -128.0);
    }

    #[test]
    fn test_comptime_undo_twos_complement_32bit() {
        assert_eq!(comptime_undo_twos_complement(4294967295.0, 32), -1.0);
        assert_eq!(comptime_undo_twos_complement(2147483648.0, 32), -2147483648.0);
    }

    #[test]
    fn test_comptime_apply_twos_complement() {
        assert_eq!(comptime_apply_twos_complement(-1.0, 8), 255.0);
        assert_eq!(comptime_apply_twos_complement(5.0, 8), 5.0);
    }

    #[test]
    fn test_undo_twos_complement_known() {
        let val = Value::Known(KnownVal::Num(255.0));
        let result = undo_twos_complement(val, 8);
        assert!(matches!(result, Value::Op(..)));
    }

    #[test]
    fn test_undo_twos_complement_with_offset_structure() {
        let val = Value::Known(KnownVal::Num(128.0));
        let (value, offset) = undo_twos_complement_with_offset(val, 8);
        assert!(matches!(value, Value::Op(..)));
        assert_eq!(offset, 127);
    }

    #[test]
    fn test_apply_twos_complement_structure() {
        let val = Value::Known(KnownVal::Num(-1.0));
        let result = apply_twos_complement(val, 8);
        assert!(matches!(result, Value::Op(..)));
    }

    #[test]
    fn test_int_pow2_known() {
        let val = Value::Known(KnownVal::Num(3.0));
        let result = int_pow2(&val, 0).unwrap();
        assert_eq!(result, Value::Known(KnownVal::Num(8.0)));
    }

    #[test]
    fn test_int_pow2_known_with_offset() {
        let val = Value::Known(KnownVal::Num(3.0));
        let result = int_pow2(&val, -1).unwrap();
        assert_eq!(result, Value::Known(KnownVal::Num(4.0)));
    }

    #[test]
    fn test_int_pow2_unknown() {
        let val = Value::GetVar { name: "x".to_string() };
        let result = int_pow2(&val, 0);
        assert!(result.is_ok());
    }

    #[test]
    fn test_comptime_undo_twos_complement_64bit() {
        assert_eq!(comptime_undo_twos_complement(0.0, 64), 0.0);
        assert_eq!(comptime_undo_twos_complement(1.0, 64), 1.0);
        assert_eq!(comptime_undo_twos_complement(100.0, 64), 100.0);
    }

    #[test]
    fn test_comptime_apply_twos_complement_64bit() {
        assert_eq!(comptime_apply_twos_complement(5.0, 64), 5.0);
        assert_eq!(comptime_apply_twos_complement(-1.0, 32), 4294967295.0);
    }

    #[test]
    fn test_undo_twos_complement_with_offset_64bit() {
        let val = Value::Known(KnownVal::Num(128.0));
        let (value, offset) = undo_twos_complement_with_offset(val, 64);
        assert!(matches!(value, Value::Op(..)));
        assert_eq!(offset, (1i64 << 62) - 1 + (1i64 << 62));
    }
}