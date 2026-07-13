use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AddrSpace {
    Default,
    Number(u32),
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Integer(IntegerTy),
    Half,
    Float,
    Double,
    Fp128,
    Pointer(PointerTy),
    Vector(VecTy),
    Array(ArrayTy),
    Struct(StructTy),
    Func(FuncTy),
    Label,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IntegerTy {
    pub width: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PointerTy {
    pub addrspace: AddrSpace,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VecTy {
    pub inner: Box<Type>,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArrayTy {
    pub inner: Box<Type>,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructTy {
    pub is_packed: bool,
    pub members: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FuncTy {
    pub return_type: Box<Type>,
    pub params: Vec<Type>,
    pub variadic: bool,
}

impl Type {
    pub fn is_agg_target(&self) -> bool {
        matches!(
            self,
            Type::Integer(_)
                | Type::Half
                | Type::Float
                | Type::Double
                | Type::Fp128
                | Type::Pointer(_)
                | Type::Vector(_)
                | Type::Array(_)
                | Type::Struct(_)
        )
    }

    pub fn is_vec_target(&self) -> bool {
        matches!(
            self,
            Type::Integer(_) | Type::Half | Type::Float | Type::Double | Type::Fp128 | Type::Pointer(_)
        )
    }

    pub fn is_floating_point(&self) -> bool {
        matches!(self, Type::Half | Type::Float | Type::Double | Type::Fp128)
    }

    pub fn integer(width: u32) -> Self {
        Type::Integer(IntegerTy { width })
    }

    pub fn pointer(addrspace: AddrSpace) -> Self {
        Type::Pointer(PointerTy { addrspace })
    }
}

impl IntegerTy {
    pub fn new(width: u32) -> Self {
        IntegerTy { width }
    }
}

impl PointerTy {
    pub fn new(addrspace: AddrSpace) -> Self {
        PointerTy { addrspace }
    }

    pub fn default_space() -> Self {
        PointerTy {
            addrspace: AddrSpace::Default,
        }
    }
}

impl VecTy {
    pub fn new(inner: Type, size: u32) -> Self {
        VecTy {
            inner: Box::new(inner),
            size,
        }
    }
}

impl ArrayTy {
    pub fn new(inner: Type, size: u32) -> Self {
        ArrayTy {
            inner: Box::new(inner),
            size,
        }
    }
}

impl StructTy {
    pub fn new(is_packed: bool, members: Vec<Type>) -> Self {
        StructTy {
            is_packed,
            members,
        }
    }
}

impl FuncTy {
    pub fn new(return_type: Type, params: Vec<Type>, variadic: bool) -> Self {
        FuncTy {
            return_type: Box::new(return_type),
            params,
            variadic,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Void => write!(f, "void"),
            Type::Integer(i) => write!(f, "i{}", i.width),
            Type::Half => write!(f, "half"),
            Type::Float => write!(f, "float"),
            Type::Double => write!(f, "double"),
            Type::Fp128 => write!(f, "fp128"),
            Type::Pointer(p) => match &p.addrspace {
                AddrSpace::Default => write!(f, "ptr"),
                AddrSpace::Number(n) => write!(f, "ptr addrspace({})", n),
                AddrSpace::Named(s) => write!(f, "ptr addrspace(\"{}\")", s),
            },
            Type::Vector(v) => write!(f, "<{} x {}>", v.size, v.inner),
            Type::Array(a) => write!(f, "[{} x {}]", a.size, a.inner),
            Type::Struct(s) => {
                if s.is_packed {
                    write!(f, "<{{")?;
                } else {
                    write!(f, "{{")?;
                }
                for (i, m) in s.members.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", m)?;
                }
                if s.is_packed {
                    write!(f, "}}>")
                } else {
                    write!(f, "}}")
                }
            }
            Type::Func(fn_ty) => {
                write!(f, "{} (", fn_ty.return_type)?;
                for (i, p) in fn_ty.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                if fn_ty.variadic {
                    write!(f, ", ...")?;
                }
                write!(f, ")")
            }
            Type::Label => write!(f, "label"),
            Type::Metadata => write!(f, "metadata"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_type() {
        let i32_ty = Type::integer(32);
        assert!(i32_ty.is_agg_target());
        assert!(i32_ty.is_vec_target());
        assert!(!i32_ty.is_floating_point());
        assert_eq!(format!("{}", i32_ty), "i32");
    }

    #[test]
    fn test_pointer_type() {
        let ptr_ty = Type::pointer(AddrSpace::Default);
        assert!(ptr_ty.is_agg_target());
        assert!(ptr_ty.is_vec_target());
        assert_eq!(format!("{}", ptr_ty), "ptr");
    }

    #[test]
    fn test_pointer_addrspace() {
        let ptr_ty = Type::pointer(AddrSpace::Number(1));
        assert_eq!(format!("{}", ptr_ty), "ptr addrspace(1)");
    }

    #[test]
    fn test_float_types() {
        assert!(Type::Half.is_floating_point());
        assert!(Type::Float.is_floating_point());
        assert!(Type::Double.is_floating_point());
        assert!(Type::Fp128.is_floating_point());
        assert!(!Type::Void.is_floating_point());
    }

    #[test]
    fn test_struct_type() {
        let struct_ty = Type::Struct(StructTy::new(
            false,
            vec![Type::integer(32), Type::Float],
        ));
        assert!(struct_ty.is_agg_target());
        assert!(!struct_ty.is_vec_target());
        assert_eq!(format!("{}", struct_ty), "{i32, float}");
    }

    #[test]
    fn test_packed_struct() {
        let struct_ty = Type::Struct(StructTy::new(
            true,
            vec![Type::integer(8), Type::integer(8)],
        ));
        assert_eq!(format!("{}", struct_ty), "<{i8, i8}>");
    }

    #[test]
    fn test_array_type() {
        let arr_ty = Type::Array(ArrayTy::new(Type::integer(32), 4));
        assert!(arr_ty.is_agg_target());
        assert!(!arr_ty.is_vec_target());
        assert_eq!(format!("{}", arr_ty), "[4 x i32]");
    }

    #[test]
    fn test_vector_type() {
        let vec_ty = Type::Vector(VecTy::new(Type::integer(32), 4));
        assert!(vec_ty.is_agg_target());
        assert!(!vec_ty.is_vec_target());
        assert_eq!(format!("{}", vec_ty), "<4 x i32>");
    }

    #[test]
    fn test_func_type() {
        let fn_ty = Type::Func(FuncTy::new(Type::integer(32), vec![Type::integer(32)], false));
        assert_eq!(format!("{}", fn_ty), "i32 (i32)");
    }

    #[test]
    fn test_variadic_func_type() {
        let fn_ty = Type::Func(FuncTy::new(Type::Void, vec![Type::integer(32)], true));
        assert_eq!(format!("{}", fn_ty), "void (i32, ...)");
    }

    #[test]
    fn test_void_not_agg_or_vec() {
        assert!(!Type::Void.is_agg_target());
        assert!(!Type::Void.is_vec_target());
    }

    #[test]
    fn test_label_not_agg_or_vec() {
        assert!(!Type::Label.is_agg_target());
        assert!(!Type::Label.is_vec_target());
    }
}