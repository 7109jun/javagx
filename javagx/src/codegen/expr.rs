//! 표현식(`Expr`) 코드 생성.

use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate};

use crate::parser::ast::*;
use crate::sema::types::{SemaType, Ty};

use super::env::FnCtx;
use super::{types, CodegenError, Codegen, TypedValue};

type CResult<T> = Result<T, CodegenError>;

/// `expr`이 (Grouping을 무시하고) 정수/실수 리터럴인지: Sema의 리터럴 승격 규칙과
/// 동일한 판정을 codegen 쪽에서도 재현하기 위함이다 (SPEC.md §3.2).
fn is_literal_expr(expr: &Expr) -> bool {
    match expr {
        Expr::IntLit(_) | Expr::FloatLit(_) => true,
        Expr::Grouping(inner) => is_literal_expr(inner),
        _ => false,
    }
}

impl<'ctx> Codegen<'ctx> {
    pub(super) fn gen_expr(&self, expr: &Expr, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        match expr {
            Expr::IntLit(v) => Ok(TypedValue {
                value: self.context.i32_type().const_int(*v as u64, true).into(),
                ty: SemaType::non_null(Ty::I32),
            }),
            Expr::FloatLit(v) => Ok(TypedValue {
                value: self.context.f64_type().const_float(*v).into(),
                ty: SemaType::non_null(Ty::F64),
            }),
            Expr::BoolLit(b) => Ok(TypedValue {
                value: self.context.bool_type().const_int(*b as u64, false).into(),
                ty: SemaType::non_null(Ty::Bool),
            }),
            Expr::CharLit(c) => Ok(TypedValue {
                value: self.context.i32_type().const_int(*c as u64, false).into(),
                ty: SemaType::non_null(Ty::Char),
            }),
            Expr::StringLit(s) => {
                let gptr = self.builder.build_global_string_ptr(s, "str_lit").unwrap();
                let ptr = gptr.as_pointer_value();
                let len = self.context.i64_type().const_int(s.len() as u64, false);
                let str_ty = types::str_struct_type(self.context);
                let v1 = self.builder.build_insert_value(str_ty.get_undef(), ptr, 0, "s0").unwrap();
                let v2 = self.builder.build_insert_value(v1, len, 1, "s1").unwrap();
                Ok(TypedValue { value: v2.into_struct_value().into(), ty: SemaType::non_null(Ty::Str) })
            }
            Expr::Null => Ok(TypedValue {
                value: self.context.ptr_type(AddressSpace::default()).const_null().into(),
                ty: SemaType::new(Ty::NullLit, true),
            }),
            Expr::SelfExpr => self.gen_ident("self", fx, 0),
            Expr::Ident(name) => self.gen_ident(name, fx, 0),
            Expr::Grouping(inner) => self.gen_expr(inner, fx),
            Expr::New { class_name, args, line } => self.gen_new(class_name, args, *line, fx),
            Expr::Call { callee, args, line } => self.gen_call(callee, args, *line, fx),
            Expr::Member { object, name, line } => self.gen_member_read(object, name, *line, fx),
            Expr::ArrayLit { elements, line } => self.gen_array_lit(elements, *line, fx),
            Expr::Index { object, index, line } => self.gen_index_read(object, index, *line, fx),
            Expr::Cast { expr, ty, line } => {
                let v = self.gen_expr(expr, fx)?;
                let target = super::sema_type_of(ty);
                self.numeric_cast(v, &target, *line)
            }
            Expr::Unary { op, expr, line } => self.gen_unary(*op, expr, *line, fx),
            Expr::Binary { op, lhs, rhs, line } => self.gen_binary(*op, lhs, rhs, *line, fx),
            Expr::Assign { op, target, value, line } => self.gen_assign(*op, target, value, *line, fx),
        }
    }

    fn gen_ident(&self, name: &str, fx: &FnCtx<'ctx>, line: usize) -> CResult<TypedValue<'ctx>> {
        let (ptr, ty) = fx
            .env
            .lookup(name)
            .cloned()
            .ok_or_else(|| CodegenError { message: format!("정의되지 않은 변수입니다: '{}'", name), line })?;
        let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &ty);
        let loaded = self.builder.build_load(llvm_ty, ptr, name).unwrap();
        Ok(TypedValue { value: loaded, ty })
    }

    fn gen_new(&self, class_name: &str, args: &[Expr], line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let layout = self.class_layouts.get(class_name).ok_or_else(|| CodegenError {
            message: format!("정의되지 않은 클래스입니다: '{}'", class_name),
            line,
        })?;
        let struct_ty = layout.struct_ty;
        let size = struct_ty
            .size_of()
            .ok_or_else(|| CodegenError { message: "크기를 계산할 수 없는 클래스입니다".into(), line })?;
        let raw = self
            .builder
            .build_call(self.rt.malloc, &[size.into()], "new_raw")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let obj_ptr = self.builder.build_pointer_cast(raw, ptr_ty, "new_obj").unwrap();

        if let Some(ctor) = self.classes.resolve_method(class_name, "init") {
            let func = self.methods[&(ctor.owner.clone(), "init".to_string())];
            let mut call_args: Vec<BasicMetadataValueEnum> = vec![obj_ptr.into()];
            for (arg_expr, param_ty) in args.iter().zip(ctor.param_types.iter()) {
                let v = self.gen_expr(arg_expr, fx)?;
                let c = self.coerce(v, param_ty, line)?;
                call_args.push(c.value.into());
            }
            self.builder.build_call(func, &call_args, "ctor_call").unwrap();
        }

        Ok(TypedValue { value: obj_ptr.into(), ty: SemaType::non_null(Ty::Class(class_name.to_string())) })
    }

    fn gen_call(&self, callee: &Expr, args: &[Expr], line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        match callee {
            Expr::Member { object, name, .. } => {
                let obj = self.gen_expr(object, fx)?;
                let cls = self.expect_class(&obj.ty, line)?;
                let method = self.classes.resolve_method(&cls, name).ok_or_else(|| CodegenError {
                    message: format!("클래스 '{}'에 정의되지 않은 메서드입니다: '{}'", cls, name),
                    line,
                })?;
                let func = self.methods[&(method.owner.clone(), name.clone())];
                let obj_ptr = obj.value.into_pointer_value();
                let mut call_args: Vec<BasicMetadataValueEnum> = vec![obj_ptr.into()];
                for (a, pt) in args.iter().zip(method.param_types.iter()) {
                    let v = self.gen_expr(a, fx)?;
                    let c = self.coerce(v, pt, line)?;
                    call_args.push(c.value.into());
                }
                let ret_ty = method.ret.clone();
                let call = self.builder.build_call(func, &call_args, "mcall").unwrap();
                let value = call.try_as_basic_value().left().unwrap_or_else(|| self.void_placeholder());
                Ok(TypedValue { value, ty: ret_ty })
            }
            Expr::Ident(name) if name == "print" && !self.funcs.contains_key("print") => {
                let v = self.gen_expr(&args[0], fx)?;
                self.gen_print(v, line)?;
                Ok(TypedValue { value: self.void_placeholder(), ty: SemaType::non_null(Ty::Void) })
            }
            Expr::Ident(name) => {
                let sig = self
                    .funcs
                    .get(name)
                    .ok_or_else(|| CodegenError { message: format!("정의되지 않은 함수입니다: '{}'", name), line })?;
                let func = self.functions[name];
                let mut call_args: Vec<BasicMetadataValueEnum> = Vec::new();
                for (a, pt) in args.iter().zip(sig.param_types.iter()) {
                    let v = self.gen_expr(a, fx)?;
                    let c = self.coerce(v, pt, line)?;
                    call_args.push(c.value.into());
                }
                let ret_ty = sig.ret.clone();
                let call = self.builder.build_call(func, &call_args, "call").unwrap();
                let value = call.try_as_basic_value().left().unwrap_or_else(|| self.void_placeholder());
                Ok(TypedValue { value, ty: ret_ty })
            }
            _ => Err(CodegenError { message: "호출할 수 없는 표현식입니다".into(), line }),
        }
    }

    /// void 표현식(문(stmt)으로만 쓰여야 함)의 자리 표시자 값. Sema가 void 값이
    /// 더 큰 표현식에 쓰이는 것을 막지 못하는 알려진 한계가 있다 (SPEC.md §6).
    fn void_placeholder(&self) -> BasicValueEnum<'ctx> {
        self.context.bool_type().const_int(0, false).into()
    }

    /// `print(value)`: 값의 타입에 맞춰 서식을 골라 표준출력에 한 줄로 출력한다 (줄바꿈 포함).
    /// 원시 타입 전반과 `str`을 지원하며, 클래스/배열 값은 아직 지원하지 않는다
    /// (사용자 정의 `toString`/원소별 출력은 stdlib 후속 버전 과제, SPEC.md §6).
    fn gen_print(&self, v: TypedValue<'ctx>, line: usize) -> CResult<()> {
        if v.ty.nullable {
            return Err(CodegenError { message: "print()는 nullable 값을 직접 출력할 수 없습니다 (null 검사가 필요합니다)".into(), line });
        }
        match &v.ty.ty {
            Ty::Str => {
                let sv = v.value.into_struct_value();
                let ptr = self.builder.build_extract_value(sv, 0, "p").unwrap().into_pointer_value();
                let len = self.builder.build_extract_value(sv, 1, "l").unwrap().into_int_value();
                let len32 = self.builder.build_int_truncate(len, self.context.i32_type(), "len32").unwrap();
                let fmt = self.builder.build_global_string_ptr("%.*s\n", "print_fmt_s").unwrap();
                self.builder
                    .build_call(self.rt.printf, &[fmt.as_pointer_value().into(), len32.into(), ptr.into()], "printf_call")
                    .unwrap();
            }
            Ty::I8 | Ty::I16 | Ty::I32 | Ty::I64 => {
                let iv = v.value.into_int_value();
                let ext = self.builder.build_int_s_extend_or_bit_cast(iv, self.context.i64_type(), "ext").unwrap();
                let fmt = self.builder.build_global_string_ptr("%lld\n", "print_fmt_i").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), ext.into()], "printf_call").unwrap();
            }
            Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 => {
                let iv = v.value.into_int_value();
                let ext = self.builder.build_int_z_extend_or_bit_cast(iv, self.context.i64_type(), "ext").unwrap();
                let fmt = self.builder.build_global_string_ptr("%llu\n", "print_fmt_u").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), ext.into()], "printf_call").unwrap();
            }
            Ty::F32 => {
                let fv = v.value.into_float_value();
                let ext = self.builder.build_float_ext(fv, self.context.f64_type(), "ext").unwrap();
                let fmt = self.builder.build_global_string_ptr("%f\n", "print_fmt_f").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), ext.into()], "printf_call").unwrap();
            }
            Ty::F64 => {
                let fv = v.value.into_float_value();
                let fmt = self.builder.build_global_string_ptr("%f\n", "print_fmt_f").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), fv.into()], "printf_call").unwrap();
            }
            Ty::Bool => {
                let iv = v.value.into_int_value();
                let t = self.builder.build_global_string_ptr("true\n", "print_true").unwrap();
                let fls = self.builder.build_global_string_ptr("false\n", "print_false").unwrap();
                let sel = self
                    .builder
                    .build_select(iv, t.as_pointer_value(), fls.as_pointer_value(), "sel")
                    .unwrap();
                let fmt = self.builder.build_global_string_ptr("%s", "print_fmt_bool").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), sel.into()], "printf_call").unwrap();
            }
            Ty::Char => {
                let iv = v.value.into_int_value();
                let fmt = self.builder.build_global_string_ptr("%c\n", "print_fmt_c").unwrap();
                self.builder.build_call(self.rt.printf, &[fmt.as_pointer_value().into(), iv.into()], "printf_call").unwrap();
            }
            other => {
                return Err(CodegenError {
                    message: format!("print()는 현재 '{}' 타입을 지원하지 않습니다 (stdlib 후속 버전에서 지원 예정)", other),
                    line,
                })
            }
        }
        Ok(())
    }

    fn expect_class(&self, ty: &SemaType, line: usize) -> CResult<String> {
        match &ty.ty {
            Ty::Class(c) => Ok(c.clone()),
            other => Err(CodegenError { message: format!("클래스 타입이 아닙니다: {}", other), line }),
        }
    }

    fn gen_member_read(&self, object: &Expr, name: &str, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let obj = self.gen_expr(object, fx)?;
        if let Ty::Array(_) = &obj.ty.ty {
            if name != "length" {
                return Err(CodegenError { message: format!("배열에는 'length' 외의 필드가 없습니다: '{}'", name), line });
            }
            let sv = obj.value.into_struct_value();
            let len = self.builder.build_extract_value(sv, 1, "length").unwrap();
            return Ok(TypedValue { value: len, ty: SemaType::non_null(Ty::I64) });
        }
        let (field_ptr, field_ty) = self.field_ptr_from(obj, name, line)?;
        let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &field_ty);
        let loaded = self.builder.build_load(llvm_ty, field_ptr, name).unwrap();
        Ok(TypedValue { value: loaded, ty: field_ty })
    }

    fn gen_array_lit(&self, elements: &[Expr], line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let str_ty = types::str_struct_type(self.context);
        if elements.is_empty() {
            // 빈 배열: 원소를 저장할 필요가 없으므로 null 포인터 + 길이 0으로 표현한다.
            // (실제 원소 타입은 Sema가 대입 위치의 선언된 타입으로 이미 결정했으며,
            // 여기서는 물리 표현이 원소 타입과 무관하므로 값 생성에 필요 없다.)
            let ptr = self.context.ptr_type(AddressSpace::default()).const_null();
            let len = self.context.i64_type().const_zero();
            let v1 = self.builder.build_insert_value(str_ty.get_undef(), ptr, 0, "a0").unwrap();
            let v2 = self.builder.build_insert_value(v1, len, 1, "a1").unwrap();
            return Ok(TypedValue { value: v2.into_struct_value().into(), ty: SemaType::new(Ty::EmptyArrayLit, false) });
        }

        let first = self.gen_expr(&elements[0], fx)?;
        let elem_ty = first.ty.clone();
        let elem_llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &elem_ty);
        let count = self.context.i64_type().const_int(elements.len() as u64, false);
        let elem_size = elem_llvm_ty
            .size_of()
            .ok_or_else(|| CodegenError { message: "크기를 계산할 수 없는 배열 원소 타입입니다".into(), line })?;
        let total_size = self.builder.build_int_mul(count, elem_size, "arr_bytes").unwrap();
        let raw = self
            .builder
            .build_call(self.rt.malloc, &[total_size.into()], "arr_raw")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        for (i, el) in elements.iter().enumerate() {
            let v = if i == 0 { first.clone() } else { self.gen_expr(el, fx)? };
            let coerced = self.coerce(v, &elem_ty, line)?;
            let idx = self.context.i64_type().const_int(i as u64, false);
            let elem_ptr = unsafe { self.builder.build_gep(elem_llvm_ty, raw, &[idx], "arr_elem").unwrap() };
            self.builder.build_store(elem_ptr, coerced.value).unwrap();
        }

        let len = self.context.i64_type().const_int(elements.len() as u64, false);
        let v1 = self.builder.build_insert_value(str_ty.get_undef(), raw, 0, "a0").unwrap();
        let v2 = self.builder.build_insert_value(v1, len, 1, "a1").unwrap();
        Ok(TypedValue { value: v2.into_struct_value().into(), ty: SemaType::non_null(Ty::Array(Box::new(elem_ty.ty))) })
    }

    fn gen_index_read(&self, object: &Expr, index: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let (elem_ptr, elem_ty) = self.array_elem_ptr(object, index, line, fx)?;
        let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &elem_ty);
        let loaded = self.builder.build_load(llvm_ty, elem_ptr, "idx").unwrap();
        Ok(TypedValue { value: loaded, ty: elem_ty })
    }

    fn array_elem_ptr(
        &self,
        object: &Expr,
        index: &Expr,
        line: usize,
        fx: &mut FnCtx<'ctx>,
    ) -> CResult<(PointerValue<'ctx>, SemaType)> {
        let obj = self.gen_expr(object, fx)?;
        let elem_ty = match &obj.ty.ty {
            Ty::Array(e) => SemaType::non_null((**e).clone()),
            other => return Err(CodegenError { message: format!("배열 타입이 아닙니다: {}", other), line }),
        };
        let idx_v = self.gen_expr(index, fx)?;
        let idx64 = self.builder.build_int_cast(idx_v.value.into_int_value(), self.context.i64_type(), "idx64").unwrap();
        let sv = obj.value.into_struct_value();
        let data_ptr = self.builder.build_extract_value(sv, 0, "data").unwrap().into_pointer_value();
        let elem_llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &elem_ty);
        let elem_ptr = unsafe { self.builder.build_gep(elem_llvm_ty, data_ptr, &[idx64], "elem_ptr").unwrap() };
        Ok((elem_ptr, elem_ty))
    }

    fn field_ptr(
        &self,
        object: &Expr,
        name: &str,
        line: usize,
        fx: &mut FnCtx<'ctx>,
    ) -> CResult<(PointerValue<'ctx>, SemaType)> {
        let obj = self.gen_expr(object, fx)?;
        self.field_ptr_from(obj, name, line)
    }

    /// 이미 평가된 객체 값(`obj`)로부터 필드 포인터를 얻는다 (읽기 경로에서 객체
    /// 표현식을 중복 평가하지 않기 위함 — 부수효과가 있는 표현식에서 중요하다).
    fn field_ptr_from(&self, obj: TypedValue<'ctx>, name: &str, line: usize) -> CResult<(PointerValue<'ctx>, SemaType)> {
        let cls = self.expect_class(&obj.ty, line)?;
        let layout = &self.class_layouts[&cls];
        let idx = *layout
            .field_index
            .get(name)
            .ok_or_else(|| CodegenError { message: format!("클래스 '{}'에 정의되지 않은 필드입니다: '{}'", cls, name), line })?;
        let obj_ptr = obj.value.into_pointer_value();
        let field_ptr = self.builder.build_struct_gep(layout.struct_ty, obj_ptr, idx, name).unwrap();
        Ok((field_ptr, layout.field_types[idx as usize].clone()))
    }

    // -----------------------------------------------------------
    // 단항 / 이항 연산자
    // -----------------------------------------------------------

    fn gen_unary(&self, op: UnaryOp, expr: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let v = self.gen_expr(expr, fx)?;
        match op {
            UnaryOp::Not => {
                let iv = v.value.into_int_value();
                let notted = self.builder.build_not(iv, "not").unwrap();
                Ok(TypedValue { value: notted.into(), ty: SemaType::non_null(Ty::Bool) })
            }
            UnaryOp::Neg => match v.value {
                BasicValueEnum::IntValue(iv) => {
                    Ok(TypedValue { value: self.builder.build_int_neg(iv, "neg").unwrap().into(), ty: v.ty })
                }
                BasicValueEnum::FloatValue(fv) => {
                    Ok(TypedValue { value: self.builder.build_float_neg(fv, "fneg").unwrap().into(), ty: v.ty })
                }
                _ => Err(CodegenError { message: "단항 '-'는 숫자 타입에만 사용할 수 있습니다".into(), line }),
            },
        }
    }

    fn gen_binary(&self, op: BinaryOp, lhs: &Expr, rhs: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let l = self.gen_expr(lhs, fx)?.value.into_int_value();
            let r = self.gen_expr(rhs, fx)?.value.into_int_value();
            let result = if op == BinaryOp::And {
                self.builder.build_and(l, r, "and").unwrap()
            } else {
                self.builder.build_or(l, r, "or").unwrap()
            };
            return Ok(TypedValue { value: result.into(), ty: SemaType::non_null(Ty::Bool) });
        }

        if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) {
            return self.gen_equality(op, lhs, rhs, line, fx);
        }

        let lv = self.gen_expr(lhs, fx)?;
        let rv = self.gen_expr(rhs, fx)?;

        // '+'는 str 연결도 허용한다 (SPEC.md §3.3.1).
        if op == BinaryOp::Add && lv.ty.ty == Ty::Str && rv.ty.ty == Ty::Str {
            let call = self.builder.build_call(self.rt.str_concat, &[lv.value.into(), rv.value.into()], "concat").unwrap();
            let result = call.try_as_basic_value().left().unwrap();
            return Ok(TypedValue { value: result, ty: SemaType::non_null(Ty::Str) });
        }

        let (l, r, ty) = self.unify_numeric_values(lhs, lv, rhs, rv, line)?;
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                let value = self.arith(op, l, r, &ty, line)?;
                Ok(TypedValue { value, ty })
            }
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                let value = self.compare(op, l, r, &ty, line)?;
                Ok(TypedValue { value: value.into(), ty: SemaType::non_null(Ty::Bool) })
            }
            _ => unreachable!("and/or/eq/noteq는 위에서 조기 반환됨"),
        }
    }

    fn gen_equality(&self, op: BinaryOp, lhs: &Expr, rhs: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let lv = self.gen_expr(lhs, fx)?;
        let rv = self.gen_expr(rhs, fx)?;

        if lv.ty.ty == Ty::NullLit || rv.ty.ty == Ty::NullLit {
            let ptr_val = if lv.ty.ty == Ty::NullLit { rv.value } else { lv.value };
            let ptr = ptr_val.into_pointer_value();
            let b = if op == BinaryOp::Eq {
                self.builder.build_is_null(ptr, "isnull").unwrap()
            } else {
                self.builder.build_is_not_null(ptr, "isnotnull").unwrap()
            };
            return Ok(TypedValue { value: b.into(), ty: SemaType::non_null(Ty::Bool) });
        }

        if lv.ty.ty == Ty::Str && rv.ty.ty == Ty::Str {
            return self.gen_str_equality(op, lv, rv);
        }

        let (l, r, ty) = if lv.ty.ty == rv.ty.ty {
            (lv.value, rv.value, lv.ty.clone())
        } else if is_literal_expr(lhs) {
            let target = rv.ty.clone();
            let cl = self.coerce(lv, &target, line)?;
            (cl.value, rv.value, target)
        } else if is_literal_expr(rhs) {
            let target = lv.ty.clone();
            let cr = self.coerce(rv, &target, line)?;
            (lv.value, cr.value, target)
        } else {
            return Err(CodegenError { message: "비교 피연산자 타입이 일치하지 않습니다".into(), line });
        };

        match (l, r) {
            (BasicValueEnum::PointerValue(lp), BasicValueEnum::PointerValue(rp)) => {
                let li = self.builder.build_ptr_to_int(lp, self.context.i64_type(), "l2i").unwrap();
                let ri = self.builder.build_ptr_to_int(rp, self.context.i64_type(), "r2i").unwrap();
                let pred = if op == BinaryOp::Eq { IntPredicate::EQ } else { IntPredicate::NE };
                let b = self.builder.build_int_compare(pred, li, ri, "ptrcmp").unwrap();
                Ok(TypedValue { value: b.into(), ty: SemaType::non_null(Ty::Bool) })
            }
            _ => {
                let b = self.compare(op, l, r, &ty, line)?;
                Ok(TypedValue { value: b.into(), ty: SemaType::non_null(Ty::Bool) })
            }
        }
    }

    /// `str` 값 내용 비교: 길이가 먼저 다르면 즉시 거짓, 같으면 `memcmp`로 바이트를 비교한다.
    fn gen_str_equality(&self, op: BinaryOp, lv: TypedValue<'ctx>, rv: TypedValue<'ctx>) -> CResult<TypedValue<'ctx>> {
        let a = lv.value.into_struct_value();
        let b = rv.value.into_struct_value();
        let la = self.builder.build_extract_value(a, 1, "la").unwrap().into_int_value();
        let lb = self.builder.build_extract_value(b, 1, "lb").unwrap().into_int_value();
        let len_eq = self.builder.build_int_compare(IntPredicate::EQ, la, lb, "len_eq").unwrap();

        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let entry_bb = self.builder.get_insert_block().unwrap();
        let cmp_bb = self.context.append_basic_block(function, "streq.cmp");
        let merge_bb = self.context.append_basic_block(function, "streq.merge");
        self.builder.build_conditional_branch(len_eq, cmp_bb, merge_bb).unwrap();

        self.builder.position_at_end(cmp_bb);
        let pa = self.builder.build_extract_value(a, 0, "pa").unwrap().into_pointer_value();
        let pb = self.builder.build_extract_value(b, 0, "pb").unwrap().into_pointer_value();
        let mres = self
            .builder
            .build_call(self.rt.memcmp, &[pa.into(), pb.into(), la.into()], "memcmp_call")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_int_value();
        let content_eq =
            self.builder.build_int_compare(IntPredicate::EQ, mres, self.context.i32_type().const_zero(), "content_eq").unwrap();
        self.builder.build_unconditional_branch(merge_bb).unwrap();
        let cmp_bb_end = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(merge_bb);
        let phi = self.builder.build_phi(self.context.bool_type(), "streq").unwrap();
        let false_val = self.context.bool_type().const_int(0, false);
        phi.add_incoming(&[(&false_val, entry_bb), (&content_eq, cmp_bb_end)]);
        let eq_val = phi.as_basic_value().into_int_value();

        let result = if op == BinaryOp::Eq { eq_val } else { self.builder.build_not(eq_val, "streq.not").unwrap() };
        Ok(TypedValue { value: result.into(), ty: SemaType::non_null(Ty::Bool) })
    }

    fn unify_numeric_values(
        &self,
        lhs_e: &Expr,
        lv: TypedValue<'ctx>,
        rhs_e: &Expr,
        rv: TypedValue<'ctx>,
        line: usize,
    ) -> CResult<(BasicValueEnum<'ctx>, BasicValueEnum<'ctx>, SemaType)> {
        if lv.ty.ty == rv.ty.ty {
            let ty = lv.ty.clone();
            return Ok((lv.value, rv.value, ty));
        }
        if is_literal_expr(lhs_e) {
            let target = rv.ty.clone();
            let l = self.coerce(lv, &target, line)?;
            return Ok((l.value, rv.value, target));
        }
        if is_literal_expr(rhs_e) {
            let target = lv.ty.clone();
            let r = self.coerce(rv, &target, line)?;
            return Ok((lv.value, r.value, target));
        }
        Err(CodegenError { message: "피연산자 타입이 일치하지 않습니다 (Sema를 통과했다면 발생할 수 없음)".into(), line })
    }

    fn arith(&self, op: BinaryOp, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>, ty: &SemaType, line: usize) -> CResult<BasicValueEnum<'ctx>> {
        match (l, r) {
            (BasicValueEnum::IntValue(li), BasicValueEnum::IntValue(ri)) => {
                let signed = types::is_signed(&ty.ty);
                let v = match op {
                    BinaryOp::Add => self.builder.build_int_add(li, ri, "add").unwrap(),
                    BinaryOp::Sub => self.builder.build_int_sub(li, ri, "sub").unwrap(),
                    BinaryOp::Mul => self.builder.build_int_mul(li, ri, "mul").unwrap(),
                    BinaryOp::Div if signed => self.builder.build_int_signed_div(li, ri, "div").unwrap(),
                    BinaryOp::Div => self.builder.build_int_unsigned_div(li, ri, "div").unwrap(),
                    BinaryOp::Mod if signed => self.builder.build_int_signed_rem(li, ri, "rem").unwrap(),
                    BinaryOp::Mod => self.builder.build_int_unsigned_rem(li, ri, "rem").unwrap(),
                    _ => unreachable!(),
                };
                Ok(v.into())
            }
            (BasicValueEnum::FloatValue(lf), BasicValueEnum::FloatValue(rf)) => {
                let v = match op {
                    BinaryOp::Add => self.builder.build_float_add(lf, rf, "fadd").unwrap(),
                    BinaryOp::Sub => self.builder.build_float_sub(lf, rf, "fsub").unwrap(),
                    BinaryOp::Mul => self.builder.build_float_mul(lf, rf, "fmul").unwrap(),
                    BinaryOp::Div => self.builder.build_float_div(lf, rf, "fdiv").unwrap(),
                    BinaryOp::Mod => self.builder.build_float_rem(lf, rf, "frem").unwrap(),
                    _ => unreachable!(),
                };
                Ok(v.into())
            }
            _ => Err(CodegenError { message: "산술 연산 피연산자 타입이 일치하지 않습니다".into(), line }),
        }
    }

    fn compare(&self, op: BinaryOp, l: BasicValueEnum<'ctx>, r: BasicValueEnum<'ctx>, ty: &SemaType, line: usize) -> CResult<inkwell::values::IntValue<'ctx>> {
        match (l, r) {
            (BasicValueEnum::IntValue(li), BasicValueEnum::IntValue(ri)) => {
                let signed = types::is_signed(&ty.ty);
                let pred = match op {
                    BinaryOp::Lt => if signed { IntPredicate::SLT } else { IntPredicate::ULT },
                    BinaryOp::LtEq => if signed { IntPredicate::SLE } else { IntPredicate::ULE },
                    BinaryOp::Gt => if signed { IntPredicate::SGT } else { IntPredicate::UGT },
                    BinaryOp::GtEq => if signed { IntPredicate::SGE } else { IntPredicate::UGE },
                    BinaryOp::Eq => IntPredicate::EQ,
                    BinaryOp::NotEq => IntPredicate::NE,
                    _ => unreachable!(),
                };
                Ok(self.builder.build_int_compare(pred, li, ri, "cmp").unwrap())
            }
            (BasicValueEnum::FloatValue(lf), BasicValueEnum::FloatValue(rf)) => {
                let pred = match op {
                    BinaryOp::Lt => FloatPredicate::OLT,
                    BinaryOp::LtEq => FloatPredicate::OLE,
                    BinaryOp::Gt => FloatPredicate::OGT,
                    BinaryOp::GtEq => FloatPredicate::OGE,
                    BinaryOp::Eq => FloatPredicate::OEQ,
                    BinaryOp::NotEq => FloatPredicate::ONE,
                    _ => unreachable!(),
                };
                Ok(self.builder.build_float_compare(pred, lf, rf, "fcmp").unwrap())
            }
            _ => Err(CodegenError { message: "비교 연산 피연산자 타입이 일치하지 않습니다".into(), line }),
        }
    }

    // -----------------------------------------------------------
    // 대입
    // -----------------------------------------------------------

    fn gen_assign(&self, op: AssignOp, target: &Expr, value: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<TypedValue<'ctx>> {
        let (ptr, target_ty) = self.lvalue_ptr(target, line, fx)?;
        let val = self.gen_expr(value, fx)?;

        let final_val = match op {
            AssignOp::Assign => self.coerce(val, &target_ty, line)?,
            _ => {
                let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &target_ty);
                let current = self.builder.build_load(llvm_ty, ptr, "cur").unwrap();
                let coerced_rhs = self.coerce(val, &target_ty, line)?;
                let bop = match op {
                    AssignOp::AddAssign => BinaryOp::Add,
                    AssignOp::SubAssign => BinaryOp::Sub,
                    AssignOp::MulAssign => BinaryOp::Mul,
                    AssignOp::DivAssign => BinaryOp::Div,
                    AssignOp::Assign => unreachable!(),
                };
                let result = self.arith(bop, current, coerced_rhs.value, &target_ty, line)?;
                TypedValue { value: result, ty: target_ty.clone() }
            }
        };
        self.builder.build_store(ptr, final_val.value).unwrap();
        Ok(final_val)
    }

    fn lvalue_ptr(&self, target: &Expr, line: usize, fx: &mut FnCtx<'ctx>) -> CResult<(PointerValue<'ctx>, SemaType)> {
        match target {
            Expr::Ident(name) => fx
                .env
                .lookup(name)
                .cloned()
                .ok_or_else(|| CodegenError { message: format!("정의되지 않은 변수입니다: '{}'", name), line }),
            Expr::Member { object, name, .. } => self.field_ptr(object, name, line, fx),
            Expr::Index { object, index, .. } => self.array_elem_ptr(object, index, line, fx),
            _ => Err(CodegenError { message: "지원되지 않는 대입 대상입니다".into(), line }),
        }
    }

    // -----------------------------------------------------------
    // 타입 변환 (캐스팅 / 리터럴 승격 / null)
    // -----------------------------------------------------------

    /// 값을 `target` 타입으로 변환한다. 정수/실수 리터럴 승격과 명시적 `as` 캐스팅에
    /// 공통으로 쓰인다. Sema가 이미 이 변환이 유효함을 검증했다고 가정한다.
    pub(super) fn coerce(&self, v: TypedValue<'ctx>, target: &SemaType, line: usize) -> CResult<TypedValue<'ctx>> {
        if v.ty.ty == target.ty {
            return Ok(TypedValue { value: v.value, ty: target.clone() });
        }
        if v.ty.ty == Ty::NullLit {
            let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, target);
            return match llvm_ty {
                BasicTypeEnum::PointerType(pt) => Ok(TypedValue { value: pt.const_null().into(), ty: target.clone() }),
                _ => Err(CodegenError {
                    message: format!(
                        "nullable 원시 타입({})의 코드 생성은 아직 지원되지 않습니다 (null 상태를 표현할 태그 표현이 설계되지 않음, SPEC.md §6)",
                        target
                    ),
                    line,
                }),
            };
        }
        if v.ty.ty == Ty::EmptyArrayLit {
            // 빈 배열의 물리 표현({null, 0})은 원소 타입과 무관하게 동일하므로 그대로 재사용한다.
            return Ok(TypedValue { value: v.value, ty: target.clone() });
        }
        self.numeric_cast(v, target, line)
    }

    fn numeric_cast(&self, v: TypedValue<'ctx>, target: &SemaType, line: usize) -> CResult<TypedValue<'ctx>> {
        let target_llvm = types::sema_ty_to_llvm(self.context, &self.class_structs, target);
        let out: BasicValueEnum = match target_llvm {
            BasicTypeEnum::IntType(it) => match v.value {
                BasicValueEnum::IntValue(iv) => self.builder.build_int_cast(iv, it, "cast").unwrap().into(),
                BasicValueEnum::FloatValue(fv) => {
                    if types::is_signed(&target.ty) {
                        self.builder.build_float_to_signed_int(fv, it, "cast").unwrap().into()
                    } else {
                        self.builder.build_float_to_unsigned_int(fv, it, "cast").unwrap().into()
                    }
                }
                _ => return Err(CodegenError { message: "지원되지 않는 숫자 변환입니다".into(), line }),
            },
            BasicTypeEnum::FloatType(ft) => match v.value {
                BasicValueEnum::FloatValue(fv) => self.builder.build_float_cast(fv, ft, "cast").unwrap().into(),
                BasicValueEnum::IntValue(iv) => {
                    if types::is_signed(&v.ty.ty) {
                        self.builder.build_signed_int_to_float(iv, ft, "cast").unwrap().into()
                    } else {
                        self.builder.build_unsigned_int_to_float(iv, ft, "cast").unwrap().into()
                    }
                }
                _ => return Err(CodegenError { message: "지원되지 않는 숫자 변환입니다".into(), line }),
            },
            _ => return Err(CodegenError { message: "지원되지 않는 캐스팅 조합입니다".into(), line }),
        };
        Ok(TypedValue { value: out, ty: target.clone() })
    }
}
