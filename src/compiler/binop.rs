use crate::optimizer::known_value_prop::simplify_value;
use crate::scratch::ast::{BoolOp, KnownVal, ListOp, Op};
use crate::scratch::{GetOfList, Value};

use super::config::{CompException, CompilerConfig, BINOP_LOOKUP_BITS};
use super::variable::{get_value_cost, sum_value_parts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinopKind {
    And,
    Or,
    Xor,
}

impl BinopKind {
    fn as_str(&self) -> &'static str {
        match self {
            BinopKind::And => "and",
            BinopKind::Or => "or",
            BinopKind::Xor => "xor",
        }
    }
}

pub fn lookup_table_name(kind: &BinopKind, cfg: &CompilerConfig) -> String {
    format!("!{} lookup{}", kind.as_str().to_uppercase(), cfg.zero_indexed_suffix)
}

fn as_known_num(value: &Value) -> Option<f64> {
    match value {
        Value::Known(KnownVal::Num(n)) => Some(*n),
        _ => None,
    }
}

fn known_to_int(value: &Value) -> Result<i128, CompException> {
    match value {
        Value::Known(KnownVal::Num(n)) => {
            if n.is_finite() && n.floor() == *n {
                Ok(*n as i128)
            } else {
                Err(CompException(format!("Cannot use non-integer known value {:?} in binop", n)))
            }
        }
        _ => Err(CompException("Expected known value".to_string())),
    }
}

fn extract_bits(
    val: Value,
    width: usize,
    start: usize,
    bits: usize,
    in_place: bool,
    skip_floor: bool,
) -> (Value, usize) {
    let end = start + bits;
    let shift = width - end;
    assert!(start == 0 || end <= width);

    let mut res = val;
    if shift != 0 {
        res = Value::Op(Op::Div(
            Box::new(res),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(shift as i32)))),
        ));
        if !skip_floor {
            res = Value::Op(Op::Floor(Box::new(res)));
        }
    }
    if start != 0 {
        res = Value::Op(Op::Mod(
            Box::new(res),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(bits as i32)))),
        ));
    }
    if in_place && shift != 0 {
        res = Value::Op(Op::Mul(
            Box::new(res),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(shift as i32)))),
        ));
    }
    (res, shift)
}

fn get_known_bit_groups(val: i128, width: usize) -> Vec<(i32, usize)> {
    let mut groups = Vec::new();
    let mut current_bit: Option<i32> = None;
    let mut current_len = 0usize;
    for i in (0..width).rev() {
        let bit = ((val >> i) & 1) as i32;
        if Some(bit) != current_bit {
            if let Some(b) = current_bit {
                groups.push((b, current_len));
            }
            current_bit = Some(bit);
            current_len = 0;
        }
        current_len += 1;
    }
    if let Some(b) = current_bit {
        groups.push((b, current_len));
    }
    groups
}

fn bin_op_part_via_lookup_table(
    kind: &BinopKind,
    lft: Value,
    rgt: Value,
    width: usize,
    start: usize,
    cfg: &CompilerConfig,
    bits: usize,
) -> Value {
    assert!(bits <= BINOP_LOOKUP_BITS);
    let table_name = lookup_table_name(kind, cfg);

    let mut lft = lft;
    let mut rgt = rgt;
    if as_known_num(&rgt).is_some() {
        std::mem::swap(&mut lft, &mut rgt);
    }

    let (extracted_lft, extracted_rgt, shift) = if width >= bits {
        let (l, s) = extract_bits(lft, width, start, bits, false, false);
        let (r, _) = extract_bits(rgt, width, start, bits, false, true);
        (l, r, s)
    } else {
        (lft, rgt, 0)
    };

    let lookup_index = Value::Op(Op::Add(
        Box::new(Value::Op(Op::Mul(
            Box::new(extracted_lft),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(BINOP_LOOKUP_BITS as i32)))),
        ))),
        Box::new(extracted_rgt),
    ));

    let mut res = Value::Op(Op::StrToFloat(Box::new(Value::GetOfList(GetOfList {
        op: ListOp::AtIndex,
        name: table_name,
        value: Box::new(lookup_index),
    }))));

    if shift != 0 {
        res = Value::Op(Op::Mul(
            Box::new(res),
            Box::new(Value::Known(KnownVal::Num(2f64.powi(shift as i32)))),
        ));
    }
    res
}

fn bin_op_with_known_via_lookup_table(
    kind: &BinopKind,
    unknown: Value,
    known: i128,
    width: usize,
    cfg: &CompilerConfig,
) -> Value {
    let mut i = 0;
    let mut final_offset: i128 = 0;
    let mut parts = Vec::new();

    while i < width {
        if *kind != BinopKind::Xor {
            let skip_over = if *kind == BinopKind::And { 0 } else { 1 };
            while i < width {
                let bit = (known >> (width - i - 1)) & 1;
                if bit != skip_over {
                    break;
                }
                if *kind == BinopKind::Or {
                    final_offset += 1i128 << (width - i - 1);
                }
                i += 1;
            }
            if i >= width {
                break;
            }
        }

        let bits = (BINOP_LOOKUP_BITS).min(width - i);
        parts.push(bin_op_part_via_lookup_table(
            kind,
            unknown.clone(),
            Value::Known(KnownVal::Num(known as f64)),
            width,
            i,
            cfg,
            bits,
        ));
        i += BINOP_LOOKUP_BITS;
    }

    if final_offset > 0 {
        parts.push(Value::Known(KnownVal::Num(final_offset as f64)));
    }

    sum_value_parts(&parts, Some(0.0))
}

fn bin_op_with_unknown_via_lookup_table(
    kind: &BinopKind,
    lft: Value,
    rgt: Value,
    width: usize,
    cfg: &CompilerConfig,
) -> Value {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < width {
        let bits = (BINOP_LOOKUP_BITS).min(width - i);
        parts.push(bin_op_part_via_lookup_table(
            kind, lft.clone(), rgt.clone(), width, i, cfg, bits,
        ));
        i += BINOP_LOOKUP_BITS;
    }
    sum_value_parts(&parts, Some(0.0))
}

fn and_with_known_mask_parts(
    unknown: &Value,
    known: i128,
    width: usize,
    cfg: &CompilerConfig,
) -> (Vec<Value>, bool) {
    let mut needs_lut = false;
    let mut current_bit_idx = 0;
    let mut parts = Vec::new();
    let mut groups = get_known_bit_groups(known, width);
    let mut i = 0;

    while current_bit_idx < width {
        let (bit, length) = groups[i];
        if bit == 0 {
            current_bit_idx += length;
            i += 1;
            continue;
        }

        let mut j = i;
        let mut next_regions: Vec<(usize, usize, usize)> = Vec::new();
        let mut current_reg_bit_idx = current_bit_idx;
        let region_end = (current_bit_idx + BINOP_LOOKUP_BITS).min(width);
        let mut reg_len = None;

        while current_reg_bit_idx < region_end {
            let (reg_bit, r_len) = groups[j];
            let current_region = (current_reg_bit_idx, r_len, j);
            current_reg_bit_idx += r_len;
            if reg_bit == 1 && current_reg_bit_idx <= region_end {
                next_regions.push(current_region);
            }
            reg_len = Some(r_len);
            j += 1;
            if j >= groups.len() {
                break;
            }
        }

        let extract_region = |idx: usize, r_len: usize| -> Value {
            extract_bits(unknown.clone(), width, idx, r_len, true, false).0
        };

        let use_lut_method = if reg_len.is_none() || next_regions.is_empty() {
            false
        } else {
            let extracted_regions: Vec<Value> = next_regions
                .iter()
                .map(|(reg_start, r_len, _)| extract_region(*reg_start, *r_len))
                .collect();
            let extract_region_method = sum_value_parts(&extracted_regions, Some(0.0));

            let bits = (BINOP_LOOKUP_BITS).min(width - current_bit_idx);
            let lookup_table_method = simplify_value(
                &bin_op_part_via_lookup_table(
                    &BinopKind::And,
                    unknown.clone(),
                    Value::Known(KnownVal::Num(known as f64)),
                    width,
                    current_bit_idx,
                    cfg,
                    bits,
                ),
                None,
            );
            let perf = &cfg.opt_target.perf;
            get_value_cost(&lookup_table_method, perf) < get_value_cost(&extract_region_method, perf)
        };

        if use_lut_method {
            let reg_len = reg_len.unwrap();
            let last_region_index = j - 1;
            let last_region_cut_off_by = reg_len as isize - (current_reg_bit_idx as isize - region_end as isize);
            assert!(last_region_cut_off_by >= 0);
            needs_lut = true;

            let bits = (BINOP_LOOKUP_BITS).min(width - current_bit_idx);
            parts.push(simplify_value(
                &bin_op_part_via_lookup_table(
                    &BinopKind::And,
                    unknown.clone(),
                    Value::Known(KnownVal::Num(known as f64)),
                    width,
                    current_bit_idx,
                    cfg,
                    bits,
                ),
                None,
            ));

            let old_region = groups[last_region_index];
            groups[last_region_index] = (old_region.0, old_region.1 - last_region_cut_off_by as usize);

            if last_region_index > 0 {
                let old_region_before = groups[last_region_index - 1];
                groups[last_region_index - 1] = (
                    old_region_before.0,
                    old_region_before.1 + last_region_cut_off_by as usize,
                );
            }

            let (reg_start, _, reg_i) = next_regions[next_regions.len() - 1];
            i = reg_i + 1;
            current_bit_idx = reg_start + groups[reg_i].1;
            if reg_i >= last_region_index {
                current_bit_idx += last_region_cut_off_by as usize;
            }
        } else {
            parts.push(extract_region(current_bit_idx, length));
            current_bit_idx += length;
            i += 1;
        }
    }

    (parts, needs_lut)
}

fn and_with_known_mask(
    unknown: &Value,
    known: i128,
    width: usize,
    cfg: &CompilerConfig,
) -> (Value, bool) {
    let (parts, needs_lut) = and_with_known_mask_parts(unknown, known, width, cfg);
    (sum_value_parts(&parts, Some(0.0)), needs_lut)
}

fn or_with_known_mask(
    unknown: &Value,
    known: i128,
    width: usize,
    cfg: &CompilerConfig,
) -> (Value, bool, bool) {
    let not_b = (1i128 << width) - 1 - known;
    let (a_and_b, needs_and_lut) = and_with_known_mask(unknown, not_b, width, cfg);
    let a_or_b_with_and = Value::Op(Op::Add(
        Box::new(a_and_b),
        Box::new(Value::Known(KnownVal::Num(known as f64))),
    ));
    let a_or_b_specialized_lut = simplify_value(
        &bin_op_with_known_via_lookup_table(&BinopKind::Or, unknown.clone(), known, width, cfg),
        None,
    );
    let perf = &cfg.opt_target.perf;
    if get_value_cost(&a_or_b_with_and, perf) < get_value_cost(&a_or_b_specialized_lut, perf) {
        (a_or_b_with_and, needs_and_lut, false)
    } else {
        (a_or_b_specialized_lut, false, true)
    }
}

fn xor_with_known_mask(
    unknown: &Value,
    known: i128,
    width: usize,
    cfg: &CompilerConfig,
) -> (Value, bool, bool) {
    let full_mask = (1i128 << width) - 1;
    if known == full_mask {
        return (
            Value::Op(Op::Sub(
                Box::new(Value::Known(KnownVal::Num(known as f64))),
                Box::new(unknown.clone()),
            )),
            false,
            false,
        );
    }

    let (a_and_b_parts, needs_and_lut) = and_with_known_mask_parts(unknown, known, width, cfg);

    let mut use_parts = true;
    let mut a_and_b_times_2_parts = Vec::new();
    for part in &a_and_b_parts {
        match part {
            Value::Op(Op::Mul(l, r)) => {
                if let Value::Known(KnownVal::Num(n)) = r.as_ref() {
                    if n.is_finite() && n.floor() == *n {
                        a_and_b_times_2_parts.push(Value::Op(Op::Mul(
                            l.clone(),
                            Box::new(Value::Known(KnownVal::Num(*n * 2.0))),
                        )));
                        continue;
                    }
                }
                use_parts = false;
                break;
            }
            _ => {
                use_parts = false;
                break;
            }
        }
    }

    let a_and_b_times_2 = if use_parts {
        sum_value_parts(&a_and_b_times_2_parts, Some(0.0))
    } else {
        Value::Op(Op::Mul(
            Box::new(sum_value_parts(&a_and_b_parts, Some(0.0))),
            Box::new(Value::Known(KnownVal::Num(2.0))),
        ))
    };

    let a_xor_b_with_and = Value::Op(Op::Add(
        Box::new(Value::Op(Op::Sub(
            Box::new(unknown.clone()),
            Box::new(a_and_b_times_2),
        ))),
        Box::new(Value::Known(KnownVal::Num(known as f64))),
    ));
    let a_xor_b_specialized_lut = simplify_value(
        &bin_op_with_known_via_lookup_table(&BinopKind::Xor, unknown.clone(), known, width, cfg),
        None,
    );
    let perf = &cfg.opt_target.perf;
    if get_value_cost(&a_xor_b_with_and, perf) < get_value_cost(&a_xor_b_specialized_lut, perf) {
        (a_xor_b_with_and, needs_and_lut, false)
    } else {
        (a_xor_b_specialized_lut, false, true)
    }
}

pub fn binop(
    kind: BinopKind,
    lft: Value,
    rgt: Value,
    width: usize,
    cfg: &CompilerConfig,
    needs_and_lut: &mut bool,
    needs_or_lut: &mut bool,
    needs_xor_lut: &mut bool,
) -> Result<Value, CompException> {
    if kind == BinopKind::Or && false {
        // is_disjoint handling is done by the caller
        return Ok(Value::Op(Op::Add(Box::new(lft), Box::new(rgt))));
    }

    if width == 1 {
        return match kind {
            BinopKind::And => Ok(Value::Op(Op::Mul(Box::new(lft), Box::new(rgt)))),
            BinopKind::Or => Ok(Value::Op(Op::BoolToFloat(Box::new(Value::BoolOp(
                BoolOp::Gt(
                    Box::new(Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))),
                    Box::new(Value::Known(KnownVal::Num(0.0))),
                ),
            ))))),
            BinopKind::Xor => {
                if as_known_num(&lft).is_some() || as_known_num(&rgt).is_some() {
                    let unknown = if as_known_num(&lft).is_some() { rgt } else { lft };
                    Ok(Value::Op(Op::Sub(
                        Box::new(Value::Known(KnownVal::Num(1.0))),
                        Box::new(unknown),
                    )))
                } else {
                    Ok(Value::Op(Op::Mod(
                        Box::new(Value::Op(Op::Add(Box::new(lft), Box::new(rgt)))),
                        Box::new(Value::Known(KnownVal::Num(2.0))),
                    )))
                }
            }
        };
    }

    let lft_known = as_known_num(&lft).is_some();
    let rgt_known = as_known_num(&rgt).is_some();

    if lft_known || rgt_known {
        let known_val = if lft_known { known_to_int(&lft)? } else { known_to_int(&rgt)? };
        let unknown = if lft_known { rgt } else { lft };

        let (res, na, no, nx) = match kind {
            BinopKind::And => {
                let (res, na) = and_with_known_mask(&unknown, known_val, width, cfg);
                (res, na, false, false)
            }
            BinopKind::Or => {
                let (res, na, no) = or_with_known_mask(&unknown, known_val, width, cfg);
                (res, na, no, false)
            }
            BinopKind::Xor => {
                let (res, na, nx) = xor_with_known_mask(&unknown, known_val, width, cfg);
                (res, na, false, nx)
            }
        };
        *needs_and_lut |= na;
        *needs_or_lut |= no;
        *needs_xor_lut |= nx;
        Ok(res)
    } else {
        match kind {
            BinopKind::And => *needs_and_lut = true,
            BinopKind::Or => *needs_or_lut = true,
            BinopKind::Xor => *needs_xor_lut = true,
        }
        Ok(bin_op_with_unknown_via_lookup_table(&kind, lft, rgt, width, cfg))
    }
}
