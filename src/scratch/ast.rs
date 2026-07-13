use std::collections::HashMap;

use serde_json::Value as JsonValue;

pub type Id = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Project3,
    Sprite3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScratchCast {
    ToStr,
    ToNum,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScratchConfig {
    pub minify: bool,
    pub minify_break_glow: bool,
    pub hide_blocks: bool,
    pub allow_hacked_blocks: bool,
    pub use_hex_if_smaller: bool,
}

impl Default for ScratchConfig {
    fn default() -> Self {
        ScratchConfig {
            minify: true,
            minify_break_glow: false,
            hide_blocks: false,
            allow_hacked_blocks: true,
            use_hex_if_smaller: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockMeta {
    pub parent: Option<Id>,
    pub next: Option<Id>,
    pub shadow: bool,
    pub x: i32,
    pub y: i32,
}

impl BlockMeta {
    pub fn new(parent: Option<Id>, next: Option<Id>) -> Self {
        BlockMeta {
            parent,
            next,
            shadow: false,
            x: 0,
            y: 0,
        }
    }

    pub fn new_shadow(parent: Option<Id>, next: Option<Id>) -> Self {
        BlockMeta {
            parent,
            next,
            shadow: true,
            x: 0,
            y: 0,
        }
    }

    pub fn add_raw_meta(&self, metaless: &mut HashMap<String, JsonValue>, cfg: &ScratchConfig) {
        if !cfg.minify_break_glow {
            if let Some(ref p) = self.parent {
                metaless.insert("parent".to_string(), JsonValue::String(p.clone()));
            } else {
                metaless.insert("parent".to_string(), JsonValue::Null);
            }
        }

        if self.parent.is_none() || !cfg.minify {
            metaless.insert("topLevel".to_string(), JsonValue::Bool(self.parent.is_none()));
        }

        if self.next.is_some() || !cfg.minify {
            match &self.next {
                Some(n) => metaless.insert("next".to_string(), JsonValue::String(n.clone())),
                None => metaless.insert("next".to_string(), JsonValue::Null),
            };
        }

        if self.shadow || !cfg.minify {
            metaless.insert("shadow".to_string(), JsonValue::Bool(self.shadow));
        }

        if (self.x != 0 || !cfg.minify) && !cfg.hide_blocks {
            metaless.insert("x".to_string(), JsonValue::Number(self.x.into()));
        }

        if (self.y != 0 || !cfg.minify) && !cfg.hide_blocks {
            metaless.insert("y".to_string(), JsonValue::Number(self.y.into()));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KnownVal {
    Str(String),
    Num(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Known(KnownVal),
    KnownBool(bool),
    Op(Op),
    BoolOp(BoolOp),
    GetVar { name: String },
    GetList { name: String },
    GetOfList(GetOfList),
    GetListLength { name: String },
    GetParam { name: String },
    CostumeInfo { op: CostumeInfoOp },
    GetCounter,
    GetAnswer,
    DaysSince2000,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CostumeInfoOp {
    Name,
    Number,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    StrToFloat(Box<Value>),
    BoolToFloat(Box<Value>),
    Add(Box<Value>, Box<Value>),
    Sub(Box<Value>, Box<Value>),
    Mul(Box<Value>, Box<Value>),
    Div(Box<Value>, Box<Value>),
    Mod(Box<Value>, Box<Value>),
    Rand(Box<Value>, Box<Value>),
    Join(Box<Value>, Box<Value>),
    LetterOf(Box<Value>, Box<Value>),
    LengthOf(Box<Value>),
    Round(Box<Value>),
    Not(Box<Value>),
    Contains(Box<Value>, Box<Value>),
    Abs(Box<Value>),
    Floor(Box<Value>),
    Ceiling(Box<Value>),
    Sqrt(Box<Value>),
    Sin(Box<Value>),
    Cos(Box<Value>),
    Tan(Box<Value>),
    Asin(Box<Value>),
    Acos(Box<Value>),
    Atan(Box<Value>),
    Ln(Box<Value>),
    Log(Box<Value>),
    Exp(Box<Value>),
    Exp10(Box<Value>),
}

impl Op {
    pub fn left(&self) -> &Value {
        match self {
            Op::StrToFloat(v) | Op::BoolToFloat(v) | Op::LengthOf(v) | Op::Round(v) | Op::Not(v) |
            Op::Abs(v) | Op::Floor(v) | Op::Ceiling(v) | Op::Sqrt(v) |
            Op::Sin(v) | Op::Cos(v) | Op::Tan(v) | Op::Asin(v) | Op::Acos(v) | Op::Atan(v) |
            Op::Ln(v) | Op::Log(v) | Op::Exp(v) | Op::Exp10(v) => v,
            Op::Add(l, _) | Op::Sub(l, _) | Op::Mul(l, _) | Op::Div(l, _) |
            Op::Mod(l, _) | Op::Rand(l, _) | Op::Join(l, _) | Op::LetterOf(l, _) |
            Op::Contains(l, _) => l,
        }
    }

    pub fn right(&self) -> &Value {
        match self {
            Op::Add(_, r) | Op::Sub(_, r) | Op::Mul(_, r) | Op::Div(_, r) |
            Op::Mod(_, r) | Op::Rand(_, r) | Op::Join(_, r) | Op::LetterOf(_, r) |
            Op::Contains(_, r) => r,
            _ => &Value::Known(KnownVal::Num(0.0)),
        }
    }

    pub fn left_mut(&mut self) -> &mut Value {
        match self {
            Op::StrToFloat(v) | Op::BoolToFloat(v) | Op::LengthOf(v) | Op::Round(v) | Op::Not(v) |
            Op::Abs(v) | Op::Floor(v) | Op::Ceiling(v) | Op::Sqrt(v) |
            Op::Sin(v) | Op::Cos(v) | Op::Tan(v) | Op::Asin(v) | Op::Acos(v) | Op::Atan(v) |
            Op::Ln(v) | Op::Log(v) | Op::Exp(v) | Op::Exp10(v) => v,
            Op::Add(l, _) | Op::Sub(l, _) | Op::Mul(l, _) | Op::Div(l, _) |
            Op::Mod(l, _) | Op::Rand(l, _) | Op::Join(l, _) | Op::LetterOf(l, _) |
            Op::Contains(l, _) => l,
        }
    }

    pub fn right_mut(&mut self) -> Option<&mut Value> {
        match self {
            Op::Add(_, r) | Op::Sub(_, r) | Op::Mul(_, r) | Op::Div(_, r) |
            Op::Mod(_, r) | Op::Rand(_, r) | Op::Join(_, r) | Op::LetterOf(_, r) |
            Op::Contains(_, r) => Some(r),
            _ => None,
        }
    }

    pub fn with_values(&self, left: Value, right: Value) -> Op {
        match self {
            Op::Add(_, _) => Op::Add(Box::new(left), Box::new(right)),
            Op::Sub(_, _) => Op::Sub(Box::new(left), Box::new(right)),
            Op::Mul(_, _) => Op::Mul(Box::new(left), Box::new(right)),
            Op::Div(_, _) => Op::Div(Box::new(left), Box::new(right)),
            Op::Mod(_, _) => Op::Mod(Box::new(left), Box::new(right)),
            Op::Rand(_, _) => Op::Rand(Box::new(left), Box::new(right)),
            Op::Join(_, _) => Op::Join(Box::new(left), Box::new(right)),
            Op::LetterOf(_, _) => Op::LetterOf(Box::new(left), Box::new(right)),
            Op::Contains(_, _) => Op::Contains(Box::new(left), Box::new(right)),
            Op::StrToFloat(_) => Op::StrToFloat(Box::new(left)),
            Op::BoolToFloat(_) => Op::BoolToFloat(Box::new(left)),
            Op::LengthOf(_) => Op::LengthOf(Box::new(left)),
            Op::Round(_) => Op::Round(Box::new(left)),
            Op::Not(_) => Op::Not(Box::new(left)),
            Op::Abs(_) => Op::Abs(Box::new(left)),
            Op::Floor(_) => Op::Floor(Box::new(left)),
            Op::Ceiling(_) => Op::Ceiling(Box::new(left)),
            Op::Sqrt(_) => Op::Sqrt(Box::new(left)),
            Op::Sin(_) => Op::Sin(Box::new(left)),
            Op::Cos(_) => Op::Cos(Box::new(left)),
            Op::Tan(_) => Op::Tan(Box::new(left)),
            Op::Asin(_) => Op::Asin(Box::new(left)),
            Op::Acos(_) => Op::Acos(Box::new(left)),
            Op::Atan(_) => Op::Atan(Box::new(left)),
            Op::Ln(_) => Op::Ln(Box::new(left)),
            Op::Log(_) => Op::Log(Box::new(left)),
            Op::Exp(_) => Op::Exp(Box::new(left)),
            Op::Exp10(_) => Op::Exp10(Box::new(left)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BoolOp {
    And(Box<Value>, Box<Value>),
    Or(Box<Value>, Box<Value>),
    Eq(Box<Value>, Box<Value>),
    Lt(Box<Value>, Box<Value>),
    Gt(Box<Value>, Box<Value>),
    Not(Box<Value>),
}

impl BoolOp {
    pub fn left(&self) -> &Value {
        match self {
            BoolOp::And(l, _) | BoolOp::Or(l, _) | BoolOp::Eq(l, _) |
            BoolOp::Lt(l, _) | BoolOp::Gt(l, _) => l,
            BoolOp::Not(v) => v,
        }
    }

    pub fn right(&self) -> &Value {
        match self {
            BoolOp::And(_, r) | BoolOp::Or(_, r) | BoolOp::Eq(_, r) |
            BoolOp::Lt(_, r) | BoolOp::Gt(_, r) => r,
            BoolOp::Not(_) => &Value::Known(KnownVal::Num(0.0)),
        }
    }

    pub fn left_mut(&mut self) -> &mut Value {
        match self {
            BoolOp::And(l, _) | BoolOp::Or(l, _) | BoolOp::Eq(l, _) |
            BoolOp::Lt(l, _) | BoolOp::Gt(l, _) => l,
            BoolOp::Not(v) => v,
        }
    }

    pub fn right_mut(&mut self) -> Option<&mut Value> {
        match self {
            BoolOp::And(_, r) | BoolOp::Or(_, r) | BoolOp::Eq(_, r) |
            BoolOp::Lt(_, r) | BoolOp::Gt(_, r) => Some(r),
            BoolOp::Not(_) => None,
        }
    }

    pub fn with_values(&self, left: Value, right: Value) -> BoolOp {
        match self {
            BoolOp::And(_, _) => BoolOp::And(Box::new(left), Box::new(right)),
            BoolOp::Or(_, _) => BoolOp::Or(Box::new(left), Box::new(right)),
            BoolOp::Eq(_, _) => BoolOp::Eq(Box::new(left), Box::new(right)),
            BoolOp::Lt(_, _) => BoolOp::Lt(Box::new(left), Box::new(right)),
            BoolOp::Gt(_, _) => BoolOp::Gt(Box::new(left), Box::new(right)),
            BoolOp::Not(_) => BoolOp::Not(Box::new(left)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListOp {
    AtIndex,
    IndexOf,
    LengthOf,
    Contains,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetOfList {
    pub op: ListOp,
    pub name: String,
    pub value: Box<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Say { value: Value },
    SwitchCostume { value: Value },
    EditVolume { op: VolumeOp, value: Value },
    Broadcast { value: Value, wait: bool },
    OnBroadcast { name: String },
    OnStartFlag,
    ControlFlow(ControlFlow),
    StopScript(StopOption),
    EditCounter(CounterOp),
    Wait { value: Value },
    Ask { value: Value, var_name: Option<String> },
    EditVar(EditVarData),
    EditList(EditListData),
    ProcedureDef(ProcedureDefData),
    ProcedureCall(ProcedureCallData),
    RawBlock(HashMap<String, JsonValue>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolumeOp {
    Set,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopOption {
    All,
    This,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CounterOp {
    Increment,
    Decrement,
    Reset,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlFlow {
    pub op: ControlOp,
    pub condition: Option<Value>,
    pub var: Option<String>,
    pub body: Option<BlockList>,
    pub else_body: Option<BlockList>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlOp {
    If,
    IfElse,
    RepTimes,
    Until,
    While,
    Forever,
    ForEach,
}

impl ControlFlow {
    pub fn validate(&self) -> Result<(), String> {
        match self.op {
            ControlOp::If | ControlOp::IfElse => {
                if self.condition.is_none() {
                    return Err(format!("{:?} requires a condition", self.op));
                }
            }
            ControlOp::RepTimes => {
                if self.condition.is_none() {
                    return Err("RepTimes requires a condition (repeat count)".to_string());
                }
                if self.body.is_none() {
                    return Err("RepTimes requires a body".to_string());
                }
            }
            ControlOp::ForEach => {
                if self.var.is_none() {
                    return Err("ForEach requires a var".to_string());
                }
                if self.body.is_none() {
                    return Err("ForEach requires a body".to_string());
                }
            }
            ControlOp::Until | ControlOp::While => {
                if self.condition.is_none() {
                    return Err(format!("{:?} requires a condition", self.op));
                }
                if self.body.is_none() {
                    return Err(format!("{:?} requires a body", self.op));
                }
            }
            ControlOp::Forever => {
                if self.body.is_none() {
                    return Err("Forever requires a body".to_string());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditVarData {
    pub op: VarOp,
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VarOp {
    Set,
    Change,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditListData {
    pub op: ListEditOp,
    pub name: String,
    pub value: Option<Value>,
    pub index: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListEditOp {
    AddTo,
    ReplaceAt,
    InsertAt,
    DeleteAt,
    DeleteAll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcedureDefData {
    pub name: String,
    pub params: Vec<String>,
    pub warp: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcedureCallData {
    pub name: String,
    pub args: Vec<Value>,
    pub run_without_refresh: bool,
}

impl Block {
    pub fn is_start(&self) -> bool {
        matches!(
            self,
            Block::OnBroadcast { .. } | Block::OnStartFlag | Block::ProcedureDef(_)
        )
    }

    pub fn is_end(&self) -> bool {
        matches!(self, Block::StopScript(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockList {
    pub blocks: Vec<Block>,
    pub end: bool,
}

impl BlockList {
    pub fn new() -> Self {
        BlockList {
            blocks: Vec::new(),
            end: false,
        }
    }

    pub fn from_block(block: Block) -> Self {
        let end = block.is_end();
        BlockList {
            blocks: vec![block],
            end,
        }
    }

    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        let end = blocks.iter().any(|b| b.is_end());
        BlockList { blocks, end }
    }

    pub fn add(&mut self, other: BlockList) {
        if self.end && !other.blocks.is_empty() {
            panic!("Cannot add blocks after ending block");
        }
        self.blocks.extend(other.blocks);
        self.end = self.end || other.end;
    }

    pub fn add_block(&mut self, block: Block) {
        if self.end {
            panic!("Cannot add block after ending block");
        }
        self.end = block.is_end();
        self.blocks.push(block);
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }
}

impl Default for BlockList {
    fn default() -> Self {
        Self::new()
    }
}

pub const SHORT_OP_TO_OPCODE: &[(&str, &str)] = &[
    ("if", "control_if"),
    ("if_else", "control_if_else"),
    ("reptimes", "control_repeat"),
    ("until", "control_repeat_until"),
    ("while", "control_while"),
    ("forever", "control_forever"),
    ("for_each", "control_for_each"),
    ("set", "data_setvariableto"),
    ("change", "data_changevariableby"),
    ("addto", "data_addtolist"),
    ("replaceat", "data_replaceitemoflist"),
    ("insertat", "data_insertatlist"),
    ("deleteat", "data_deleteoflist"),
    ("deleteall", "data_deletealloflist"),
    ("atindex", "data_itemoflist"),
    ("indexof", "data_itemnumoflist"),
    ("str_to_float", "operator_add"),
    ("add", "operator_add"),
    ("sub", "operator_subtract"),
    ("mul", "operator_multiply"),
    ("div", "operator_divide"),
    ("mod", "operator_mod"),
    ("rand", "operator_random"),
    ("join", "operator_join"),
    ("letter_of", "operator_letter_of"),
    ("length_of", "operator_length"),
    ("round", "operator_round"),
    ("bool_to_float", "operator_round"),
    ("not", "operator_not"),
    ("and", "operator_and"),
    ("or", "operator_or"),
    ("=", "operator_equals"),
    ("<", "operator_lt"),
    (">", "operator_gt"),
    ("contains", "operator_contains"),
];

pub fn opcode_from_short_op(short: &str) -> Option<&'static str> {
    SHORT_OP_TO_OPCODE
        .iter()
        .find(|(k, _)| *k == short)
        .map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_is_start() {
        assert!(Block::OnStartFlag.is_start());
        assert!(Block::OnBroadcast { name: "msg".to_string() }.is_start());
        assert!(!Block::Say { value: Value::Known(KnownVal::Num(0.0)) }.is_start());
    }

    #[test]
    fn test_block_is_end() {
        assert!(Block::StopScript(StopOption::All).is_end());
        assert!(!Block::OnStartFlag.is_end());
    }

    #[test]
    fn test_block_list() {
        let mut list = BlockList::new();
        list.add_block(Block::Say { value: Value::Known(KnownVal::Num(1.0)) });
        assert_eq!(list.len(), 1);
        assert!(!list.end);
        list.add_block(Block::StopScript(StopOption::This));
        assert!(list.end);
    }

    #[test]
    fn test_opcode_from_short_op() {
        assert_eq!(opcode_from_short_op("add"), Some("operator_add"));
        assert_eq!(opcode_from_short_op("set"), Some("data_setvariableto"));
        assert_eq!(opcode_from_short_op("if"), Some("control_if"));
        assert_eq!(opcode_from_short_op("nonexistent"), None);
    }

    #[test]
    fn test_scratch_config_default() {
        let cfg = ScratchConfig::default();
        assert!(cfg.minify);
        assert!(!cfg.minify_break_glow);
        assert!(!cfg.hide_blocks);
        assert!(cfg.allow_hacked_blocks);
        assert!(!cfg.use_hex_if_smaller);
    }
}