//! 최소 런타임: 외부 libc 함수 선언 + `str` 연결(concatenation) 헬퍼.
//!
//! v0.1은 별도의 stdlib(6단계)이 없으므로, `+`에 의한 문자열 연결과
//! `print()` 빌트인이 요구하는 최소한의 동작만 여기서 직접 IR로 정의한다.

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

use super::types::str_struct_type;

pub struct Runtime<'ctx> {
    pub malloc: FunctionValue<'ctx>,
    pub memcpy: FunctionValue<'ctx>,
    pub memcmp: FunctionValue<'ctx>,
    pub printf: FunctionValue<'ctx>,
    pub str_concat: FunctionValue<'ctx>,
}

impl<'ctx> Runtime<'ctx> {
    pub fn declare(context: &'ctx Context, module: &Module<'ctx>) -> Self {
        let i8ptr = context.ptr_type(AddressSpace::default());
        let i64ty = context.i64_type();
        let i32ty = context.i32_type();

        let malloc_ty = i8ptr.fn_type(&[i64ty.into()], false);
        let malloc = module.add_function("malloc", malloc_ty, None);

        let memcpy_ty = i8ptr.fn_type(&[i8ptr.into(), i8ptr.into(), i64ty.into()], false);
        let memcpy = module.add_function("memcpy", memcpy_ty, None);

        let memcmp_ty = i32ty.fn_type(&[i8ptr.into(), i8ptr.into(), i64ty.into()], false);
        let memcmp = module.add_function("memcmp", memcmp_ty, None);

        let printf_ty = i32ty.fn_type(&[i8ptr.into()], true);
        let printf = module.add_function("printf", printf_ty, None);

        let str_ty = str_struct_type(context);
        let str_concat_ty = str_ty.fn_type(&[str_ty.into(), str_ty.into()], false);
        let str_concat = module.add_function("jagx_str_concat", str_concat_ty, None);

        let rt = Runtime { malloc, memcpy, memcmp, printf, str_concat };
        rt.define_str_concat(context, &str_ty);
        rt
    }

    /// `jagx_str_concat({p1,l1}, {p2,l2}) -> {p1p2, l1+l2}` 본문을 직접 IR로 정의한다.
    fn define_str_concat(&self, context: &'ctx Context, str_ty: &inkwell::types::StructType<'ctx>) {
        let builder = context.create_builder();
        let entry = context.append_basic_block(self.str_concat, "entry");
        builder.position_at_end(entry);

        let a = self.str_concat.get_nth_param(0).unwrap().into_struct_value();
        let b = self.str_concat.get_nth_param(1).unwrap().into_struct_value();

        let p1 = builder.build_extract_value(a, 0, "p1").unwrap().into_pointer_value();
        let l1 = builder.build_extract_value(a, 1, "l1").unwrap().into_int_value();
        let p2 = builder.build_extract_value(b, 0, "p2").unwrap().into_pointer_value();
        let l2 = builder.build_extract_value(b, 1, "l2").unwrap().into_int_value();

        let total = builder.build_int_add(l1, l2, "total_len").unwrap();
        let buf = builder
            .build_call(self.malloc, &[total.into()], "buf")
            .unwrap()
            .try_as_basic_value()
            .left()
            .unwrap()
            .into_pointer_value();

        builder.build_call(self.memcpy, &[buf.into(), p1.into(), l1.into()], "cpy1").unwrap();
        let buf2 = unsafe { builder.build_gep(context.i8_type(), buf, &[l1], "buf_off").unwrap() };
        builder.build_call(self.memcpy, &[buf2.into(), p2.into(), l2.into()], "cpy2").unwrap();

        let undef = str_ty.get_undef();
        let with_ptr = builder.build_insert_value(undef, buf, 0, "s0").unwrap();
        let result = builder.build_insert_value(with_ptr, total, 1, "s1").unwrap();
        builder.build_return(Some(&result.into_struct_value())).unwrap();
    }
}
