use std::fmt;

use super::types::Type;
use super::values::{LabelVal, ResultLocalVar, Value};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnaryOpcode {
    FNeg,
}

impl fmt::Display for UnaryOpcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnaryOpcode::FNeg => write!(f, "fneg"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOpcode {
    Add,
    FAdd,
    Sub,
    FSub,
    Mul,
    FMul,
    UDiv,
    SDiv,
    FDiv,
    URem,
    SRem,
    FRem,
    Shl,
    LShr,
    AShr,
    And,
    Or,
    Xor,
}

impl BinaryOpcode {
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "add" => Some(BinaryOpcode::Add),
            "fadd" => Some(BinaryOpcode::FAdd),
            "sub" => Some(BinaryOpcode::Sub),
            "fsub" => Some(BinaryOpcode::FSub),
            "mul" => Some(BinaryOpcode::Mul),
            "fmul" => Some(BinaryOpcode::FMul),
            "udiv" => Some(BinaryOpcode::UDiv),
            "sdiv" => Some(BinaryOpcode::SDiv),
            "fdiv" => Some(BinaryOpcode::FDiv),
            "urem" => Some(BinaryOpcode::URem),
            "srem" => Some(BinaryOpcode::SRem),
            "frem" => Some(BinaryOpcode::FRem),
            "shl" => Some(BinaryOpcode::Shl),
            "lshr" => Some(BinaryOpcode::LShr),
            "ashr" => Some(BinaryOpcode::AShr),
            "and" => Some(BinaryOpcode::And),
            "or" => Some(BinaryOpcode::Or),
            "xor" => Some(BinaryOpcode::Xor),
            _ => None,
        }
    }
}

impl fmt::Display for BinaryOpcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryOpcode::Add => write!(f, "add"),
            BinaryOpcode::FAdd => write!(f, "fadd"),
            BinaryOpcode::Sub => write!(f, "sub"),
            BinaryOpcode::FSub => write!(f, "fsub"),
            BinaryOpcode::Mul => write!(f, "mul"),
            BinaryOpcode::FMul => write!(f, "fmul"),
            BinaryOpcode::UDiv => write!(f, "udiv"),
            BinaryOpcode::SDiv => write!(f, "sdiv"),
            BinaryOpcode::FDiv => write!(f, "fdiv"),
            BinaryOpcode::URem => write!(f, "urem"),
            BinaryOpcode::SRem => write!(f, "srem"),
            BinaryOpcode::FRem => write!(f, "frem"),
            BinaryOpcode::Shl => write!(f, "shl"),
            BinaryOpcode::LShr => write!(f, "lshr"),
            BinaryOpcode::AShr => write!(f, "ashr"),
            BinaryOpcode::And => write!(f, "and"),
            BinaryOpcode::Or => write!(f, "or"),
            BinaryOpcode::Xor => write!(f, "xor"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConvOpcode {
    Trunc,
    ZExt,
    SExt,
    FPTrunc,
    FPExt,
    FPToUI,
    FPToSI,
    UIToFP,
    SIToFP,
    PtrToInt,
    PtrToAddr,
    IntToPtr,
    BitCast,
    AddrSpaceCast,
}

impl ConvOpcode {
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "trunc" => Some(ConvOpcode::Trunc),
            "zext" => Some(ConvOpcode::ZExt),
            "sext" => Some(ConvOpcode::SExt),
            "fptrunc" => Some(ConvOpcode::FPTrunc),
            "fpext" => Some(ConvOpcode::FPExt),
            "fptoui" => Some(ConvOpcode::FPToUI),
            "fptosi" => Some(ConvOpcode::FPToSI),
            "uitofp" => Some(ConvOpcode::UIToFP),
            "sitofp" => Some(ConvOpcode::SIToFP),
            "ptrtoint" => Some(ConvOpcode::PtrToInt),
            "ptrtoaddr" => Some(ConvOpcode::PtrToAddr),
            "inttoptr" => Some(ConvOpcode::IntToPtr),
            "bitcast" => Some(ConvOpcode::BitCast),
            "addrspacecast" => Some(ConvOpcode::AddrSpaceCast),
            _ => None,
        }
    }
}

impl fmt::Display for ConvOpcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvOpcode::Trunc => write!(f, "trunc"),
            ConvOpcode::ZExt => write!(f, "zext"),
            ConvOpcode::SExt => write!(f, "sext"),
            ConvOpcode::FPTrunc => write!(f, "fptrunc"),
            ConvOpcode::FPExt => write!(f, "fpext"),
            ConvOpcode::FPToUI => write!(f, "fptoui"),
            ConvOpcode::FPToSI => write!(f, "fptosi"),
            ConvOpcode::UIToFP => write!(f, "uitofp"),
            ConvOpcode::SIToFP => write!(f, "sitofp"),
            ConvOpcode::PtrToInt => write!(f, "ptrtoint"),
            ConvOpcode::PtrToAddr => write!(f, "ptrtoaddr"),
            ConvOpcode::IntToPtr => write!(f, "inttoptr"),
            ConvOpcode::BitCast => write!(f, "bitcast"),
            ConvOpcode::AddrSpaceCast => write!(f, "addrspacecast"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ICmpCond {
    Eq,
    Ne,
    Ugt,
    Uge,
    Ult,
    Ule,
    Sgt,
    Sge,
    Slt,
    Sle,
}

impl ICmpCond {
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "eq" => Some(ICmpCond::Eq),
            "ne" => Some(ICmpCond::Ne),
            "ugt" => Some(ICmpCond::Ugt),
            "uge" => Some(ICmpCond::Uge),
            "ult" => Some(ICmpCond::Ult),
            "ule" => Some(ICmpCond::Ule),
            "sgt" => Some(ICmpCond::Sgt),
            "sge" => Some(ICmpCond::Sge),
            "slt" => Some(ICmpCond::Slt),
            "sle" => Some(ICmpCond::Sle),
            _ => None,
        }
    }
}

impl fmt::Display for ICmpCond {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ICmpCond::Eq => write!(f, "eq"),
            ICmpCond::Ne => write!(f, "ne"),
            ICmpCond::Ugt => write!(f, "ugt"),
            ICmpCond::Uge => write!(f, "uge"),
            ICmpCond::Ult => write!(f, "ult"),
            ICmpCond::Ule => write!(f, "ule"),
            ICmpCond::Sgt => write!(f, "sgt"),
            ICmpCond::Sge => write!(f, "sge"),
            ICmpCond::Slt => write!(f, "slt"),
            ICmpCond::Sle => write!(f, "sle"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FCmpCond {
    FalseCond,
    Oeq,
    Ogt,
    Oge,
    Olt,
    Ole,
    One,
    Ord,
    Ueq,
    Ugt,
    Uge,
    Ult,
    Ule,
    Une,
    Uno,
    TrueCond,
}

impl FCmpCond {
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "false" => Some(FCmpCond::FalseCond),
            "oeq" => Some(FCmpCond::Oeq),
            "ogt" => Some(FCmpCond::Ogt),
            "oge" => Some(FCmpCond::Oge),
            "olt" => Some(FCmpCond::Olt),
            "ole" => Some(FCmpCond::Ole),
            "one" => Some(FCmpCond::One),
            "ord" => Some(FCmpCond::Ord),
            "ueq" => Some(FCmpCond::Ueq),
            "ugt" => Some(FCmpCond::Ugt),
            "uge" => Some(FCmpCond::Uge),
            "ult" => Some(FCmpCond::Ult),
            "ule" => Some(FCmpCond::Ule),
            "une" => Some(FCmpCond::Une),
            "uno" => Some(FCmpCond::Uno),
            "true" => Some(FCmpCond::TrueCond),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallTailKind {
    NoTail,
    Tail,
    MustTail,
}

impl CallTailKind {
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "notail" => Some(CallTailKind::NoTail),
            "tail" => Some(CallTailKind::Tail),
            "musttail" => Some(CallTailKind::MustTail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Intrinsic {
    VaStart,
    VaEnd,
    VaCopy,
    Abs,
    SMax,
    SMin,
    UMax,
    UMin,
    MemCpy,
    MemCpyInline,
    MemMove,
    FAbs,
    FShl,
    FShr,
    UAddWithOverflow,
    USubWithOverflow,
    UMulWithOverflow,
    FMulAdd,
    LifetimeStart,
    LifetimeEnd,
    NoAliasScopeDecl,
    Expect,
    ExpectWithProbability,
    Assume,
    PtrMask,
}

impl Intrinsic {
    pub fn llvm_name(&self) -> &'static str {
        match self {
            Intrinsic::VaStart => "llvm.va_start",
            Intrinsic::VaEnd => "llvm.va_end",
            Intrinsic::VaCopy => "llvm.va_copy",
            Intrinsic::Abs => "llvm.abs",
            Intrinsic::SMax => "llvm.smax",
            Intrinsic::SMin => "llvm.smin",
            Intrinsic::UMax => "llvm.umax",
            Intrinsic::UMin => "llvm.umin",
            Intrinsic::MemCpy => "llvm.memcpy",
            Intrinsic::MemCpyInline => "llvm.memcpy.inline",
            Intrinsic::MemMove => "llvm.memmove",
            Intrinsic::FAbs => "llvm.fabs",
            Intrinsic::FShl => "llvm.fshl",
            Intrinsic::FShr => "llvm.fshr",
            Intrinsic::UAddWithOverflow => "llvm.uadd.with.overflow",
            Intrinsic::USubWithOverflow => "llvm.usub.with.overflow",
            Intrinsic::UMulWithOverflow => "llvm.umul.with.overflow",
            Intrinsic::FMulAdd => "llvm.fmuladd",
            Intrinsic::LifetimeStart => "llvm.lifetime.start",
            Intrinsic::LifetimeEnd => "llvm.lifetime.end",
            Intrinsic::NoAliasScopeDecl => "llvm.experimental.noalias.scope.decl",
            Intrinsic::Expect => "llvm.expect",
            Intrinsic::ExpectWithProbability => "llvm.expect.with.probability",
            Intrinsic::Assume => "llvm.assume",
            Intrinsic::PtrMask => "llvm.ptrmask",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        if !name.starts_with("llvm.") {
            return None;
        }
        let candidates: Vec<(Intrinsic, &'static str)> = vec![
            (Intrinsic::VaStart, "llvm.va_start"),
            (Intrinsic::VaEnd, "llvm.va_end"),
            (Intrinsic::VaCopy, "llvm.va_copy"),
            (Intrinsic::Abs, "llvm.abs"),
            (Intrinsic::SMax, "llvm.smax"),
            (Intrinsic::SMin, "llvm.smin"),
            (Intrinsic::UMax, "llvm.umax"),
            (Intrinsic::UMin, "llvm.umin"),
            (Intrinsic::MemCpy, "llvm.memcpy"),
            (Intrinsic::MemCpyInline, "llvm.memcpy.inline"),
            (Intrinsic::MemMove, "llvm.memmove"),
            (Intrinsic::FAbs, "llvm.fabs"),
            (Intrinsic::FShl, "llvm.fshl"),
            (Intrinsic::FShr, "llvm.fshr"),
            (Intrinsic::UAddWithOverflow, "llvm.uadd.with.overflow"),
            (Intrinsic::USubWithOverflow, "llvm.usub.with.overflow"),
            (Intrinsic::UMulWithOverflow, "llvm.umul.with.overflow"),
            (Intrinsic::FMulAdd, "llvm.fmuladd"),
            (Intrinsic::LifetimeStart, "llvm.lifetime.start"),
            (Intrinsic::LifetimeEnd, "llvm.lifetime.end"),
            (Intrinsic::NoAliasScopeDecl, "llvm.experimental.noalias.scope.decl"),
            (Intrinsic::Expect, "llvm.expect"),
            (Intrinsic::ExpectWithProbability, "llvm.expect.with.probability"),
            (Intrinsic::Assume, "llvm.assume"),
            (Intrinsic::PtrMask, "llvm.ptrmask"),
        ];
        let matching: Vec<_> = candidates
            .into_iter()
            .filter(|(_, n)| name.starts_with(n))
            .collect();
        if matching.is_empty() {
            return None;
        }
        matching
            .into_iter()
            .max_by_key(|(_, n)| n.len())
            .map(|(i, _)| i)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Instr {
    Ret(Ret),
    UncondBr(UncondBr),
    CondBr(CondBr),
    Switch(Switch),
    Unreachable,
    UnaryOp(UnaryOp),
    BinaryOp(BinaryOp),
    ExtractElement(ExtractElement),
    InsertElement(InsertElement),
    ShuffleVector(ShuffleVector),
    ExtractValue(ExtractValue),
    InsertValue(InsertValue),
    Alloca(Alloca),
    Load(Load),
    Store(Store),
    GetElementPtr(GetElementPtr),
    Conversion(Conversion),
    ICmp(ICmp),
    FCmp(FCmp),
    Phi(Phi),
    Select(Select),
    Freeze(Freeze),
    Call(Call),
    VaArg(VaArg),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ret {
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UncondBr {
    pub branch: LabelVal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CondBr {
    pub cond: Value,
    pub branch_true: LabelVal,
    pub branch_false: LabelVal,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Switch {
    pub cond: Value,
    pub branch_default: LabelVal,
    pub branch_table: Vec<(Value, LabelVal)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnaryOp {
    pub result: ResultLocalVar,
    pub opcode: UnaryOpcode,
    pub operand: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BinaryOp {
    pub result: ResultLocalVar,
    pub opcode: BinaryOpcode,
    pub left: Value,
    pub right: Value,
    pub is_nuw: bool,
    pub is_nsw: bool,
    pub is_exact: bool,
    pub is_disjoint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractElement {
    pub result: ResultLocalVar,
    pub agg: Value,
    pub index: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InsertElement {
    pub result: ResultLocalVar,
    pub agg: Value,
    pub item: Value,
    pub index: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShuffleVector {
    pub result: ResultLocalVar,
    pub fst_vector: Value,
    pub snd_vector: Value,
    pub mask_vector: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtractValue {
    pub result: ResultLocalVar,
    pub agg: Value,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InsertValue {
    pub result: ResultLocalVar,
    pub agg: Value,
    pub element: Value,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Alloca {
    pub result: ResultLocalVar,
    pub allocated_type: Type,
    pub num_elements: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Load {
    pub result: ResultLocalVar,
    pub loaded_type: Type,
    pub address: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Store {
    pub value: Value,
    pub address: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GetElementPtr {
    pub result: ResultLocalVar,
    pub base_ptr_type: Type,
    pub base_ptr: Value,
    pub indices: Vec<Value>,
    pub is_inbounds: bool,
    pub is_nusw: bool,
    pub is_nuw: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Conversion {
    pub result: ResultLocalVar,
    pub opcode: ConvOpcode,
    pub value: Value,
    pub res_type: Type,
    pub is_nuw: bool,
    pub is_nsw: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ICmp {
    pub result: ResultLocalVar,
    pub cond: ICmpCond,
    pub left: Value,
    pub right: Value,
    pub is_samesign: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FCmp {
    pub result: ResultLocalVar,
    pub cond: FCmpCond,
    pub left: Value,
    pub right: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Phi {
    pub result: ResultLocalVar,
    pub incoming: Vec<(Value, LabelVal)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Select {
    pub result: ResultLocalVar,
    pub cond: Value,
    pub true_value: Value,
    pub false_value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Freeze {
    pub result: ResultLocalVar,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Call {
    pub result: Option<ResultLocalVar>,
    pub func: Value,
    pub return_type: Type,
    pub args: Vec<Value>,
    pub params: Vec<Type>,
    pub variadic: bool,
    pub tail_kind: CallTailKind,
    pub intrinsic: Option<Intrinsic>,
    pub callees: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VaArg {
    pub result: ResultLocalVar,
    pub arglist: Value,
    pub argty: Type,
}

impl Instr {
    pub fn result(&self) -> Option<&ResultLocalVar> {
        match self {
            Instr::Ret(_) | Instr::Store(_) | Instr::Unreachable => None,
            Instr::Call(c) => c.result.as_ref(),
            Instr::UncondBr(_) | Instr::CondBr(_) | Instr::Switch(_) => None,
            Instr::UnaryOp(i) => Some(&i.result),
            Instr::BinaryOp(i) => Some(&i.result),
            Instr::ExtractElement(i) => Some(&i.result),
            Instr::InsertElement(i) => Some(&i.result),
            Instr::ShuffleVector(i) => Some(&i.result),
            Instr::ExtractValue(i) => Some(&i.result),
            Instr::InsertValue(i) => Some(&i.result),
            Instr::Alloca(i) => Some(&i.result),
            Instr::Load(i) => Some(&i.result),
            Instr::GetElementPtr(i) => Some(&i.result),
            Instr::Conversion(i) => Some(&i.result),
            Instr::ICmp(i) => Some(&i.result),
            Instr::FCmp(i) => Some(&i.result),
            Instr::Phi(i) => Some(&i.result),
            Instr::Select(i) => Some(&i.result),
            Instr::Freeze(i) => Some(&i.result),
            Instr::VaArg(i) => Some(&i.result),
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Instr::Ret(_)
                | Instr::UncondBr(_)
                | Instr::CondBr(_)
                | Instr::Switch(_)
                | Instr::Unreachable
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    pub label: String,
    pub instrs: Vec<Instr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub return_type: Type,
    pub params: Vec<super::values::ArgumentVal>,
    pub variadic: bool,
    pub intrinsic: Option<Intrinsic>,
    pub blocks: indexmap::IndexMap<String, Block>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlobalVar {
    pub name: String,
    pub type_: Type,
    pub is_constant: bool,
    pub init: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: String,
    pub functions: indexmap::IndexMap<String, Function>,
    pub global_vars: indexmap::IndexMap<String, GlobalVar>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::values::{KnownIntVal, LabelVal};

    #[test]
    fn test_binary_opcode_from_str() {
        assert_eq!(BinaryOpcode::try_from_str("add"), Some(BinaryOpcode::Add));
        assert_eq!(BinaryOpcode::try_from_str("xor"), Some(BinaryOpcode::Xor));
        assert_eq!(BinaryOpcode::try_from_str("invalid"), None);
    }

    #[test]
    fn test_conv_opcode_from_str() {
        assert_eq!(ConvOpcode::try_from_str("zext"), Some(ConvOpcode::ZExt));
        assert_eq!(ConvOpcode::try_from_str("bitcast"), Some(ConvOpcode::BitCast));
        assert_eq!(ConvOpcode::try_from_str("invalid"), None);
    }

    #[test]
    fn test_icmp_cond_from_str() {
        assert_eq!(ICmpCond::try_from_str("eq"), Some(ICmpCond::Eq));
        assert_eq!(ICmpCond::try_from_str("sle"), Some(ICmpCond::Sle));
        assert_eq!(ICmpCond::try_from_str("invalid"), None);
    }

    #[test]
    fn test_fcmp_cond_from_str() {
        assert_eq!(FCmpCond::try_from_str("oeq"), Some(FCmpCond::Oeq));
        assert_eq!(FCmpCond::try_from_str("false"), Some(FCmpCond::FalseCond));
        assert_eq!(FCmpCond::try_from_str("true"), Some(FCmpCond::TrueCond));
    }

    #[test]
    fn test_intrinsic_from_name() {
        assert_eq!(
            Intrinsic::from_name("llvm.va_start"),
            Some(Intrinsic::VaStart)
        );
        assert_eq!(
            Intrinsic::from_name("llvm.memcpy.inline"),
            Some(Intrinsic::MemCpyInline)
        );
        assert_eq!(Intrinsic::from_name("printf"), None);
    }

    #[test]
    fn test_intrinsic_memcpy_inline_preferred_over_memcpy() {
        let result = Intrinsic::from_name("llvm.memcpy.inline");
        assert_eq!(result, Some(Intrinsic::MemCpyInline));
    }

    #[test]
    fn test_lifetime_end_p0() {
        assert_eq!(Intrinsic::from_name("llvm.lifetime.end.p0"), Some(Intrinsic::LifetimeEnd));
        assert_eq!(Intrinsic::from_name("llvm.lifetime.start.p0"), Some(Intrinsic::LifetimeStart));
        assert_eq!(Intrinsic::from_name("llvm.va_end.p0"), Some(Intrinsic::VaEnd));
    }

    #[test]
    fn test_instr_is_terminator() {
        use crate::ir::types::*;
        let ret = Instr::Ret(Ret { value: None });
        assert!(ret.is_terminator());

        let br = Instr::UncondBr(UncondBr {
            branch: LabelVal::new(Type::Label, "exit"),
        });
        assert!(br.is_terminator());

        let binop = Instr::BinaryOp(BinaryOp {
            result: ResultLocalVar::new("tmp"),
            opcode: BinaryOpcode::Add,
            left: Value::KnownInt(KnownIntVal::new(Type::integer(32), 1, 32)),
            right: Value::KnownInt(KnownIntVal::new(Type::integer(32), 2, 32)),
            is_nuw: false,
            is_nsw: false,
            is_exact: false,
            is_disjoint: false,
        });
        assert!(!binop.is_terminator());
    }

    #[test]
    fn test_instr_result() {
        use crate::ir::types::*;
        let binop = Instr::BinaryOp(BinaryOp {
            result: ResultLocalVar::new("tmp"),
            opcode: BinaryOpcode::Add,
            left: Value::KnownInt(KnownIntVal::new(Type::integer(32), 1, 32)),
            right: Value::KnownInt(KnownIntVal::new(Type::integer(32), 2, 32)),
            is_nuw: false,
            is_nsw: false,
            is_exact: false,
            is_disjoint: false,
        });
        assert_eq!(binop.result().unwrap().name, "tmp");

        let store = Instr::Store(Store {
            value: Value::KnownInt(KnownIntVal::new(Type::integer(32), 0, 32)),
            address: Value::KnownInt(KnownIntVal::new(Type::integer(32), 0, 32)),
        });
        assert!(store.result().is_none());
    }

    #[test]
    fn test_binary_opcode_display() {
        assert_eq!(format!("{}", BinaryOpcode::Add), "add");
        assert_eq!(format!("{}", BinaryOpcode::Xor), "xor");
    }

    #[test]
    fn test_call_tail_kind_from_str() {
        assert_eq!(
            CallTailKind::try_from_str("notail"),
            Some(CallTailKind::NoTail)
        );
        assert_eq!(CallTailKind::try_from_str("tail"), Some(CallTailKind::Tail));
        assert_eq!(
            CallTailKind::try_from_str("musttail"),
            Some(CallTailKind::MustTail)
        );
    }
}