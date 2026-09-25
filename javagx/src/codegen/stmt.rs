//! 문장(`Stmt`) 코드 생성: 함수/메서드 본문 빌드, 제어 흐름(if/while/return 등).

use inkwell::values::FunctionValue;
use inkwell::AddressSpace;

use crate::parser::ast::*;
use crate::sema::types::{SemaType, Ty};

use super::env::FnCtx;
use super::{sema_type_of, types, CodegenError, Codegen};

type CResult<T> = Result<T, CodegenError>;

impl<'ctx> Codegen<'ctx> {
    pub(super) fn define_func(&self, f: &FuncDecl) -> CResult<()> {
        let function = self.functions[&f.name];
        let ret = f.ret_ty.as_ref().map(sema_type_of).unwrap_or(SemaType::non_null(Ty::Void));
        let mut fx = FnCtx::new(None);
        self.build_function_body(function, &f.params, false, None, &f.body, &ret, &mut fx)
    }

    pub(super) fn define_method(&self, class: &ClassDecl, m: &FuncDecl) -> CResult<()> {
        let function = self.methods[&(class.name.clone(), m.name.clone())];
        let ret = m.ret_ty.as_ref().map(sema_type_of).unwrap_or(SemaType::non_null(Ty::Void));
        let mut fx = FnCtx::new(Some(class.name.clone()));
        self.build_function_body(function, &m.params, m.has_self, Some(&class.name), &m.body, &ret, &mut fx)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_function_body(
        &self,
        function: FunctionValue<'ctx>,
        params: &[Param],
        has_self: bool,
        self_class: Option<&str>,
        body: &[Stmt],
        ret_ty: &SemaType,
        fx: &mut FnCtx<'ctx>,
    ) -> CResult<()> {
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        let mut idx = 0u32;
        if has_self {
            let cls = self_class.expect("has_self이면 self_class가 있어야 함");
            // 오파크 포인터: 모든 클래스 포인터는 동일한 LLVM 포인터 타입을 공유하므로
            // pointee 타입(cls)은 GEP 등 실제 역참조 시점에만 필요하다.
            let self_llvm_ty = self.context.ptr_type(AddressSpace::default());
            let param_val = function.get_nth_param(idx).unwrap();
            let alloca = self.builder.build_alloca(self_llvm_ty, "self").unwrap();
            self.builder.build_store(alloca, param_val).unwrap();
            fx.env.define("self", alloca, SemaType::non_null(Ty::Class(cls.to_string())));
            idx += 1;
        }
        for p in params {
            let ty = sema_type_of(&p.ty);
            let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &ty);
            let param_val = function.get_nth_param(idx).unwrap();
            let alloca = self.builder.build_alloca(llvm_ty, &p.name).unwrap();
            self.builder.build_store(alloca, param_val).unwrap();
            fx.env.define(p.name.clone(), alloca, ty);
            idx += 1;
        }

        for s in body {
            self.gen_stmt(s, fx, ret_ty)?;
        }

        // void 함수가 명시적 return 없이 끝나면 보정한다.
        // (non-void인데 도달했다면 Sema의 "모든 경로 반환" 검사를 통과한 이상 발생하지 않아야 하는
        // 상황이므로 unreachable로 표시해 LLVM 검증을 통과시킨다.)
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            if ret_ty.ty == Ty::Void {
                self.builder.build_return(None).unwrap();
            } else {
                self.builder.build_unreachable().unwrap();
            }
        }
        Ok(())
    }

    fn gen_stmt(&self, stmt: &Stmt, fx: &mut FnCtx<'ctx>, ret_ty: &SemaType) -> CResult<()> {
        // 이전 문장이 이미 해당 블록을 종료(return/break/continue)했다면, 이후의
        // 문장은 도달 불가능한 코드이므로 생성을 건너뛴다 (LLVM은 terminator 이후
        // 명령어를 허용하지 않는다).
        if self.builder.get_insert_block().unwrap().get_terminator().is_some() {
            return Ok(());
        }
        match stmt {
            Stmt::VarDecl(vd) => self.gen_var_decl(vd, fx),
            Stmt::If(i) => self.gen_if(i, fx, ret_ty),
            Stmt::While(w) => self.gen_while(w, fx, ret_ty),
            Stmt::For(f) => self.gen_for(f, fx, ret_ty),
            Stmt::Return { value, line } => self.gen_return(value, *line, ret_ty, fx),
            Stmt::Break { line } => {
                let (_, brk) = fx
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or_else(|| CodegenError { message: "루프 바깥에서 'break'를 사용할 수 없습니다".into(), line: *line })?;
                self.builder.build_unconditional_branch(brk).unwrap();
                Ok(())
            }
            Stmt::Continue { line } => {
                let (cont, _) = fx
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or_else(|| CodegenError { message: "루프 바깥에서 'continue'를 사용할 수 없습니다".into(), line: *line })?;
                self.builder.build_unconditional_branch(cont).unwrap();
                Ok(())
            }
            Stmt::Expr(e) => {
                self.gen_expr(e, fx)?;
                Ok(())
            }
            Stmt::ClassDecl(c) => {
                Err(CodegenError { message: "중첩된 class 선언은 지원되지 않습니다".into(), line: c.line })
            }
            Stmt::FuncDecl(f) => {
                Err(CodegenError { message: "중첩된 fn 선언은 지원되지 않습니다".into(), line: f.line })
            }
        }
    }

    fn gen_var_decl(&self, vd: &VarDecl, fx: &mut FnCtx<'ctx>) -> CResult<()> {
        let val = self.gen_expr(&vd.value, fx)?;
        let declared_ty = vd.ty.as_ref().map(sema_type_of).unwrap_or_else(|| val.ty.clone());
        let coerced = self.coerce(val, &declared_ty, vd.line)?;
        let llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &declared_ty);
        let alloca = self.builder.build_alloca(llvm_ty, &vd.name).unwrap();
        self.builder.build_store(alloca, coerced.value).unwrap();
        fx.env.define(vd.name.clone(), alloca, declared_ty);
        Ok(())
    }

    fn gen_if(&self, if_stmt: &IfStmt, fx: &mut FnCtx<'ctx>, ret_ty: &SemaType) -> CResult<()> {
        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let merge_bb = self.context.append_basic_block(function, "if.end");

        for (i, (cond, body)) in if_stmt.branches.iter().enumerate() {
            let cond_val = self.gen_expr(cond, fx)?.value.into_int_value();
            let then_bb = self.context.append_basic_block(function, &format!("if.then{}", i));
            let else_bb = self.context.append_basic_block(function, &format!("if.else{}", i));
            self.builder.build_conditional_branch(cond_val, then_bb, else_bb).unwrap();

            self.builder.position_at_end(then_bb);
            fx.env.push_scope();
            for s in body {
                self.gen_stmt(s, fx, ret_ty)?;
            }
            fx.env.pop_scope();
            if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                self.builder.build_unconditional_branch(merge_bb).unwrap();
            }

            self.builder.position_at_end(else_bb);
        }

        if let Some(else_body) = &if_stmt.else_body {
            fx.env.push_scope();
            for s in else_body {
                self.gen_stmt(s, fx, ret_ty)?;
            }
            fx.env.pop_scope();
        }
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder.build_unconditional_branch(merge_bb).unwrap();
        }

        self.builder.position_at_end(merge_bb);
        Ok(())
    }

    fn gen_while(&self, w: &WhileStmt, fx: &mut FnCtx<'ctx>, ret_ty: &SemaType) -> CResult<()> {
        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let cond_bb = self.context.append_basic_block(function, "while.cond");
        let body_bb = self.context.append_basic_block(function, "while.body");
        let end_bb = self.context.append_basic_block(function, "while.end");

        self.builder.build_unconditional_branch(cond_bb).unwrap();
        self.builder.position_at_end(cond_bb);
        let cond_val = self.gen_expr(&w.cond, fx)?.value.into_int_value();
        self.builder.build_conditional_branch(cond_val, body_bb, end_bb).unwrap();

        self.builder.position_at_end(body_bb);
        fx.loop_stack.push((cond_bb, end_bb));
        fx.env.push_scope();
        for s in &w.body {
            self.gen_stmt(s, fx, ret_ty)?;
        }
        fx.env.pop_scope();
        fx.loop_stack.pop();
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder.build_unconditional_branch(cond_bb).unwrap();
        }

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn gen_for(&self, f: &ForStmt, fx: &mut FnCtx<'ctx>, ret_ty: &SemaType) -> CResult<()> {
        let iter = self.gen_expr(&f.iter, fx)?;
        let elem_ty = match &iter.ty.ty {
            Ty::Array(e) => SemaType::non_null((**e).clone()),
            other => return Err(CodegenError { message: format!("for 루프의 반복 대상은 배열이어야 합니다 (실제: {})", other), line: f.line }),
        };
        let sv = iter.value.into_struct_value();
        let data_ptr = self.builder.build_extract_value(sv, 0, "for.data").unwrap().into_pointer_value();
        let len = self.builder.build_extract_value(sv, 1, "for.len").unwrap().into_int_value();
        let elem_llvm_ty = types::sema_ty_to_llvm(self.context, &self.class_structs, &elem_ty);

        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let cond_bb = self.context.append_basic_block(function, "for.cond");
        let body_bb = self.context.append_basic_block(function, "for.body");
        let inc_bb = self.context.append_basic_block(function, "for.inc");
        let end_bb = self.context.append_basic_block(function, "for.end");

        let idx_alloca = self.builder.build_alloca(self.context.i64_type(), "for.idx").unwrap();
        self.builder.build_store(idx_alloca, self.context.i64_type().const_zero()).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(cond_bb);
        let idx_val = self.builder.build_load(self.context.i64_type(), idx_alloca, "for.idx.v").unwrap().into_int_value();
        let cmp = self.builder.build_int_compare(inkwell::IntPredicate::ULT, idx_val, len, "for.cmp").unwrap();
        self.builder.build_conditional_branch(cmp, body_bb, end_bb).unwrap();

        self.builder.position_at_end(body_bb);
        let elem_ptr = unsafe { self.builder.build_gep(elem_llvm_ty, data_ptr, &[idx_val], "for.elem_ptr").unwrap() };
        let elem_val = self.builder.build_load(elem_llvm_ty, elem_ptr, "for.elem").unwrap();
        let loop_var_alloca = self.builder.build_alloca(elem_llvm_ty, &f.var_name).unwrap();
        self.builder.build_store(loop_var_alloca, elem_val).unwrap();

        fx.env.push_scope();
        fx.env.define(f.var_name.clone(), loop_var_alloca, elem_ty);
        fx.loop_stack.push((inc_bb, end_bb));
        for s in &f.body {
            self.gen_stmt(s, fx, ret_ty)?;
        }
        fx.loop_stack.pop();
        fx.env.pop_scope();
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder.build_unconditional_branch(inc_bb).unwrap();
        }

        self.builder.position_at_end(inc_bb);
        let cur = self.builder.build_load(self.context.i64_type(), idx_alloca, "for.cur").unwrap().into_int_value();
        let one = self.context.i64_type().const_int(1, false);
        let next = self.builder.build_int_add(cur, one, "for.next").unwrap();
        self.builder.build_store(idx_alloca, next).unwrap();
        self.builder.build_unconditional_branch(cond_bb).unwrap();

        self.builder.position_at_end(end_bb);
        Ok(())
    }

    fn gen_return(&self, value: &Option<Expr>, line: usize, ret_ty: &SemaType, fx: &mut FnCtx<'ctx>) -> CResult<()> {
        match value {
            Some(e) => {
                let val = self.gen_expr(e, fx)?;
                let coerced = self.coerce(val, ret_ty, line)?;
                self.builder.build_return(Some(&coerced.value)).unwrap();
            }
            None => {
                self.builder.build_return(None).unwrap();
            }
        }
        Ok(())
    }
}
