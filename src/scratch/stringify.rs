use crate::scratch::ast::*;
use crate::scratch::Project;

const SCRATCHBLOCKS_MESSAGE: &str = "(::ring)Compiled with llvm2scratch!(::ring)::extension ring // Special blocks used internally by the compiler\n(bool to float <>::extension) // This converts a boolean to an int using the round(_) block if necessary\n(str to float ()::extension) // This converts a string to a float using the (_ + 0) block if necessary\n<true::extension> // Known true block using the <not <>> block\n<false::extension> // Known false block using an empty boolean input";

fn escape_scratch_blocks_str(val: &str) -> String {
    // Escape backslashes first so that backslashes inserted for other escapes
    // are not doubled.
    let mut res = val.replace('\\', "\\\\");
    res = res.replace("::", "\\:\\:");
    for c in r"()[]<>".chars() {
        res = res.replace(c, &format!("\\{}", c));
    }
    if res.ends_with(" v") {
        let mut chars: Vec<char> = res.chars().collect();
        chars.pop(); // remove 'v'
        chars.push('\\');
        chars.push('v');
        res = chars.into_iter().collect();
    }
    res
}

fn known_to_scratch_blocks(val: &KnownVal, dropdown: bool) -> String {
    match val {
        KnownVal::Bool(b) => format!("<{}::extension>", if *b { "true" } else { "false" }),
        KnownVal::Num(n) => {
            if n.is_infinite() {
                return if *n > 0.0 { "[Infinity]".to_string() } else { "[-Infinity]".to_string() };
            }
            if n.is_nan() {
                return "[NaN]".to_string();
            }
            let int_val = *n as i64;
            if *n == int_val as f64 {
                format!("({})", int_val)
            } else {
                format!("({})", n)
            }
        }
        KnownVal::Str(s) => {
            format!("[{}{}]", escape_scratch_blocks_str(s), if dropdown { " v" } else { "" })
        }
    }
}

fn known_to_plain_str(val: &KnownVal) -> String {
    match val {
        KnownVal::Str(s) => format!("\"{}\"", s),
        KnownVal::Num(n) => {
            if n.is_infinite() && *n > 0.0 {
                "Infinity".to_string()
            } else if n.is_infinite() && *n < 0.0 {
                "-Infinity".to_string()
            } else if n.is_nan() {
                "NaN".to_string()
            } else if *n == 0.0 && n.is_sign_negative() {
                "-0".to_string()
            } else if n.fract() == 0.0 {
                (*n as i64).to_string()
            } else {
                n.to_string()
            }
        }
        KnownVal::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
    }
}

fn stringify_known(val: &KnownVal, scratchblocks: bool, dropdown: bool) -> String {
    if scratchblocks {
        known_to_scratch_blocks(val, dropdown)
    } else {
        known_to_plain_str(val)
    }
}

fn stringify_known_bool(b: bool, scratchblocks: bool) -> String {
    if scratchblocks {
        format!("<{}::extension>", if b { "true" } else { "false" })
    } else {
        format!("<{}>", if b { "true" } else { "false" })
    }
}

fn is_numeric_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.')
}

fn localize_param_name(name: &str) -> String {
    if name.starts_with('%') {
        name.to_string()
    } else {
        format!("%{}", name)
    }
}

fn stringify_value(value: &Value, scratchblocks: bool) -> String {
    match value {
        Value::Known(kv) => stringify_known(kv, scratchblocks, false),
        Value::KnownBool(b) => stringify_known_bool(*b, scratchblocks),
        Value::Op(op) => stringify_op(op, scratchblocks),
        Value::BoolOp(bop) => stringify_bool_op(bop, scratchblocks),
        Value::GetVar { name } => {
            if scratchblocks {
                let escaped = escape_scratch_blocks_str(name);
                if is_numeric_name(&escaped) {
                    format!("({}::variables)", escaped)
                } else {
                    format!("({})", escaped)
                }
            } else {
                format!("({})", name)
            }
        }
        Value::GetList { name } => {
            if scratchblocks {
                format!("({}::list)", escape_scratch_blocks_str(name))
            } else {
                format!("(list {})", name)
            }
        }
        Value::GetOfList(data) => {
            let var = if scratchblocks {
                stringify_known(&KnownVal::Str(data.name.clone()), true, true)
            } else {
                data.name.clone()
            };
            let val = stringify_value(&data.value, scratchblocks);
            match data.op {
                ListOp::AtIndex => format!("(item {} of {})", val, var),
                ListOp::IndexOf => format!("(item # of {} in {})", val, var),
                ListOp::LengthOf => format!("(item {} of {})", val, var),
                ListOp::Contains => format!("(item {} of {})", val, var),
            }
        }
        Value::GetListLength { name } => {
            if scratchblocks {
                let var = stringify_known(&KnownVal::Str(name.clone()), true, true);
                format!("(length of {})", var)
            } else {
                format!("(length of list {})", name)
            }
        }
        Value::GetParam { name } => {
            let localized = localize_param_name(name);
            if scratchblocks {
                format!("({}::custom)", escape_scratch_blocks_str(&localized))
            } else {
                format!("(param {})", localized)
            }
        }
        Value::CostumeInfo { op } => {
            let op_str = match op {
                CostumeInfoOp::Name => "name",
                CostumeInfoOp::Number => "number",
            };
            if scratchblocks {
                format!("(costume [{} v])", op_str)
            } else {
                format!("(costume {})", op_str)
            }
        }
        Value::GetCounter => {
            if scratchblocks {
                "(counter::control)".to_string()
            } else {
                "(counter)".to_string()
            }
        }
        Value::GetAnswer => "(answer)".to_string(),
        Value::DaysSince2000 => "(days since 2000)".to_string(),
    }
}

fn stringify_op(op: &Op, scratchblocks: bool) -> String {
    let left = stringify_value(op.left(), scratchblocks);
    let right = op.right();
    match op {
        Op::Add(_, _) => format!("({} + {})", left, stringify_value(right, scratchblocks)),
        Op::Sub(_, _) => format!("({} - {})", left, stringify_value(right, scratchblocks)),
        Op::Mul(_, _) => format!("({} * {})", left, stringify_value(right, scratchblocks)),
        Op::Div(_, _) => format!("({} / {})", left, stringify_value(right, scratchblocks)),
        Op::Mod(_, _) => format!("({} mod {})", left, stringify_value(right, scratchblocks)),
        Op::Rand(_, _) => format!("(pick random {} to {})", left, stringify_value(right, scratchblocks)),
        Op::Join(_, _) => format!("(join {} {})", left, stringify_value(right, scratchblocks)),
        Op::LetterOf(_, _) => format!("(letter {} of {})", left, stringify_value(right, scratchblocks)),
        Op::LengthOf(_) => format!("(length of {})", left),
        Op::Round(_) => format!("(round {})", left),
        Op::Not(_) => format!("<not {}>", left),
        Op::Contains(_, _) => {
            let suffix = if scratchblocks { "?" } else { "" };
            format!("<{} contains {}{}>", left, stringify_value(right, scratchblocks), suffix)
        }
        Op::BoolToFloat(_) => {
            let force = if scratchblocks { "::extension" } else { "" };
            format!("(bool to float {}{})", left, force)
        }
        Op::StrToFloat(_) => {
            let force = if scratchblocks { "::extension" } else { "" };
            format!("(str to float {}{})", left, force)
        }
        Op::Abs(_) |
        Op::Floor(_) |
        Op::Ceiling(_) |
        Op::Sqrt(_) |
        Op::Sin(_) |
        Op::Cos(_) |
        Op::Tan(_) |
        Op::Asin(_) |
        Op::Acos(_) |
        Op::Atan(_) |
        Op::Ln(_) |
        Op::Log(_) |
        Op::Exp(_) |
        Op::Exp10(_) => {
            let name = match op {
                Op::Abs(_) => "abs",
                Op::Floor(_) => "floor",
                Op::Ceiling(_) => "ceiling",
                Op::Sqrt(_) => "sqrt",
                Op::Sin(_) => "sin",
                Op::Cos(_) => "cos",
                Op::Tan(_) => "tan",
                Op::Asin(_) => "asin",
                Op::Acos(_) => "acos",
                Op::Atan(_) => "atan",
                Op::Ln(_) => "ln",
                Op::Log(_) => "log",
                Op::Exp(_) => "e ^",
                Op::Exp10(_) => "10 ^",
                _ => unreachable!(),
            };
            if scratchblocks {
                format!("([{} v] of {})", name, left)
            } else {
                format!("({} of {})", name, left)
            }
        }
    }
}

fn stringify_bool_op(bop: &BoolOp, scratchblocks: bool) -> String {
    let left = stringify_value(bop.left(), scratchblocks);
    if let Some(right) = bop.right_opt() {
        let op_str = match bop {
            BoolOp::And(_, _) => "and",
            BoolOp::Or(_, _) => "or",
            BoolOp::Eq(_, _) => "=",
            BoolOp::Lt(_, _) => "<",
            BoolOp::Gt(_, _) => ">",
            BoolOp::Not(_) => unreachable!(),
        };
        let suffix = if scratchblocks && matches!(bop, BoolOp::Eq(_, _)) && op_str == "=" {
            // Python special-cases "contains" with a ? suffix; = has no suffix.
            ""
        } else {
            ""
        };
        format!("<{} {} {}{}>", left, op_str, stringify_value(right, scratchblocks), suffix)
    } else {
        format!("<not {}>", left)
    }
}

impl BoolOp {
    fn right_opt(&self) -> Option<&Value> {
        match self {
            BoolOp::And(_, r) |
            BoolOp::Or(_, r) |
            BoolOp::Eq(_, r) |
            BoolOp::Lt(_, r) |
            BoolOp::Gt(_, r) => Some(r),
            BoolOp::Not(_) => None,
        }
    }
}

fn indent_block_list(blocks: &BlockList, scratchblocks: bool) -> String {
    let inner = stringify_block_list(blocks, scratchblocks);
    inner
        .lines()
        .map(|line| format!("  {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stringify_block_list(blocks: &BlockList, scratchblocks: bool) -> String {
    blocks
        .blocks
        .iter()
        .map(|b| stringify_block(b, scratchblocks))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stringify_block(block: &Block, scratchblocks: bool) -> String {
    match block {
        Block::Say { value } => format!("say {}", stringify_value(value, scratchblocks)),
        Block::SwitchCostume { value } => {
            let inner = if scratchblocks {
                if let Value::Known(kv) = value {
                    stringify_known(kv, true, true)
                } else {
                    stringify_value(value, true)
                }
            } else {
                stringify_value(value, false)
            };
            format!("switch costume to {}", inner)
        }
        Block::EditVolume { op, value } => {
            let prefix = match op {
                VolumeOp::Set => "set volume to ",
                VolumeOp::Change => "change volume by ",
            };
            format!("{}{}", prefix, stringify_value(value, scratchblocks))
        }
        Block::Broadcast { value, wait } => {
            let inner = if scratchblocks {
                if let Value::Known(kv) = value {
                    stringify_known(kv, true, true)
                } else {
                    stringify_value(value, true)
                }
            } else {
                stringify_value(value, false)
            };
            if *wait {
                format!("broadcast {} and wait", inner)
            } else {
                format!("broadcast {}", inner)
            }
        }
        Block::OnBroadcast { name } => {
            if scratchblocks {
                format!("when I recieve [{} v]", escape_scratch_blocks_str(name))
            } else {
                format!("when I recieve {}", name)
            }
        }
        Block::OnStartFlag => "when green flag clicked".to_string(),
        Block::ControlFlow(cf) => stringify_control_flow(cf, scratchblocks),
        Block::StopScript(opt) => {
            let (plain, dropdown) = match opt {
                StopOption::All => ("stop all", "stop all"),
                StopOption::This => ("this script", "this script"),
                StopOption::Other => ("other scripts in sprite", "other scripts in sprite"),
            };
            if scratchblocks {
                format!("stop [{} v]", dropdown)
            } else {
                format!("stop {}", plain)
            }
        }
        Block::EditCounter(op) => {
            let action = match op {
                CounterOp::Increment => "increment counter",
                CounterOp::Decrement => "decrement counter",
                CounterOp::Reset => "clear counter",
            };
            if scratchblocks {
                format!("{}::control", action)
            } else {
                action.to_string()
            }
        }
        Block::Wait { value } => format!("wait {} seconds", stringify_value(value, scratchblocks)),
        Block::Ask { value, .. } => format!("ask {} and wait", stringify_value(value, scratchblocks)),
        Block::EditVar(data) => {
            let value_str = stringify_value(&data.value, scratchblocks);
            if scratchblocks {
                let var = stringify_known(&KnownVal::Str(data.name.clone()), true, true);
                match data.op {
                    VarOp::Set => format!("set {} to {}", var, value_str),
                    VarOp::Change => format!("change {} by {}", var, value_str),
                }
            } else {
                match data.op {
                    VarOp::Set => format!("{} = {}", data.name, value_str),
                    VarOp::Change => format!("change {} by {}", data.name, value_str),
                }
            }
        }
        Block::EditList(data) => {
            let var = if scratchblocks {
                stringify_known(&KnownVal::Str(data.name.clone()), true, true)
            } else {
                data.name.clone()
            };
            match data.op {
                ListEditOp::AddTo => {
                    let item = data.value.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    format!("add {} to {}", item, var)
                }
                ListEditOp::ReplaceAt => {
                    let index = data.index.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    let item = data.value.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    format!("replace item {} of {} with {}", index, var, item)
                }
                ListEditOp::InsertAt => {
                    let index = data.index.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    let item = data.value.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    format!("insert {} at {} of {}", item, index, var)
                }
                ListEditOp::DeleteAt => {
                    let index = data.index.as_ref().map(|v| stringify_value(v, scratchblocks)).unwrap_or_default();
                    format!("delete {} of {}", index, var)
                }
                ListEditOp::DeleteAll => {
                    format!("delete all of {}", var)
                }
            }
        }
        Block::ProcedureDef(data) => {
            let name = if scratchblocks {
                escape_scratch_blocks_str(&data.name)
            } else {
                data.name.clone()
            };
            let params: Vec<String> = data
                .params
                .iter()
                .map(|p| {
                    let localized = localize_param_name(p);
                    format!("({})", if scratchblocks { escape_scratch_blocks_str(&localized) } else { localized })
                })
                .collect();
            std::iter::once("define".to_string())
                .chain(std::iter::once(name))
                .chain(params)
                .collect::<Vec<_>>()
                .join(" ")
        }
        Block::ProcedureCall(data) => {
            let name = if scratchblocks {
                escape_scratch_blocks_str(&data.name)
            } else {
                data.name.clone()
            };
            let args: Vec<String> = data
                .args
                .iter()
                .map(|a| stringify_value(a, scratchblocks))
                .collect();
            if scratchblocks {
                let mut parts = vec![name];
                parts.extend(args);
                parts.push("::custom".to_string());
                parts.join(" ")
            } else {
                let mut parts = vec!["call".to_string(), name];
                parts.extend(args);
                parts.join(" ")
            }
        }
        Block::Pen(op) => {
            match op {
                PenOp::Down => "pen down".to_string(),
                PenOp::Up => "pen up".to_string(),
                PenOp::Clear => "clear pen".to_string(),
                PenOp::SetColor { color } => {
                    let c = stringify_value(color, scratchblocks);
                    format!("set pen color to {}", c)
                }
                PenOp::SetSize { size } => {
                    let s = stringify_value(size, scratchblocks);
                    format!("set pen size to {}", s)
                }
            }
        }
        Block::MotionGoto { x, y } => {
            format!("go to x: {} y: {}", stringify_value(x, scratchblocks), stringify_value(y, scratchblocks))
        }
        Block::RawBlock(contents) => {
            // Raw blocks are not expected to be stringified; output a readable fallback.
            format!("// raw block: {:?}", contents)
        }
    }
}

fn stringify_control_flow(cf: &ControlFlow, scratchblocks: bool) -> String {
    let keyword = match cf.op {
        ControlOp::If | ControlOp::IfElse => "if",
        ControlOp::RepTimes => "repeat",
        ControlOp::Until => "repeat until",
        ControlOp::While => "while",
        ControlOp::Forever => "forever",
        ControlOp::ForEach => "for each",
    };

    let mut res = keyword.to_string();

    if cf.op == ControlOp::ForEach {
        if let Some(var) = &cf.var {
            let name = if scratchblocks {
                stringify_known(&KnownVal::Str(var.clone()), true, true)
            } else {
                var.clone()
            };
            res.push_str(&format!(" {} in", name));
        }
    }

    if let Some(cond) = &cf.condition {
        res.push_str(&format!(" {}", stringify_value(cond, scratchblocks)));
    }

    if scratchblocks {
        match cf.op {
            ControlOp::While | ControlOp::ForEach => res.push_str(" {"),
            ControlOp::If | ControlOp::IfElse => res.push_str(" then"),
            _ => {}
        }
    }

    if let Some(body) = &cf.body {
        res.push('\n');
        res.push(' ');
        res.push_str(&indent_block_list(body, scratchblocks));
    }

    if let Some(else_body) = &cf.else_body {
        res.push_str("\nelse\n");
        res.push_str(&indent_block_list(else_body, scratchblocks));
    }

    if scratchblocks {
        match cf.op {
            ControlOp::While | ControlOp::ForEach => res.push_str("\n}@loopArrow::control"),
            _ => res.push_str("\nend"),
        }
    }

    res
}

impl Project {
    pub fn stringify(&self, scratchblocks: bool) -> String {
        if scratchblocks {
            let mut res = SCRATCHBLOCKS_MESSAGE.to_string();
            if !self.code.is_empty() {
                res.push_str("\n\n");
                res.push_str(
                    &self
                        .code
                        .iter()
                        .map(|bl| stringify_block_list(bl, true))
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                );
            }
            res.push('\n');
            res
        } else {
            self
                .code
                .iter()
                .map(|bl| stringify_block_list(bl, false))
                .collect::<Vec<_>>()
                .join("\n\n")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_scratch_blocks_str() {
        assert_eq!(escape_scratch_blocks_str("a::b"), r"a\:\:b");
        assert_eq!(escape_scratch_blocks_str("a (b)"), r"a \(b\)");
        assert_eq!(escape_scratch_blocks_str("x v"), r"x \v");
    }

    #[test]
    fn test_known_to_scratch_blocks() {
        assert_eq!(known_to_scratch_blocks(&KnownVal::Num(42.0), false), "(42)");
        assert_eq!(known_to_scratch_blocks(&KnownVal::Str("hello".to_string()), false), "[hello]");
        assert_eq!(known_to_scratch_blocks(&KnownVal::Str("hello".to_string()), true), "[hello v]");
        assert_eq!(known_to_scratch_blocks(&KnownVal::Bool(true), false), "<true::extension>");
    }

    #[test]
    fn test_stringify_simple_blocks() {
        let bl = BlockList::from_blocks(vec![
            Block::OnStartFlag,
            Block::Say { value: Value::Known(KnownVal::Str("Hello!".to_string())) },
        ]);
        let text = stringify_block_list(&bl, false);
        assert!(text.contains("when green flag clicked"));
        assert!(text.contains("say \"Hello!\""));
    }

    #[test]
    fn test_stringify_scratchblocks() {
        let bl = BlockList::from_blocks(vec![
            Block::OnStartFlag,
            Block::Say { value: Value::Known(KnownVal::Str("Hello!".to_string())) },
        ]);
        let text = stringify_block_list(&bl, true);
        assert!(text.contains("when green flag clicked"));
        assert!(text.contains("say [Hello!]"));
    }
}
