use std::collections::HashMap;

#[cfg(test)]
use num_bigint::BigUint;

use crate::ir::types::*;
use crate::ir::values::*;
use crate::ir::instructions::*;

pub const PARAM_ATTRS: &[&str] = &[
    "zeroext", "signext", "noext", "inreg",
    "byval", "byref", "preallocated", "inalloca",
    "sret", "elementtype", "align", "noalias",
    "captures", "nofree", "nest", "returned",
    "nonnull", "dereferenceable",
    "dereferenceable_or_null", "swiftself",
    "swiftasync", "swifterror", "immarg",
    "noundef", "nofpclass", "alignstack",
    "allocalign", "allocptr", "readnone",
    "readonly", "writeonly", "writeable",
    "initializes", "dead_on_unwind",
    "dead_on_return", "range",
];

pub const CALL_CONV: &[&str] = &[
    "ccc", "fastcc", "coldcc", "ghccc", "anyregcc",
    "preserve_mostcc", "preserve_allcc", "preserve_nonecc",
    "cxx_fast_tlscc", "tailcc", "swiftcc",
    "swifttailcc", "cfguard_checkcc", "cc",
];

pub fn pre_ret_func_attrs() -> Vec<&'static str> {
    let mut attrs: Vec<&'static str> = vec![
        "private", "internal", "available_externally", "linkonce",
        "weak", "common", "appending", "extern_weak",
        "linkonce_odr", "weak_odr", "external",
        "dso_preemptable", "dso_local",
        "default", "hidden", "protected",
        "dllimport", "dllexport", "localdynamic",
        "unnamed_addr", "local_unnamed_addr",
    ];
    attrs.extend(CALL_CONV);
    attrs.extend(PARAM_ATTRS);
    attrs
}

pub fn pre_ret_call_attrs() -> Vec<&'static str> {
    let mut attrs: Vec<&'static str> = vec![
        "tail", "musttail", "notail",
        "call",
        "nnan", "ninf", "nsz", "arcp", "contract",
        "afn", "reassoc", "fast",
        "addrspace",
    ];
    attrs.extend(CALL_CONV);
    attrs.extend(PARAM_ATTRS);
    attrs
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParseQuotedResult {
    pub decoded: String,
    pub rest: String,
    pub parsed_len: usize,
}

pub fn parse_quoted(rest: &str) -> Result<ParseQuotedResult, String> {
    if !rest.starts_with('"') {
        return Err(format!("Expected '\"' at start, got: {}", rest));
    }

    let chars: Vec<char> = rest.chars().collect();
    let mut decoded = String::new();
    let mut i = 1;
    let mut escaped = false;
    let mut escaped_hex = String::new();

    while chars[i] != '"' {
        let is_backslash = chars[i] == '\\';

        if escaped {
            if is_backslash {
                escaped = false;
                decoded.push('\\');
            } else {
                let c = chars[i];
                let cl = c.to_ascii_lowercase();
                if cl.is_ascii_digit() || ('a'..='f').contains(&cl) {
                    escaped_hex.push(cl);
                    if escaped_hex.len() >= 2 {
                        let val = u8::from_str_radix(&escaped_hex, 16)
                            .map_err(|e| e.to_string())?;
                        decoded.push(val as char);
                        escaped_hex.clear();
                        escaped = false;
                    }
                } else {
                    match c {
                        'n' => decoded.push('\n'),
                        't' => decoded.push('\t'),
                        'r' => decoded.push('\r'),
                        '"' => decoded.push('"'),
                        _ => decoded.push(c),
                    }
                    escaped = false;
                }
            }
        } else if is_backslash {
            escaped = true;
        } else {
            decoded.push(chars[i]);
        }
        i += 1;
    }

    i += 1;
    if escaped {
        return Err("Unterminated escape sequence".to_string());
    }

    let byte_pos = chars[..i].iter().map(|c| c.len_utf8()).sum();
    Ok(ParseQuotedResult {
        decoded,
        rest: rest[byte_pos..].to_string(),
        parsed_len: byte_pos,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseUntilResult {
    pub found: bool,
    pub tokens: Vec<String>,
    pub parsed: String,
    pub match_token: String,
    pub rest: String,
}

pub fn parse_until<F>(rest: &str, match_fn: F, ignore_unterminated: bool) -> Result<ParseUntilResult, String>
where
    F: Fn(&str) -> bool,
{
    let brackets_open = ['(', '[', '{', '<'];
    let brackets_close = [')', ']', '}', '>'];
    let bracket_pairs: HashMap<char, char> = [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('<', '>'),
    ]
    .into_iter()
    .collect();

    let chars: Vec<char> = rest.chars().collect();
    let total_len = chars.len();
    let mut i: usize = 0;
    let mut start_i;
    let mut token;
    let mut tokens: Vec<String> = Vec::new();
    let mut bracket_stack: Vec<char> = Vec::new();

    loop {
        token = String::new();
        start_i = i;

        if i > 0 && match_fn(&token) {
            break;
        }

        if i > total_len - 1 {
            return Ok(ParseUntilResult {
                found: false,
                tokens,
                parsed: rest.to_string(),
                match_token: String::new(),
                rest: String::new(),
            });
        }

        if bracket_pairs.contains_key(&chars[i]) {
            bracket_stack.push(bracket_pairs[&chars[i]]);
            while !bracket_stack.is_empty() {
                i += 1;
                if i > total_len - 1 {
                    if ignore_unterminated {
                        break;
                    } else {
                        return Err("Could not find closing bracket, exceeded length of rest".to_string());
                    }
                }

                if chars[i] == bracket_stack[bracket_stack.len() - 1] {
                    bracket_stack.pop();
                } else if brackets_close.contains(&chars[i]) {
                    return Err(format!(
                        "Got closing bracket without opening one: {} in {}",
                        chars[i], rest
                    ));
                } else if bracket_pairs.contains_key(&chars[i]) {
                    bracket_stack.push(bracket_pairs[&chars[i]]);
                } else if chars[i] == '"' {
                    let byte_pos: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
                    let qr = parse_quoted(&rest[byte_pos..])?;
                    i += qr.parsed_len - 1;
                }
            }
            let clamped_i = i.min(total_len - 1);
            let bp_start: usize = chars[..start_i].iter().map(|c| c.len_utf8()).sum();
            let bp_end: usize = chars[..=clamped_i].iter().map(|c| c.len_utf8()).sum();
            token = rest[bp_start..bp_end].trim().to_string();
        } else if brackets_close.contains(&chars[i]) {
            return Err(format!(
                "Got closing bracket without opening one: {} in {}",
                chars[i], rest
            ));
        } else if chars[i] == '"' {
            let byte_pos: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
            let qr = parse_quoted(&rest[byte_pos..])?;
            let bp_start: usize = chars[..start_i].iter().map(|c| c.len_utf8()).sum();
            i += qr.parsed_len - 1;
            let new_bp_end: usize = chars[..=i].iter().map(|c| c.len_utf8()).sum();
            token = rest[bp_start..new_bp_end].trim().to_string();
        } else {
            while i < total_len
                && chars[i] != ' '
                && !brackets_open.contains(&chars[i])
                && chars[i] != '"'
            {
                i += 1;
            }

            let bp_start: usize = chars[..start_i].iter().map(|c| c.len_utf8()).sum();
            let bp_end: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
            token = rest[bp_start..bp_end].trim().to_string();

            if i < total_len && (brackets_open.contains(&chars[i]) || chars[i] == '"') {
                i -= 1;
            }
        }

        if !token.is_empty() {
            tokens.push(token.clone());
        }
        i += 1;

        if match_fn(&token) {
            break;
        }
    }

    i = i.saturating_sub(1);

    let byte_start_i: usize = chars[..start_i].iter().map(|c| c.len_utf8()).sum();
    let byte_i: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();

    Ok(ParseUntilResult {
        found: true,
        tokens,
        parsed: rest[..byte_start_i].to_string(),
        match_token: rest[byte_start_i..byte_i].to_string(),
        rest: rest[byte_i..].to_string(),
    })
}

pub fn parse_until_end(rest: &str, ignore_unterminated: bool) -> Vec<String> {
    let result = parse_until(rest, |_| false, ignore_unterminated).unwrap();
    result.tokens
}

pub fn parse_until_end_strict(rest: &str) -> Vec<String> {
    parse_until_end(rest, false)
}

pub fn parse_until_comma(rest: &str) -> Vec<String> {
    let result = parse_until(rest, |x| x.ends_with(','), false).unwrap();
    let mut tokens = result.tokens;
    if tokens.is_empty() {
        return tokens;
    }
    if tokens[tokens.len() - 1] == "," {
        tokens.pop();
    } else {
        let last = tokens.len() - 1;
        if tokens[last].ends_with(',') {
            tokens[last] = tokens[last].trim_end_matches(',').to_string();
        }
    }
    tokens
}

pub fn parse_comma_separated(rest: &str) -> Vec<Vec<String>> {
    if rest.trim().is_empty() {
        return Vec::new();
    }

    let mut tokens_list: Vec<Vec<String>> = Vec::new();
    let mut remaining = rest;

    loop {
        let result = parse_until(remaining, |x| x.ends_with(','), false).unwrap();
        let mut tokens = result.tokens;
        if tokens.is_empty() {
            break;
        }
        if tokens[tokens.len() - 1] == "," {
            tokens.pop();
        } else {
            let last = tokens.len() - 1;
            if tokens[last].ends_with(',') {
                tokens[last] = tokens[last].trim_end_matches(',').to_string();
            }
        }
        tokens_list.push(tokens);
        if !result.found {
            break;
        }
        remaining = &remaining[remaining.len() - result.rest.len()..];
        if remaining.trim().is_empty() {
            break;
        }
    }

    tokens_list
}

pub fn parse_type_token(
    type_str: &str,
    structs: &HashMap<String, Type>,
    is_struct: bool,
) -> Result<Type, String> {
    let ts = type_str.trim();
    if ts == "define" || ts.starts_with(';') {
        panic!("parse_type_token called with ts={:?}", ts);
    }

    if (ts.starts_with('{') && ts.ends_with('}'))
        || (ts.starts_with("<{") && ts.ends_with("}>"))
        || is_struct
    {
        if ts == "opaque" {
            return Err("Opaque structures not yet supported".to_string());
        }

        let is_packed = ts.starts_with("<{");
        let rest = if is_packed {
            &ts[2..ts.len() - 2]
        } else {
            &ts[1..ts.len() - 1]
        };

        let tokens_list = parse_comma_separated(rest.trim());
        let mut members: Vec<Type> = Vec::new();
        for tokens in &tokens_list {
            let (member, remaining) = parse_type_tokens(tokens, structs)?;
            if !remaining.is_empty() {
                return Err(format!("Unexpected tokens after struct member: {:?}", remaining));
            }
            if !member.is_agg_target() {
                return Err(format!("Struct member must be AggTargetTy, got: {:?}", member));
            }
            members.push(member);
        }

        return Ok(Type::Struct(StructTy::new(is_packed, members)));
    }

    if ts == "void" {
        return Ok(Type::Void);
    }

    if ts.starts_with('i') && ts[1..].chars().all(|c| c.is_ascii_digit()) {
        let width: u32 = ts[1..].parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
        return Ok(Type::integer(width));
    }

    match ts {
        "half" => return Ok(Type::Half),
        "float" => return Ok(Type::Float),
        "double" => return Ok(Type::Double),
        "fp128" => return Ok(Type::Fp128),
        "bfloat" | "x86_fp80" | "ppc_fp128" => {
            return Err(format!("Unsupported FP type: {}", ts));
        }
        "x86_amx" | "x86_mmx" => {
            return Err(format!("Unsupported type: {}", ts));
        }
        "ptr" => {
            return Ok(Type::pointer(AddrSpace::Default));
        }
        "label" => return Ok(Type::Label),
        "token" => return Err("Token type not supported yet".to_string()),
        "metadata" => return Ok(Type::Metadata),
        _ => {}
    }

    if ts.starts_with('<') && ts.ends_with('>') && ts.contains(" x ") {
        let inner_str = &ts[1..ts.len() - 1];
        let parts: Vec<&str> = inner_str.splitn(2, " x ").collect();
        let size_str = parts[0].trim();
        let inner_type_str = parts[1].trim();

        if size_str.chars().all(|c| c.is_ascii_digit()) {
            let size: u32 = size_str.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
            let inner_tokens = parse_until_end_strict(inner_type_str);
            let (inner, remaining) = parse_type_tokens(&inner_tokens, structs)?;
            if !remaining.is_empty() {
                return Err(format!("Unexpected tokens after vec inner type: {:?}", remaining));
            }
            if !inner.is_vec_target() {
                return Err(format!("Vec inner must be VecTargetTy, got: {:?}", inner));
            }
            return Ok(Type::Vector(VecTy::new(inner, size)));
        } else if size_str == "vscale" {
            return Err(format!("Scalable vectors not supported: {}", ts));
        }
    }

    if ts.starts_with('[') && ts.ends_with(']') && ts.contains(" x ") {
        let inner_str = &ts[1..ts.len() - 1];
        let parts: Vec<&str> = inner_str.splitn(2, " x ").collect();
        let size: u32 = parts[0].trim().parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
        let inner_tokens = parse_until_end_strict(parts[1].trim());
        let (inner, remaining) = parse_type_tokens(&inner_tokens, structs)?;
        if !remaining.is_empty() {
            return Err(format!("Unexpected tokens after array inner type: {:?}", remaining));
        }
        if !inner.is_agg_target() {
            return Err(format!("Array inner must be AggTargetTy, got: {:?}", inner));
        }
        return Ok(Type::Array(ArrayTy::new(inner, size)));
    }

    if let Some(name) = ts.strip_prefix('%') {
        if let Some(ty) = structs.get(name) {
            return Ok(ty.clone());
        }
        return Err(format!("Unknown named struct: {} (structs: {:?})", name, structs.keys()));
    }

    Err(format!("Unsupported type: {}", ts))
}

pub fn parse_type_tokens(
    tokens: &[String],
    structs: &HashMap<String, Type>,
) -> Result<(Type, Vec<String>), String> {
    let mut is_struct = false;
    let mut is_addrspace = false;
    let mut current_type: Option<Type> = None;

    for i in 0..tokens.len() {
        let token = &tokens[i];

        if token.starts_with('(') && token.ends_with(')') {
            if is_addrspace {
                let contents = token[1..token.len() - 1].trim();
                let space: AddrSpace = if contents.starts_with('"') {
                    let qr = parse_quoted(contents)?;
                    AddrSpace::Named(qr.decoded)
                } else {
                    let n: u32 = contents.parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
                    AddrSpace::Number(n)
                };

                if let Some(Type::Pointer(ref mut ptr)) = current_type {
                    ptr.addrspace = space;
                } else {
                    return Err("addrspace without pointer type".to_string());
                }
                is_addrspace = false;
            } else {
                let ct = current_type
                    .as_ref()
                    .ok_or("Expected type before function params")?;

                let inner = &token[1..token.len() - 1];
                let tokens_list = parse_comma_separated(inner);

                let mut args: Vec<Type> = Vec::new();
                let mut variadic = false;

                for (j, sub_tokens) in tokens_list.iter().enumerate() {
                    if sub_tokens.len() == 1 && sub_tokens[0] == "..." {
                        if j != tokens_list.len() - 1 {
                            return Err("... must be last in params".to_string());
                        }
                        variadic = true;
                        continue;
                    }
                    let (parsed, remaining) = parse_type_tokens(sub_tokens, structs)?;
                    if !remaining.is_empty() {
                        return Err(format!("Unexpected tokens in func param: {:?}", remaining));
                    }
                    args.push(parsed);
                }

                current_type = Some(Type::Func(FuncTy::new(ct.clone(), args, variadic)));
            }
        } else if token == "type" {
            is_struct = true;
        } else if token == "addrspace" {
            match &current_type {
                Some(Type::Pointer(_)) => {
                    is_addrspace = true;
                }
                _ => return Err("addrspace without pointer type".to_string()),
            }
        } else if current_type.is_none() {
            current_type = Some(parse_type_token(token, structs, is_struct)?);
            is_struct = false;
        } else {
            return Ok((current_type.unwrap(), tokens[i..].to_vec()));
        }
    }

    current_type
        .map(|t| Ok((t, Vec::new())))
        .unwrap_or_else(|| Err("No type found".to_string()))
}

pub fn undo_twos_complement(val: i128, width: u32) -> i128 {
    if (val & (1i128 << (width - 1))) != 0 {
        val - (1i128 << width)
    } else {
        val
    }
}

pub fn apply_twos_complement(val: i128, width: u32) -> i128 {
    if val < 0 {
        val + (1i128 << width)
    } else {
        val
    }
}

fn parse_big_uint(s: &str, width: u32) -> Result<u128, String> {
    let mut result: u128 = 0;
    for ch in s.chars() {
        let digit = ch.to_digit(10).ok_or_else(|| format!("Invalid digit in big integer: {}", ch))?;
        if width < 128 {
            let modulus = 1u128 << width;
            result = (result * 10 + digit as u128) % modulus;
        } else {
            result = result.wrapping_mul(10).wrapping_add(digit as u128);
        }
    }
    Ok(result)
}

fn apply_twos_complement_big(abs_val: u128, width: u32) -> u128 {
    if width < 128 {
        let modulus = 1u128 << width;
        (modulus - abs_val % modulus) % modulus
    } else {
        (0u128).wrapping_sub(abs_val)
    }
}

pub fn parse_ieee_float(s: &str) -> Result<f64, String> {
    let bits_str = &s[2..];
    let bits: u64 = u64::from_str_radix(bits_str, 16).map_err(|e| e.to_string())?;
    Ok(f64::from_bits(bits))
}

pub fn get_zero_init_val(ty: &Type) -> Result<Value, String> {
    parse_constant_token(ty, "zeroinitializer", &HashMap::new(), &[], false, false)
}

pub fn parse_bracketed_list_token(
    token: &str,
    size: usize,
    opening: &str,
    closing: &str,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Vec<Value>, String> {
    if !token.starts_with(opening) || !token.ends_with(closing) {
        return Err(format!("Invalid constant: {}", token));
    }

    let inner = &token[opening.len()..token.len() - closing.len()];
    let member_tokens = parse_comma_separated(inner);
    if member_tokens.len() != size {
        return Err(format!(
            "Expected {} members, got {}",
            size,
            member_tokens.len()
        ));
    }

    let mut values = Vec::new();
    for mem_tokens in &member_tokens {
        let (mem, remaining) = parse_type_constant_tokens(mem_tokens, structs, func_names)?;
        if !remaining.is_empty() {
            return Err(format!("Unexpected tokens after bracketed member: {:?}", remaining));
        }
        values.push(mem);
    }

    Ok(values)
}

pub fn parse_constant_token(
    ty: &Type,
    token: &str,
    structs: &HashMap<String, Type>,
    func_names: &[String],
    is_char_arr: bool,
    is_splat: bool,
) -> Result<Value, String> {
    if token == "undef" || token == "poison" {
        return Ok(Value::Undef(UndefVal::new(ty.clone())));
    }

    // Local/global references can appear with any type (e.g. aggregate operands,
    // phi incoming values). Handle them before trying to parse a constant literal.
    if token.starts_with('%') {
        return Ok(Value::LocalVar(LocalVarVal::new(ty.clone(), &token[1..])));
    }
    if token.starts_with('@') {
        let name = &token[1..];
        if func_names.iter().any(|f| f == name) {
            return Ok(Value::Function(FunctionVal::new(ty.clone(), name)));
        } else {
            return Ok(Value::GlobalPtr(GlobalPtrVal::new(ty.clone(), name)));
        }
    }

    match ty {
        Type::Integer(int_ty) => {
            let width = int_ty.width;
            let value: u128 = if token == "true" {
                if width != 1 {
                    return Err(format!("true with width {}", width));
                }
                1
            } else if token == "false" {
                if width != 1 {
                    return Err(format!("false with width {}", width));
                }
                0
            } else if token.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(v) = token.parse::<u128>() {
                    v
                } else {
                    parse_big_uint(token, width)?
                }
            } else if token.starts_with('-') && token[1..].chars().all(|c| c.is_ascii_digit()) {
                let abs_str = &token[1..];
                if let Ok(neg) = abs_str.parse::<i128>() {
                    apply_twos_complement(-neg, width) as u128
                } else {
                    let abs_val = parse_big_uint(abs_str, width)?;
                    apply_twos_complement_big(abs_val, width)
                }
            } else if token.starts_with("u0x") || token.starts_with("s0x") {
                let hex_str = &token[3..];
                let val: u128 = u128::from_str_radix(hex_str, 16).map_err(|e| e.to_string())?;
                if token.starts_with("s0x") {
                    let value_width = 128 - val.leading_zeros();
                    apply_twos_complement(undo_twos_complement(val as i128, value_width), width) as u128
                } else {
                    val
                }
            } else if token == "zeroinitializer" {
                0
            } else {
                return Err(format!("Invalid integer constant: {}", token));
            };

            if width < 128 && value >= (1u128 << width) {
                return Err(format!("Integer {} too large for width {}", value, width));
            }

            Ok(Value::KnownInt(KnownIntVal::new(ty.clone(), value, width)))
        }

        Type::Half | Type::Float | Type::Double | Type::Fp128 => {
            let value: f64 = if token.contains('.') {
                if !token.contains('e') && !token.contains('E') {
                    token.parse().map_err(|e: std::num::ParseFloatError| e.to_string())?
                } else {
                    let parts: Vec<&str> = token.splitn(2, 'e').collect();
                    if parts.len() != 2 {
                        return Err(format!("Invalid exponential notation: {}", token));
                    }
                    let coeff: f64 = parts[0].parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
                    let exp_str = parts[1];
                    let exp_sign: i32 = if exp_str.starts_with('+') {
                        1
                    } else if exp_str.starts_with('-') {
                        -1
                    } else {
                        return Err(format!("Expected +/- in exponent: {}", exp_str));
                    };
                    let exp_val: u32 = exp_str[1..].parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
                    coeff * 10f64.powi(exp_sign * exp_val as i32)
                }
            } else if token.starts_with("0x") {
                if matches!(ty, Type::Fp128) {
                    return Err("0xL 128-bit fp format not yet supported".to_string());
                }
                parse_ieee_float(token)?
            } else if token == "zeroinitializer" {
                0.0
            } else {
                return Err(format!("Invalid fp constant: {}", token));
            };

            Ok(Value::KnownFloat(KnownFloatVal::new(ty.clone(), value)))
        }

        Type::Pointer(_) => {
            if token == "null" || token == "zeroinitializer" {
                return Ok(Value::NullPtr(NullPtrVal::new(ty.clone())));
            }

            if token.starts_with('@') || token.starts_with('%') {
                let name = &token[1..];
                let name_string = name.to_string();
                if func_names.iter().any(|f| f == &name_string) {
                    return Ok(Value::Function(FunctionVal::new(ty.clone(), name)));
                } else {
                    return Ok(Value::GlobalPtr(GlobalPtrVal::new(ty.clone(), name)));
                }
            }

            Err(format!("Invalid pointer constant: {}", token))
        }

        Type::Struct(struct_ty) => {
            if token == "zeroinitializer" {
                let mut mems = Vec::new();
                for mem in &struct_ty.members {
                    mems.push(get_zero_init_val(mem)?);
                }
                return Ok(Value::KnownStruct(KnownStructVal {
                    type_: ty.clone(),
                    values: mems,
                }));
            }

            let (opening, closing) = if struct_ty.is_packed {
                ("<{", "}>")
            } else {
                ("{", "}")
            };
            let mems = parse_bracketed_list_token(
                token,
                struct_ty.members.len(),
                opening,
                closing,
                structs,
                func_names,
            )?;
            Ok(Value::KnownStruct(KnownStructVal {
                type_: ty.clone(),
                values: mems,
            }))
        }

        Type::Array(arr_ty) => {
            if token == "zeroinitializer" {
                let mut arr_vals = Vec::new();
                for _ in 0..arr_ty.size {
                    arr_vals.push(get_zero_init_val(&arr_ty.inner)?);
                }
                return Ok(Value::KnownArr(KnownArrVal {
                    type_: ty.clone(),
                    values: arr_vals,
                }));
            }

            if !is_char_arr {
                let mems = parse_bracketed_list_token(
                    token,
                    arr_ty.size as usize,
                    "[",
                    "]",
                    structs,
                    func_names,
                )?;
                return Ok(Value::KnownArr(KnownArrVal {
                    type_: ty.clone(),
                    values: mems,
                }));
            }

            if *arr_ty.inner != Type::integer(8) {
                return Err("Char array inner type must be i8".to_string());
            }
            let qr = parse_quoted(token)?;
            if !qr.rest.is_empty() {
                return Err("Unexpected rest after char array".to_string());
            }
            if qr.decoded.len() != arr_ty.size as usize {
                return Err(format!(
                    "Char array size mismatch: expected {}, got {}",
                    arr_ty.size,
                    qr.decoded.len()
                ));
            }

            let mut values = Vec::new();
            for ch in qr.decoded.chars() {
                values.push(Value::KnownInt(KnownIntVal::new(
                    Type::integer(8),
                    ch as u128,
                    8,
                )));
            }
            Ok(Value::KnownArr(KnownArrVal {
                type_: ty.clone(),
                values,
            }))
        }

        Type::Vector(vec_ty) => {
            if token == "zeroinitializer" {
                let mut vec_vals = Vec::new();
                for _ in 0..vec_ty.size {
                    vec_vals.push(get_zero_init_val(&vec_ty.inner)?);
                }
                return Ok(Value::KnownVec(KnownVecVal {
                    type_: ty.clone(),
                    values: vec_vals,
                }));
            }

            if !is_splat {
                let mems = parse_bracketed_list_token(
                    token,
                    vec_ty.size as usize,
                    "<",
                    ">",
                    structs,
                    func_names,
                )?;
                return Ok(Value::KnownVec(KnownVecVal {
                    type_: ty.clone(),
                    values: mems,
                }));
            }

            if !token.starts_with('(') || !token.ends_with(')') {
                return Err(format!("Invalid splat vec constant: {}", token));
            }
            let inner_tokens = parse_until_end_strict(&token[1..token.len() - 1]);
            let (value, remaining) = parse_type_constant_tokens(&inner_tokens, structs, func_names)?;
            if !remaining.is_empty() {
                return Err(format!("Unexpected tokens in splat: {:?}", remaining));
            }

            let mut values = Vec::new();
            for _ in 0..vec_ty.size {
                values.push(value.clone());
            }
            Ok(Value::KnownVec(KnownVecVal {
                type_: ty.clone(),
                values,
            }))
        }

        _ => Err(format!(
            "Invalid constant - type {:?} not supported: {}",
            ty, token
        )),
    }
}

pub fn get_const_expr_bracket_values(
    brackets: &str,
    count: usize,
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<Vec<Value>, String> {
    if !brackets.starts_with('(') || !brackets.ends_with(')') {
        return Err(format!("Expected brackets: {}", brackets));
    }

    let tokens_list = parse_comma_separated(&brackets[1..brackets.len() - 1]);
    let mut values = Vec::new();
    for tokens in &tokens_list {
        let (parsed, remaining) = parse_type_constant_tokens(tokens, structs, func_names)?;
        if !remaining.is_empty() {
            return Err(format!("Unexpected tokens in const expr: {:?}", remaining));
        }
        values.push(parsed);
    }
    if values.len() != count {
        return Err(format!(
            "Expected {} values, got {}",
            count,
            values.len()
        ));
    }
    Ok(values)
}

pub fn parse_type_constant_tokens(
    tokens: &[String],
    structs: &HashMap<String, Type>,
    func_names: &[String],
) -> Result<(Value, Vec<String>), String> {
    let (ty, mut remaining) = parse_type_tokens(tokens, structs)?;

    if remaining.is_empty() {
        return Err("Expected constant value after type".to_string());
    }

    let first = &remaining[0];

    if matches!(
        first.as_str(),
        "blockaddress" | "dso_local_equivalent" | "no_cfi" | "ptrauth" | "asm"
    ) {
        return Err(format!("Unsupported value type {}", first));
    }

    let conv_opcodes = [
        "trunc", "ptrtoint", "ptrtoaddr", "inttoptr", "bitcast", "addrspacecast",
    ];
    if conv_opcodes.contains(&first.as_str()) {
        let opcode_str = first.as_str();
        let brackets = &remaining[1];
        if !brackets.starts_with('(') || !brackets.ends_with(')') {
            return Err(format!("Expected brackets after conv opcode: {}", brackets));
        }
        let contents = &brackets[1..brackets.len() - 1];

        let result = parse_until(contents, |x| x == "to", false)?;
        let mut casted_tokens = result.tokens;
        casted_tokens.pop();

        let (casted, rest_casted_tokens) =
            parse_type_constant_tokens(&casted_tokens, structs, func_names)?;
        if !rest_casted_tokens.is_empty() {
            return Err("Unexpected tokens after casted value".to_string());
        }

        let casted_type_tokens = parse_until_end_strict(&result.rest);
        let (casted_type, rest) = parse_type_tokens(&casted_type_tokens, structs)?;
        if !rest.is_empty() {
            return Err("Unexpected tokens after cast type".to_string());
        }

        let opcode = ConvOpcode::try_from_str(opcode_str)
            .ok_or_else(|| format!("Unknown conv opcode: {}", opcode_str))?;

        let conv = Conversion {
            result: ResultLocalVar::new(""),
            opcode,
            value: casted,
            res_type: casted_type,
            is_nuw: false,
            is_nsw: false,
        };

        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::Conversion(Box::new(conv)),
            }),
            remaining[2..].to_vec(),
        ));
    }

    if first == "getelementptr" {
        let mut i = 1;
        while !remaining[i].starts_with('(') {
            i += 1;
        }
        let brackets = &remaining[i];
        if !brackets.ends_with(')') {
            return Err("GEP brackets not closed".to_string());
        }

        let parts = parse_comma_separated(&brackets[1..brackets.len() - 1]);
        if parts.len() < 3 {
            return Err("GEP needs at least 3 parts".to_string());
        }

        let (base_ptr_type, rest) = parse_type_tokens(&parts[0], structs)?;
        if !rest.is_empty() {
            return Err("Unexpected tokens after GEP base type".to_string());
        }

        let mut parsed_parts = Vec::new();
        for part in &parts[1..] {
            let (parsed, rest) = parse_type_constant_tokens(part, structs, func_names)?;
            if !rest.is_empty() {
                return Err("Unexpected tokens in GEP part".to_string());
            }
            parsed_parts.push(parsed);
        }

        let base_ptr = parsed_parts[0].clone();

        let gep = GetElementPtr {
            result: ResultLocalVar::new(""),
            base_ptr_type,
            base_ptr,
            indices: parsed_parts[1..].to_vec(),
            is_inbounds: false,
            is_nusw: false,
            is_nuw: false,
        };

        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::GetElementPtr(Box::new(gep)),
            }),
            remaining[i + 1..].to_vec(),
        ));
    }

    if first == "extractelement" {
        let values = get_const_expr_bracket_values(&remaining[1], 2, structs, func_names)?;
        let ee = ExtractElement {
            result: ResultLocalVar::new(""),
            agg: values[0].clone(),
            index: values[1].clone(),
        };
        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::ExtractElement(Box::new(ee)),
            }),
            remaining[2..].to_vec(),
        ));
    }

    if first == "insertelement" {
        let values = get_const_expr_bracket_values(&remaining[1], 3, structs, func_names)?;
        let ie = InsertElement {
            result: ResultLocalVar::new(""),
            agg: values[0].clone(),
            item: values[1].clone(),
            index: values[2].clone(),
        };
        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::InsertElement(Box::new(ie)),
            }),
            remaining[2..].to_vec(),
        ));
    }

    if first == "shufflevector" {
        let values = get_const_expr_bracket_values(&remaining[1], 3, structs, func_names)?;
        let sv = ShuffleVector {
            result: ResultLocalVar::new(""),
            fst_vector: values[0].clone(),
            snd_vector: values[1].clone(),
            mask_vector: values[2].clone(),
        };
        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::ShuffleVector(Box::new(sv)),
            }),
            remaining[2..].to_vec(),
        ));
    }

    let binop_opcodes = ["add", "sub", "mul", "shl", "xor"];
    if binop_opcodes.contains(&first.as_str()) {
        let values = get_const_expr_bracket_values(&remaining[1], 2, structs, func_names)?;
        let opcode = BinaryOpcode::try_from_str(first)
            .ok_or_else(|| format!("Unknown binop: {}", first))?;
        let binop = BinaryOp {
            result: ResultLocalVar::new(""),
            opcode,
            left: values[0].clone(),
            right: values[1].clone(),
            is_nuw: false,
            is_nsw: false,
            is_exact: false,
            is_disjoint: false,
        };
        return Ok((
            Value::ConstExpr(ConstExprVal {
                type_: ty,
                expr: ConstExpr::BinaryOp(Box::new(binop)),
            }),
            remaining[2..].to_vec(),
        ));
    }

    let mut is_char_arr = false;
    let mut is_splat = false;

    if matches!(ty, Type::Array(_)) && first == "c" {
        remaining = remaining[1..].to_vec();
        is_char_arr = true;
    } else if matches!(ty, Type::Vector(_)) && first == "splat" {
        remaining = remaining[1..].to_vec();
        is_splat = true;
    }

    let val = parse_constant_token(&ty, &remaining[0], structs, func_names, is_char_arr, is_splat)?;
    Ok((val, remaining[1..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_quoted() {
        let result = parse_quoted("\"hello world\" etc etc").unwrap();
        assert_eq!(result.decoded, "hello world");
        assert_eq!(result.rest, " etc etc");
        assert_eq!(result.parsed_len, 13);
    }

    #[test]
    fn test_parse_quoted_backslash() {
        let result = parse_quoted(r#""\\"lol"#).unwrap();
        assert_eq!(result.decoded, "\\");
        assert_eq!(result.rest, "lol");
    }

    #[test]
    fn test_parse_quoted_byte() {
        let result = parse_quoted(r#""\22\00\\""another string""#).unwrap();
        assert_eq!(result.decoded, "\"\x00\\");
        assert_eq!(result.rest, "\"another string\"");
    }

    #[test]
    fn test_parse_until_normal() {
        let result = parse_until("hi, a, lol", |t| t.ends_with(','), false).unwrap();
        assert!(result.found);
        assert_eq!(result.tokens, vec!["hi,"]);
    }

    #[test]
    fn test_parse_until_bracket() {
        let result = parse_until(
            "[<>, \"hello\"] (\"hi\"), hi",
            |t| t == ",",
            false,
        )
        .unwrap();
        assert!(result.found);
        assert_eq!(result.tokens, vec!["[<>, \"hello\"]", "(\"hi\")", ","]);
    }

    #[test]
    fn test_parse_until_end() {
        let tokens = parse_until_end_strict("hi(lol())");
        assert_eq!(tokens, vec!["hi", "(lol())"]);
    }

    #[test]
    fn test_parse_type_int() {
        let result = parse_type_token("i32", &HashMap::new(), false).unwrap();
        assert_eq!(result, Type::integer(32));
    }

    #[test]
    fn test_parse_type_float() {
        let result = parse_type_token("fp128", &HashMap::new(), false).unwrap();
        assert_eq!(result, Type::Fp128);
    }

    #[test]
    fn test_parse_type_vec() {
        let result = parse_type_token("<2 x ptr>", &HashMap::new(), false).unwrap();
        assert_eq!(result, Type::Vector(VecTy::new(Type::pointer(AddrSpace::Default), 2)));
    }

    #[test]
    fn test_parse_type_arr() {
        let result = parse_type_token("[3 x [2 x <2 x i3>]]", &HashMap::new(), false).unwrap();
        assert_eq!(
            result,
            Type::Array(ArrayTy::new(
                Type::Array(ArrayTy::new(Type::Vector(VecTy::new(Type::integer(3), 2)), 2)),
                3
            ))
        );
    }

    #[test]
    fn test_parse_type_tokens_struct() {
        let tokens: Vec<String> = vec!["type".to_string(), "{ i32, float, <2 x i3> }".to_string()];
        let (result, rest) = parse_type_tokens(&tokens, &HashMap::new()).unwrap();
        assert!(rest.is_empty());
        assert_eq!(
            result,
            Type::Struct(StructTy::new(
                false,
                vec![Type::integer(32), Type::Float, Type::Vector(VecTy::new(Type::integer(3), 2))]
            ))
        );
    }

    #[test]
    fn test_parse_type_tokens_ptr_addrspace() {
        let tokens: Vec<String> = vec![
            "ptr".to_string(),
            "addrspace".to_string(),
            "(12)".to_string(),
        ];
        let (result, rest) = parse_type_tokens(&tokens, &HashMap::new()).unwrap();
        assert!(rest.is_empty());
        assert_eq!(result, Type::pointer(AddrSpace::Number(12)));
    }

    #[test]
    fn test_undo_twos_complement() {
        assert_eq!(undo_twos_complement(254, 8), -2);
        assert_eq!(undo_twos_complement(127, 8), 127);
    }

    #[test]
    fn test_apply_twos_complement() {
        assert_eq!(apply_twos_complement(-2, 8), 254);
        assert_eq!(apply_twos_complement(127, 8), 127);
    }

    #[test]
    fn test_parse_constant_int() {
        let ty = Type::integer(8);
        let result = parse_constant_token(&ty, "120", &HashMap::new(), &[], false, false).unwrap();
        match result {
            Value::KnownInt(k) => {
                assert_eq!(k.value, BigUint::from(120u32));
                assert_eq!(k.width, 8);
            }
            _ => panic!("Expected KnownIntVal"),
        }
    }

    #[test]
    fn test_parse_constant_neg_int() {
        let ty = Type::integer(8);
        let result = parse_constant_token(&ty, "-2", &HashMap::new(), &[], false, false).unwrap();
        match result {
            Value::KnownInt(k) => assert_eq!(k.value, BigUint::from(254u32)),
            _ => panic!("Expected KnownIntVal"),
        }
    }

    #[test]
    fn test_parse_constant_bool() {
        let ty = Type::integer(1);
        let result = parse_constant_token(&ty, "true", &HashMap::new(), &[], false, false).unwrap();
        match result {
            Value::KnownInt(k) => assert_eq!(k.value, BigUint::from(1u32)),
            _ => panic!("Expected KnownIntVal"),
        }
    }

    #[test]
    fn test_parse_constant_hex() {
        let ty = Type::integer(16);
        let result = parse_constant_token(&ty, "u0xFF", &HashMap::new(), &[], false, false).unwrap();
        match result {
            Value::KnownInt(k) => assert_eq!(k.value, BigUint::from(255u32)),
            _ => panic!("Expected KnownIntVal"),
        }
    }

    #[test]
    fn test_parse_constant_float() {
        let ty = Type::Double;
        let result = parse_constant_token(&ty, "-3.1415", &HashMap::new(), &[], false, false).unwrap();
        match result {
            Value::KnownFloat(k) => assert!((k.value - (-3.1415)).abs() < 1e-10),
            _ => panic!("Expected KnownFloatVal"),
        }
    }

    #[test]
    fn test_parse_constant_null_ptr() {
        let ty = Type::pointer(AddrSpace::Default);
        let result = parse_constant_token(&ty, "zeroinitializer", &HashMap::new(), &[], false, false).unwrap();
        assert!(matches!(result, Value::NullPtr(_)));
    }

    #[test]
    fn test_parse_ieee_float() {
        let result = parse_ieee_float("0x432ff973cafa8000").unwrap();
        assert!((result - 4.5e15).abs() / 4.5e15 < 1e-10);
    }
}