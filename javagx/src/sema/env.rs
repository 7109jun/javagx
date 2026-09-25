//! 함수/메서드 본문 내부의 지역 변수 스코프 스택.

use std::collections::HashMap;

use crate::sema::types::SemaType;

#[derive(Debug, Clone)]
pub struct VarBinding {
    pub ty: SemaType,
    pub mutable: bool,
}

pub struct Env {
    scopes: Vec<HashMap<String, VarBinding>>,
}

impl Env {
    pub fn new() -> Self {
        Env { scopes: vec![HashMap::new()] }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
        debug_assert!(!self.scopes.is_empty(), "최상위 스코프는 제거할 수 없습니다");
    }

    pub fn define(&mut self, name: impl Into<String>, binding: VarBinding) {
        self.scopes
            .last_mut()
            .expect("scope 스택은 항상 최소 1개 존재")
            .insert(name.into(), binding);
    }

    pub fn lookup(&self, name: &str) -> Option<&VarBinding> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}
