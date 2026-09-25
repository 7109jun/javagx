//! `sema::types::Ty` → LLVM 타입 매핑, 클래스 구조체 레이아웃 계산.

use std::collections::HashMap;

use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum, StructType};
use inkwell::AddressSpace;

use crate::sema::class_table::ClassTable;
use crate::sema::types::{SemaType, Ty};

/// 클래스 하나의 LLVM 구조체 레이아웃: 상속 체인의 필드를 루트→리프 순서로 평탄화한다.
/// (v0.1은 가상 디스패치를 지원하지 않으므로 — SPEC.md §6 — vtable 포인터가 없다.)
#[derive(Debug, Clone)]
pub struct ClassLayout<'ctx> {
    pub struct_ty: StructType<'ctx>,
    /// 필드 이름 → 구조체 내 인덱스 (GEP에 사용).
    pub field_index: HashMap<String, u32>,
    pub field_types: Vec<SemaType>,
}

/// 모든 클래스의 필드를 상위 클래스 우선 순서로 수집한다.
fn flatten_fields(classes: &ClassTable, class_name: &str) -> Vec<(String, SemaType)> {
    let info = classes.get(class_name).expect("클래스가 존재해야 함 (Sema에서 이미 검증됨)");
    let mut fields = match &info.superclass {
        Some(sup) => flatten_fields(classes, sup),
        None => Vec::new(),
    };
    // HashMap 순서는 비결정적이므로 이름순으로 정렬해 재현 가능한 레이아웃을 만든다.
    let mut own: Vec<_> = info.fields.iter().map(|(n, f)| (n.clone(), f.ty.clone())).collect();
    own.sort_by(|a, b| a.0.cmp(&b.0));
    fields.extend(own);
    fields
}

/// 클래스 테이블 전체에 대해 LLVM 구조체 타입과 필드 레이아웃을 만든다.
/// 두 단계로 진행한다: 먼저 이름만 있는 opaque 구조체를 만들어 상호 참조(클래스가
/// 자기 자신을 참조하는 필드는 없지만, 다른 클래스 포인터를 필드로 가질 수 있음)를
/// 가능케 하고, 이후 본문을 채운다.
pub fn build_class_layouts<'ctx>(
    context: &'ctx Context,
    classes: &ClassTable,
) -> HashMap<String, ClassLayout<'ctx>> {
    let mut names: Vec<&str> = classes.names().collect();
    names.sort_unstable();

    let opaque: HashMap<String, StructType<'ctx>> = names
        .iter()
        .map(|n| (n.to_string(), context.opaque_struct_type(&format!("class.{}", n))))
        .collect();

    let mut layouts = HashMap::new();
    for name in &names {
        let flat = flatten_fields(classes, name);
        let mut field_index = HashMap::new();
        let mut field_types = Vec::new();
        let mut llvm_fields: Vec<BasicTypeEnum<'ctx>> = Vec::new();
        for (i, (fname, fty)) in flat.into_iter().enumerate() {
            field_index.insert(fname, i as u32);
            let llvm_ty = sema_ty_to_llvm(context, &opaque, &fty);
            llvm_fields.push(llvm_ty);
            field_types.push(fty);
        }
        let struct_ty = opaque[*name];
        struct_ty.set_body(&llvm_fields, false);
        layouts.insert((*name).to_string(), ClassLayout { struct_ty, field_index, field_types });
    }
    layouts
}

/// `SemaType`을 LLVM의 `BasicTypeEnum`으로 변환한다. `void`는 값 타입이 아니므로
/// 여기서 다루지 않는다 (함수 반환 타입 처리는 codegen/mod.rs에서 별도 처리).
pub fn sema_ty_to_llvm<'ctx>(
    context: &'ctx Context,
    class_structs: &HashMap<String, StructType<'ctx>>,
    ty: &SemaType,
) -> BasicTypeEnum<'ctx> {
    match &ty.ty {
        Ty::I8 | Ty::U8 => context.i8_type().into(),
        Ty::I16 | Ty::U16 => context.i16_type().into(),
        Ty::I32 | Ty::U32 => context.i32_type().into(),
        Ty::I64 | Ty::U64 => context.i64_type().into(),
        Ty::F32 => context.f32_type().into(),
        Ty::F64 => context.f64_type().into(),
        Ty::Bool => context.bool_type().into(),
        Ty::Char => context.i32_type().into(), // 유니코드 코드포인트
        Ty::Str => str_struct_type(context).into(),
        // 오파크 포인터 하에서는 원소 타입과 무관하게 모든 배열이 `{ptr, i64}`로 동일하게
        // 표현된다 (str과 물리적으로 같은 fat-pointer 레이아웃). 원소 크기는 실제
        // 인덱싱/생성 시점에 SemaType으로부터 별도로 계산한다.
        Ty::Array(_) => str_struct_type(context).into(),
        Ty::Void => panic!("void는 값 타입으로 변환할 수 없습니다"),
        Ty::Class(name) => {
            if !class_structs.contains_key(name) {
                panic!("클래스 구조체가 없습니다: {}", name);
            }
            // 오파크 포인터: 모든 클래스가 동일한 포인터 타입을 공유한다 (pointee는 GEP 시점에 지정).
            context.ptr_type(AddressSpace::default()).into()
        }
        Ty::NullLit | Ty::EmptyArrayLit => {
            panic!("NullLit/EmptyArrayLit 마커 타입은 코드 생성 단계까지 남아있으면 안 됩니다 (Sema 버그)")
        }
    }
}

/// `str` 타입의 LLVM 표현: `{ i8* data, i64 len }` (불변 fat pointer, 값으로 전달).
pub fn str_struct_type(context: &Context) -> StructType<'_> {
    context.struct_type(
        &[context.ptr_type(AddressSpace::default()).into(), context.i64_type().into()],
        false,
    )
}

pub fn sema_ty_to_metadata<'ctx>(
    context: &'ctx Context,
    class_structs: &HashMap<String, StructType<'ctx>>,
    ty: &SemaType,
) -> BasicMetadataTypeEnum<'ctx> {
    sema_ty_to_llvm(context, class_structs, ty).into()
}

/// 정수 타입(`i8..u64`)이 부호 있는 타입인지 여부. LLVM IR 자체는 부호를 구분하지
/// 않지만(같은 iN 타입), 나눗셈/나머지/비교/확장 명령 선택에 필요하다.
pub fn is_signed(ty: &Ty) -> bool {
    matches!(ty, Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64)
}
