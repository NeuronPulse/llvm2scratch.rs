use std::hash::{Hash, Hasher};

use super::types::Type;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultLocalVar {
    pub name: String,
}

impl ResultLocalVar {
    pub fn new(name: impl Into<String>) -> Self {
        ResultLocalVar { name: name.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Argument(ArgumentVal),
    Function(FunctionVal),
    LocalVar(LocalVarVal),
    GlobalPtr(GlobalPtrVal),
    NullPtr(NullPtrVal),
    Undef(UndefVal),
    KnownInt(KnownIntVal),
    KnownFloat(KnownFloatVal),
    KnownVec(KnownVecVal),
    KnownArr(KnownArrVal),
    KnownStruct(KnownStructVal),
    Label(LabelVal),
    ConstExpr(ConstExprVal),
    Metadata(MetadataVal),
}

impl Eq for Value {}

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::Argument(v) => v.hash(state),
            Value::Function(v) => v.hash(state),
            Value::LocalVar(v) => v.hash(state),
            Value::GlobalPtr(v) => v.hash(state),
            Value::NullPtr(v) => v.hash(state),
            Value::Undef(v) => v.hash(state),
            Value::KnownInt(v) => v.hash(state),
            Value::KnownFloat(v) => {
                v.type_.hash(state);
                v.value.to_bits().hash(state);
            }
            Value::KnownVec(v) => v.hash(state),
            Value::KnownArr(v) => v.hash(state),
            Value::KnownStruct(v) => v.hash(state),
            Value::Label(v) => v.hash(state),
            Value::ConstExpr(v) => v.hash(state),
            Value::Metadata(v) => v.hash(state),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArgumentVal {
    pub type_: Type,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionVal {
    pub type_: Type,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalVarVal {
    pub type_: Type,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GlobalPtrVal {
    pub type_: Type,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NullPtrVal {
    pub type_: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UndefVal {
    pub type_: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownIntVal {
    pub type_: Type,
    pub value: u128,
    pub width: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KnownFloatVal {
    pub type_: Type,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownVecVal {
    pub type_: Type,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownArrVal {
    pub type_: Type,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownStructVal {
    pub type_: Type,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelVal {
    pub type_: Type,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConstExprVal {
    pub type_: Type,
    pub expr: ConstExpr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetadataVal {
    pub type_: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstExpr {
    Conversion(Box<super::instructions::Conversion>),
    GetElementPtr(Box<super::instructions::GetElementPtr>),
    ExtractElement(Box<super::instructions::ExtractElement>),
    InsertElement(Box<super::instructions::InsertElement>),
    ShuffleVector(Box<super::instructions::ShuffleVector>),
    BinaryOp(Box<super::instructions::BinaryOp>),
}

impl Value {
    pub fn type_(&self) -> &Type {
        match self {
            Value::Argument(v) => &v.type_,
            Value::Function(v) => &v.type_,
            Value::LocalVar(v) => &v.type_,
            Value::GlobalPtr(v) => &v.type_,
            Value::NullPtr(v) => &v.type_,
            Value::Undef(v) => &v.type_,
            Value::KnownInt(v) => &v.type_,
            Value::KnownFloat(v) => &v.type_,
            Value::KnownVec(v) => &v.type_,
            Value::KnownArr(v) => &v.type_,
            Value::KnownStruct(v) => &v.type_,
            Value::Label(v) => &v.type_,
            Value::ConstExpr(v) => &v.type_,
            Value::Metadata(v) => &v.type_,
        }
    }

    pub fn is_known(&self) -> bool {
        matches!(
            self,
            Value::Function(_)
                | Value::GlobalPtr(_)
                | Value::NullPtr(_)
                | Value::Undef(_)
                | Value::KnownInt(_)
                | Value::KnownFloat(_)
                | Value::KnownVec(_)
                | Value::KnownArr(_)
                | Value::KnownStruct(_)
                | Value::Label(_)
                | Value::ConstExpr(_)
                | Value::Metadata(_)
        )
    }

    pub fn is_known_agg_target(&self) -> bool {
        matches!(
            self,
            Value::Function(_)
                | Value::GlobalPtr(_)
                | Value::NullPtr(_)
                | Value::Undef(_)
                | Value::KnownInt(_)
                | Value::KnownFloat(_)
                | Value::KnownVec(_)
                | Value::KnownArr(_)
                | Value::KnownStruct(_)
                | Value::ConstExpr(_)
        )
    }

    pub fn is_known_vec_target(&self) -> bool {
        matches!(
            self,
            Value::Undef(_) | Value::KnownInt(_) | Value::KnownFloat(_)
        )
    }
}

impl ArgumentVal {
    pub fn new(type_: Type, name: impl Into<String>) -> Self {
        ArgumentVal {
            type_,
            name: name.into(),
        }
    }
}

impl FunctionVal {
    pub fn new(type_: Type, name: impl Into<String>) -> Self {
        FunctionVal {
            type_,
            name: name.into(),
        }
    }
}

impl LocalVarVal {
    pub fn new(type_: Type, name: impl Into<String>) -> Self {
        LocalVarVal {
            type_,
            name: name.into(),
        }
    }
}

impl GlobalPtrVal {
    pub fn new(type_: Type, name: impl Into<String>) -> Self {
        GlobalPtrVal {
            type_,
            name: name.into(),
        }
    }
}

impl NullPtrVal {
    pub fn new(type_: Type) -> Self {
        NullPtrVal { type_ }
    }
}

impl UndefVal {
    pub fn new(type_: Type) -> Self {
        UndefVal { type_ }
    }
}

impl KnownIntVal {
    pub fn new(type_: Type, value: u128, width: u32) -> Self {
        KnownIntVal {
            type_,
            value,
            width,
        }
    }
}

impl KnownFloatVal {
    pub fn new(type_: Type, value: f64) -> Self {
        KnownFloatVal { type_, value }
    }
}

impl LabelVal {
    pub fn new(type_: Type, label: impl Into<String>) -> Self {
        LabelVal {
            type_,
            label: label.into(),
        }
    }
}

impl MetadataVal {
    pub fn new(type_: Type) -> Self {
        MetadataVal { type_ }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::types::*;

    #[test]
    fn test_argument_val() {
        let arg = ArgumentVal::new(Type::integer(32), "x");
        let val = Value::Argument(arg);
        assert!(matches!(val.type_(), Type::Integer(_)));
        assert!(!val.is_known());
    }

    #[test]
    fn test_known_int_val() {
        let known_int = KnownIntVal::new(Type::integer(32), 42, 32);
        let val = Value::KnownInt(known_int);
        assert!(val.is_known());
        assert!(val.is_known_agg_target());
        assert!(val.is_known_vec_target());
    }

    #[test]
    fn test_known_float_val() {
        let known_float = KnownFloatVal::new(Type::Double, 3.14);
        let val = Value::KnownFloat(known_float);
        assert!(val.is_known());
        assert!(val.is_known_agg_target());
        assert!(val.is_known_vec_target());
    }

    #[test]
    fn test_local_var_val() {
        let local = LocalVarVal::new(Type::integer(32), "tmp");
        let val = Value::LocalVar(local);
        assert!(!val.is_known());
        assert!(!val.is_known_agg_target());
    }

    #[test]
    fn test_null_ptr_val() {
        let null_ptr = NullPtrVal::new(Type::pointer(AddrSpace::Default));
        let val = Value::NullPtr(null_ptr);
        assert!(val.is_known());
        assert!(val.is_known_agg_target());
        assert!(!val.is_known_vec_target());
    }

    #[test]
    fn test_label_val() {
        let label = LabelVal::new(Type::Label, "entry");
        let val = Value::Label(label);
        assert!(val.is_known());
        assert!(!val.is_known_agg_target());
    }

    #[test]
    fn test_result_local_var() {
        let rlv = ResultLocalVar::new("tmp1");
        assert_eq!(rlv.name, "tmp1");
    }
}