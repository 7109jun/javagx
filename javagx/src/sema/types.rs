//! 의미 분석 단계에서 사용하는 해석된(resolved) 타입 표현.
//!
//! `ast::TypeAnnotation`은 파싱 직후의 "구문상" 타입(클래스 존재 여부 등을
//! 검증하지 않은 상태)이고, `SemaType`은 클래스 테이블 기준으로 검증까지
//! 마친 "해석된" 타입이다.

use crate::parser::ast::{BaseType, TypeAnnotation};
use crate::sema::class_table::ClassTable;
use crate::sema::SemaError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Str,
    Char,
    Void,
    /// 사용자 정의 클래스 타입 (클래스 테이블에 존재함이 검증된 상태).
    Class(String),
    /// 배열 타입 `T[]`. 원소는 nullable을 갖지 않는다 (v0.1 제약, SPEC.md §6).
    Array(Box<Ty>),
    /// `null` 리터럴 전용 마커 타입. 어떤 선언된 타입에도 나타나지 않으며,
    /// `check_expr(Expr::Null)`의 결과로만 생성되어 대입 호환성 검사에서
    /// "nullable 타입이면 허용"이라는 특수 규칙에만 쓰인다.
    NullLit,
    /// 빈 배열 리터럴(`[]`) 전용 마커 타입. `NullLit`과 동일한 패턴으로, 원소
    /// 타입을 스스로 알 수 없으므로 대입 호환성 검사에서 "배열 타입이면 허용"
    /// 규칙에만 쓰인다.
    EmptyArrayLit,
}

impl Ty {
    pub fn is_integer(&self) -> bool {
        matches!(
            self,
            Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 | Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }

    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Ty::I8 => "i8", Ty::I16 => "i16", Ty::I32 => "i32", Ty::I64 => "i64",
            Ty::U8 => "u8", Ty::U16 => "u16", Ty::U32 => "u32", Ty::U64 => "u64",
            Ty::F32 => "f32", Ty::F64 => "f64",
            Ty::Bool => "bool", Ty::Str => "str", Ty::Char => "char", Ty::Void => "void",
            Ty::Class(name) => return write!(f, "{}", name),
            Ty::Array(elem) => return write!(f, "{}[]", elem),
            Ty::NullLit => "null",
            Ty::EmptyArrayLit => "[]",
        };
        write!(f, "{}", s)
    }
}

/// nullable 여부까지 포함한 완전한 타입.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemaType {
    pub ty: Ty,
    pub nullable: bool,
}

impl SemaType {
    pub fn new(ty: Ty, nullable: bool) -> Self {
        SemaType { ty, nullable }
    }

    pub fn non_null(ty: Ty) -> Self {
        SemaType { ty, nullable: false }
    }

    pub fn is_numeric(&self) -> bool {
        self.ty.is_numeric()
    }

    pub fn is_integer(&self) -> bool {
        self.ty.is_integer()
    }

    pub fn is_float(&self) -> bool {
        self.ty.is_float()
    }
}

impl std::fmt::Display for SemaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.ty, if self.nullable { "?" } else { "" })
    }
}

/// `ast::TypeAnnotation`을 `SemaType`으로 해석한다.
/// `Named(class)`가 클래스 테이블에 없으면 에러.
pub fn resolve_type(
    ann: &TypeAnnotation,
    classes: &ClassTable,
    line: usize,
) -> Result<SemaType, SemaError> {
    let ty = resolve_base_type(&ann.base, classes, line)?;
    Ok(SemaType::new(ty, ann.nullable))
}

/// `BaseType`(주석의 nullable을 제외한 "맨" 타입 부분)을 재귀적으로 해석한다.
/// 배열 원소 타입처럼 자체적인 nullable 플래그를 갖지 않는 위치에서 쓰인다.
fn resolve_base_type(base: &BaseType, classes: &ClassTable, line: usize) -> Result<Ty, SemaError> {
    Ok(match base {
        BaseType::I8 => Ty::I8, BaseType::I16 => Ty::I16,
        BaseType::I32 => Ty::I32, BaseType::I64 => Ty::I64,
        BaseType::U8 => Ty::U8, BaseType::U16 => Ty::U16,
        BaseType::U32 => Ty::U32, BaseType::U64 => Ty::U64,
        BaseType::F32 => Ty::F32, BaseType::F64 => Ty::F64,
        BaseType::Bool => Ty::Bool,
        BaseType::Str => Ty::Str,
        BaseType::Char => Ty::Char,
        BaseType::Void => Ty::Void,
        BaseType::Named(name) => {
            if !classes.contains(name) {
                return Err(SemaError {
                    message: format!("정의되지 않은 타입입니다: '{}'", name),
                    line,
                });
            }
            Ty::Class(name.clone())
        }
        BaseType::Array(elem) => Ty::Array(Box::new(resolve_base_type(elem, classes, line)?)),
    })
}
