use crate::ir;
use crate::scratch::{KnownVal, Op, Value};

use super::config::{CompException, IdxbleValue, VARIABLE_MAX_BITS, PTR_WIDTH_BITS};

pub fn get_size_of(ty: &ir::Type, include_padding: bool) -> Result<usize, CompException> {
    match ty {
        ir::Type::Integer(int_ty) => {
            if include_padding {
                Ok((int_ty.width as usize).div_ceil(8))
            } else {
                Ok((int_ty.width as f64 / VARIABLE_MAX_BITS as f64).ceil() as usize)
            }
        }
        ir::Type::Half => {
            if include_padding { Ok(2) } else { Ok(1) }
        }
        ir::Type::Float => {
            if include_padding { Ok(4) } else { Ok(1) }
        }
        ir::Type::Double => {
            if include_padding { Ok(8) } else { Ok(1) }
        }
        ir::Type::Fp128 => {
            if include_padding { Ok(16) } else { Ok(1) }
        }
        ir::Type::Array(arr_ty) => {
            let inner_size = get_size_of(&arr_ty.inner, include_padding)?;
            Ok(arr_ty.size as usize * inner_size)
        }
        ir::Type::Struct(struct_ty) => {
            let mut total = 0;
            for member in &struct_ty.members {
                total += get_size_of(member, include_padding)?;
            }
            Ok(total)
        }
        ir::Type::Pointer(_) => {
            if include_padding {
                Ok(PTR_WIDTH_BITS.div_ceil(8))
            } else {
                Ok(1)
            }
        }
        ir::Type::Vector(vec_ty) => {
            let elem_size = get_size_of(&vec_ty.inner, include_padding)?;
            Ok(vec_ty.size as usize * elem_size)
        }
        _ => Err(CompException(format!("Cannot get size of type: {:?}", ty))),
    }
}

pub struct GepResult {
    pub known_offset: i64,
    pub unknown_offsets: Vec<GepUnknownOffset>,
    pub result_type: ir::Type,
    pub i8_gep_div: usize,
}

pub struct GepUnknownOffset {
    pub index_val: Value,
    pub index_width: usize,
    pub multiplier: usize,
}

pub fn get_gep_offsets(
    base_ptr_type: &ir::Type,
    indices: &[(Value, usize)],
    include_padding: bool,
    i8_gep_div: usize,
) -> Result<GepResult, CompException> {
    let mut known_offset: i64 = 0;
    let mut unknown_offsets: Vec<GepUnknownOffset> = Vec::new();
    let mut is_arr_offset = true;
    let mut inner_type = base_ptr_type.clone();

    for (i, (index_val, index_width)) in indices.iter().enumerate() {
        if is_arr_offset {
            if i != 0 {
                if let ir::Type::Array(arr_ty) = &inner_type {
                    inner_type = arr_ty.inner.as_ref().clone();
                } else {
                    return Err(CompException("Expected array type for array offset".to_string()));
                }
            }

            let offset_size = get_size_of(&inner_type, include_padding)?;

            match index_val {
                Value::Known(kv) => match kv {
                    KnownVal::Num(n) if n.fract() == 0.0 => {
                        let no_twos_comp = super::twos_complement::comptime_undo_twos_complement(*n, *index_width);
                        known_offset += (no_twos_comp as i64) * (offset_size as i64);
                    }
                    _ => {
                        unknown_offsets.push(GepUnknownOffset {
                            index_val: index_val.clone(),
                            index_width: *index_width,
                            multiplier: offset_size,
                        });
                    }
                },
                _ => {
                    unknown_offsets.push(GepUnknownOffset {
                        index_val: index_val.clone(),
                        index_width: *index_width,
                        multiplier: offset_size,
                    });
                }
            }
        } else if let ir::Type::Struct(struct_ty) = &inner_type {
            match index_val {
                Value::Known(kv) => match kv {
                    KnownVal::Num(n) if n.fract() == 0.0 => {
                        let member_offset = *n as usize;
                        for member in &struct_ty.members[..member_offset.min(struct_ty.members.len())] {
                            known_offset += get_size_of(member, include_padding)? as i64;
                        }
                        if member_offset < struct_ty.members.len() {
                            inner_type = struct_ty.members[member_offset].clone();
                        }
                    }
                    _ => {
                        return Err(CompException("Struct index must be a known integer".to_string()));
                    }
                },
                _ => {
                    return Err(CompException("Struct index must be a known value".to_string()));
                }
            }
        } else {
            return Err(CompException("Expected struct type for struct offset".to_string()));
        }

        is_arr_offset = matches!(inner_type, ir::Type::Array(_));
    }

    let is_i8_gep = i8_gep_div > 1 && matches!(base_ptr_type, ir::Type::Integer(i) if i.width == 8);
    if is_i8_gep {
        known_offset /= i8_gep_div as i64;
    }

    Ok(GepResult {
        known_offset,
        unknown_offsets,
        result_type: inner_type,
        i8_gep_div,
    })
}

pub fn apply_gep_offsets(
    base: Value,
    known_offset: i64,
    unknown_offsets: &[GepUnknownOffset],
    is_nuw: bool,
    memory_size: usize,
    i8_gep_div: usize,
) -> Value {
    let mut final_val = base;

    if is_nuw {
        if known_offset != 0 {
            final_val = Value::Op(Op::Add(
                Box::new(final_val),
                Box::new(Value::Known(KnownVal::Num(known_offset as f64))),
            ));
        }
        for item in unknown_offsets {
            let offset = if i8_gep_div > 1 && item.multiplier == 1 {
                Value::Op(Op::Div(
                    Box::new(item.index_val.clone()),
                    Box::new(Value::Known(KnownVal::Num(i8_gep_div as f64))),
                ))
            } else if item.multiplier != 1 {
                Value::Op(Op::Mul(
                    Box::new(Value::Known(KnownVal::Num(item.multiplier as f64))),
                    Box::new(item.index_val.clone()),
                ))
            } else {
                item.index_val.clone()
            };
            final_val = Value::Op(Op::Add(Box::new(final_val), Box::new(offset)));
        }
    } else {
        let max_intermediate = (1u64 << 53) as f64;

        let mut twos_comp_sum: Vec<&GepUnknownOffset> = Vec::new();
        let mut rev_twos_comp: Vec<&GepUnknownOffset> = Vec::new();

        for item in unknown_offsets {
            let pow_width = 2f64.powi(item.index_width as i32);
            if item.index_width != PTR_WIDTH_BITS
                || pow_width + item.multiplier as f64 * pow_width >= max_intermediate
            {
                rev_twos_comp.push(item);
            } else {
                twos_comp_sum.push(item);
            }
        }

        twos_comp_sum.sort_by_key(|k| 2f64.powi(k.index_width as i32) as usize * k.multiplier);
        rev_twos_comp.sort_by_key(|k| 2f64.powi(k.index_width as i32) as usize * k.multiplier);

        let mut cuml_offset = known_offset;
        let has_rev_twos_comp = !rev_twos_comp.is_empty();

        for item in rev_twos_comp {
            let (rev, rev_offset) = super::twos_complement::undo_twos_complement_with_offset(
                item.index_val.clone(),
                item.index_width,
            );
            let this_offset =
                if i8_gep_div > 1 && item.multiplier == 1 {
                    rev_offset / i8_gep_div as i64
                } else {
                    rev_offset * item.multiplier as i64
                };

            let rev = if item.multiplier as f64 * 2f64.powi(item.index_width as i32) >= max_intermediate {
                Value::Op(Op::Add(Box::new(rev), Box::new(Value::Known(KnownVal::Num(rev_offset as f64)))))
            } else {
                rev
            };

            let actual_offset = if item.multiplier as f64 * 2f64.powi(item.index_width as i32) >= max_intermediate {
                0i64
            } else {
                this_offset
            };

            if cuml_offset + actual_offset >= max_intermediate as i64 {
                final_val = Value::Op(Op::Add(
                    Box::new(final_val),
                    Box::new(Value::Known(KnownVal::Num(cuml_offset as f64))),
                ));
                cuml_offset = 0;
            }

            cuml_offset += actual_offset;

            let offset_val = if i8_gep_div > 1 && item.multiplier == 1 {
                Value::Op(Op::Div(
                    Box::new(rev),
                    Box::new(Value::Known(KnownVal::Num(i8_gep_div as f64))),
                ))
            } else if item.multiplier != 1 {
                Value::Op(Op::Mul(
                    Box::new(Value::Known(KnownVal::Num(item.multiplier as f64))),
                    Box::new(rev),
                ))
            } else {
                rev
            };

            final_val = Value::Op(Op::Add(Box::new(final_val), Box::new(offset_val)));
        }

        if cuml_offset != 0 {
            final_val = Value::Op(Op::Add(
                Box::new(final_val),
                Box::new(Value::Known(KnownVal::Num(cuml_offset as f64))),
            ));
        }

        let mut cuml_max_val: f64 = if has_rev_twos_comp {
            10.0 * memory_size as f64
        } else {
            0.0
        };
        let final_mod_step = !twos_comp_sum.is_empty();

        for item in twos_comp_sum {
            let this_max_val = item.multiplier as f64 * 2f64.powi(item.index_width as i32);

            if cuml_max_val + this_max_val >= max_intermediate {
                final_val = Value::Op(Op::Mod(
                    Box::new(final_val),
                    Box::new(Value::Known(KnownVal::Num(2f64.powi(PTR_WIDTH_BITS as i32)))),
                ));
                cuml_max_val = 2f64.powi(PTR_WIDTH_BITS as i32);
            }

            cuml_max_val += this_max_val;

            let offset_val = if i8_gep_div > 1 && item.multiplier == 1 {
                Value::Op(Op::Div(
                    Box::new(item.index_val.clone()),
                    Box::new(Value::Known(KnownVal::Num(i8_gep_div as f64))),
                ))
            } else if item.multiplier != 1 {
                Value::Op(Op::Mul(
                    Box::new(Value::Known(KnownVal::Num(item.multiplier as f64))),
                    Box::new(item.index_val.clone()),
                ))
            } else {
                item.index_val.clone()
            };
            final_val = Value::Op(Op::Add(Box::new(final_val), Box::new(offset_val)));
        }

        if final_mod_step {
            final_val = Value::Op(Op::Mod(
                Box::new(final_val),
                Box::new(Value::Known(KnownVal::Num(2f64.powi(PTR_WIDTH_BITS as i32)))),
            ));
        }
    }

    final_val
}

pub enum PaddedValue {
    Single(Value),
    Indexed(IdxbleValue),
}

pub fn pad_value(value: PaddedValue, size: usize) -> PaddedValue {
    let mut values: Vec<Value> = match value {
        PaddedValue::Single(v) => vec![v],
        PaddedValue::Indexed(iv) => iv.vals,
    };
    let padding_len = size.saturating_sub(values.len());
    for _ in 0..padding_len {
        values.push(Value::Known(KnownVal::Num(0.0)));
    }

    if size > 1 {
        PaddedValue::Indexed(IdxbleValue { vals: values })
    } else {
        PaddedValue::Single(values.into_iter().next().unwrap_or(Value::Known(KnownVal::Num(0.0))))
    }
}

pub struct AggOffsetResult {
    pub offset: i64,
    pub size: usize,
}

pub fn get_agg_offset(
    agg: &ir::Type,
    indices: &[usize],
    include_padding: bool,
) -> Result<AggOffsetResult, CompException> {
    let gep_indices: Vec<(Value, usize)> = std::iter::once(0usize)
        .chain(indices.iter().copied())
        .map(|idx| (Value::Known(KnownVal::Num(idx as f64)), 32))
        .collect();

    let gep_result = get_gep_offsets(agg, &gep_indices, include_padding, 1)?;

    if !gep_result.unknown_offsets.is_empty() {
        return Err(CompException(
            "getAggOffset produced unknown offsets, but all indices should be known".to_string(),
        ));
    }

    let size = get_size_of(&gep_result.result_type, include_padding)?;

    Ok(AggOffsetResult {
        offset: gep_result.known_offset,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::types::{AddrSpace, ArrayTy, IntegerTy, PointerTy, StructTy};

    #[test]
    fn test_get_size_of_integer() {
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 32 }), false).unwrap(), 1);
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 8 }), false).unwrap(), 1);
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 64 }), false).unwrap(), 2);
    }

    #[test]
    fn test_get_size_of_integer_with_padding() {
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 32 }), true).unwrap(), 4);
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 8 }), true).unwrap(), 1);
        assert_eq!(get_size_of(&ir::Type::Integer(IntegerTy { width: 64 }), true).unwrap(), 8);
    }

    #[test]
    fn test_get_size_of_float() {
        assert_eq!(get_size_of(&ir::Type::Float, false).unwrap(), 1);
        assert_eq!(get_size_of(&ir::Type::Float, true).unwrap(), 4);
        assert_eq!(get_size_of(&ir::Type::Double, true).unwrap(), 8);
    }

    #[test]
    fn test_get_size_of_pointer() {
        assert_eq!(get_size_of(&ir::Type::Pointer(PointerTy { addrspace: AddrSpace::Default }), false).unwrap(), 1);
        assert_eq!(get_size_of(&ir::Type::Pointer(PointerTy { addrspace: AddrSpace::Default }), true).unwrap(), 4);
    }

    #[test]
    fn test_get_size_of_array() {
        let arr = ir::Type::Array(ArrayTy {
            size: 10,
            inner: Box::new(ir::Type::Integer(IntegerTy { width: 32 })),
        });
        assert_eq!(get_size_of(&arr, false).unwrap(), 10);
        assert_eq!(get_size_of(&arr, true).unwrap(), 40);
    }

    #[test]
    fn test_get_size_of_struct() {
        let s = ir::Type::Struct(StructTy {
            is_packed: false,
            members: vec![
                ir::Type::Integer(IntegerTy { width: 32 }),
                ir::Type::Integer(IntegerTy { width: 8 }),
            ],
        });
        assert_eq!(get_size_of(&s, false).unwrap(), 2);
        assert_eq!(get_size_of(&s, true).unwrap(), 5);
    }

    #[test]
    fn test_get_gep_offsets_simple() {
        let base_type = ir::Type::Integer(IntegerTy { width: 32 });
        let indices: Vec<(Value, usize)> = vec![
            (Value::Known(KnownVal::Num(3.0)), 32),
        ];
        let result = get_gep_offsets(&base_type, &indices, false, 1).unwrap();
        assert_eq!(result.known_offset, 3);
    }

    #[test]
    fn test_get_gep_offsets_unknown() {
        let base_type = ir::Type::Integer(IntegerTy { width: 32 });
        let indices: Vec<(Value, usize)> = vec![
            (Value::GetVar { name: "idx".to_string() }, 32),
        ];
        let result = get_gep_offsets(&base_type, &indices, false, 1).unwrap();
        assert_eq!(result.known_offset, 0);
        assert_eq!(result.unknown_offsets.len(), 1);
    }

    #[test]
    fn test_apply_gep_offsets_nuw() {
        let base = Value::Known(KnownVal::Num(10.0));
        let result = apply_gep_offsets(base, 5, &[], true, 4096, 1);
        assert!(matches!(result, Value::Op(..)));
    }

    #[test]
    fn test_apply_gep_offsets_zero_offset() {
        let base = Value::Known(KnownVal::Num(10.0));
        let result = apply_gep_offsets(base, 0, &[], true, 4096, 1);
        assert_eq!(result, Value::Known(KnownVal::Num(10.0)));
    }

    #[test]
    fn test_pad_value_single_to_indexed() {
        let val = PaddedValue::Single(Value::Known(KnownVal::Num(42.0)));
        let padded = pad_value(val, 3);
        match padded {
            PaddedValue::Indexed(iv) => {
                assert_eq!(iv.vals.len(), 3);
                assert_eq!(iv.vals[0], Value::Known(KnownVal::Num(42.0)));
                assert_eq!(iv.vals[1], Value::Known(KnownVal::Num(0.0)));
                assert_eq!(iv.vals[2], Value::Known(KnownVal::Num(0.0)));
            }
            _ => panic!("Expected Indexed"),
        }
    }

    #[test]
    fn test_pad_value_single_size_1() {
        let val = PaddedValue::Single(Value::Known(KnownVal::Num(42.0)));
        let padded = pad_value(val, 1);
        match padded {
            PaddedValue::Single(v) => assert_eq!(v, Value::Known(KnownVal::Num(42.0))),
            _ => panic!("Expected Single"),
        }
    }

    #[test]
    fn test_pad_value_indexed() {
        let iv = IdxbleValue {
            vals: vec![Value::Known(KnownVal::Num(1.0)), Value::Known(KnownVal::Num(2.0))],
        };
        let padded = pad_value(PaddedValue::Indexed(iv), 4);
        match padded {
            PaddedValue::Indexed(iv) => {
                assert_eq!(iv.vals.len(), 4);
                assert_eq!(iv.vals[2], Value::Known(KnownVal::Num(0.0)));
                assert_eq!(iv.vals[3], Value::Known(KnownVal::Num(0.0)));
            }
            _ => panic!("Expected Indexed"),
        }
    }

    #[test]
    fn test_apply_gep_offsets_twos_comp_sum_with_final_mod() {
        let base = Value::Known(KnownVal::Num(10.0));
        let offsets = vec![GepUnknownOffset {
            index_val: Value::Known(KnownVal::Num(3.0)),
            index_width: 32,
            multiplier: 1,
        }];
        let result = apply_gep_offsets(base, 0, &offsets, false, 4096, 1);
        assert!(matches!(result, Value::Op(Op::Mod(_, _))));
    }

    #[test]
    fn test_apply_gep_offsets_cuml_offset_before_twos_comp() {
        let base = Value::Known(KnownVal::Num(10.0));
        let offsets = vec![GepUnknownOffset {
            index_val: Value::GetVar { name: "idx".to_string() },
            index_width: 32,
            multiplier: 1,
        }];
        let result = apply_gep_offsets(base, 5, &offsets, false, 4096, 1);
        assert!(matches!(result, Value::Op(_)));
    }

    #[test]
    fn test_get_agg_offset_simple() {
        let agg = ir::Type::Struct(StructTy {
            is_packed: false,
            members: vec![
                ir::Type::Integer(IntegerTy { width: 32 }),
                ir::Type::Integer(IntegerTy { width: 8 }),
            ],
        });
        let result = get_agg_offset(&agg, &[0], false).unwrap();
        assert_eq!(result.offset, 0);
        assert_eq!(result.size, 1);
    }

    #[test]
    fn test_get_agg_offset_second_member() {
        let agg = ir::Type::Struct(StructTy {
            is_packed: false,
            members: vec![
                ir::Type::Integer(IntegerTy { width: 32 }),
                ir::Type::Integer(IntegerTy { width: 8 }),
            ],
        });
        let result = get_agg_offset(&agg, &[1], false).unwrap();
        assert_eq!(result.offset, 1);
        assert_eq!(result.size, 1);
    }

    #[test]
    fn test_get_agg_offset_nested() {
        let inner = ir::Type::Struct(StructTy {
            is_packed: false,
            members: vec![
                ir::Type::Integer(IntegerTy { width: 32 }),
                ir::Type::Integer(IntegerTy { width: 32 }),
            ],
        });
        let agg = ir::Type::Struct(StructTy {
            is_packed: false,
            members: vec![
                ir::Type::Integer(IntegerTy { width: 32 }),
                inner,
            ],
        });
        let result = get_agg_offset(&agg, &[1, 0], false).unwrap();
        assert_eq!(result.offset, 1);
        assert_eq!(result.size, 1);
    }
}