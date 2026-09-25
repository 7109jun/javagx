//! 함수/메서드 본문 코드 생성 중에만 존재하는 가변 상태: 지역 변수(alloca) 스코프와
//! 현재 루프의 continue/break 대상 블록 스택.

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::values::PointerValue;

use crate::sema::types::SemaType;

pub struct CgEnv<'ctx> {
    scopes: Vec<HashMap<String, (PointerValue<'ctx>, SemaType)>>,
}

impl<'ctx> CgEnv<'ctx> {
    pub fn new() -> Self {
        CgEnv { scopes: vec![HashMap::new()] }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn define(&mut self, name: impl Into<String>, ptr: PointerValue<'ctx>, ty: SemaType) {
        self.scopes.last_mut().expect("스코프 스택 비어있음").insert(name.into(), (ptr, ty));
    }

    pub fn lookup(&self, name: &str) -> Option<&(PointerValue<'ctx>, SemaType)> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }
}

impl<'ctx> Default for CgEnv<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}

/// 현재 함수 본문 코드 생성 상태.
pub struct FnCtx<'ctx> {
    pub env: CgEnv<'ctx>,
    /// (continue 대상, break 대상) 블록 스택 — 중첩 루프 지원.
    pub loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    /// 메서드 본문일 때 `self`가 가리키는 클래스 이름 (필드 조회에 사용).
    pub current_class: Option<String>,
}

impl<'ctx> FnCtx<'ctx> {
    pub fn new(current_class: Option<String>) -> Self {
        FnCtx { env: CgEnv::new(), loop_stack: Vec::new(), current_class }
    }
}
