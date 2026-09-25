//! 의미 분석(Semantic Analysis): 심볼 테이블, 스코프, 정적 타입 검사.
//!
//! 파이프라인: `build_class_table` → `build_func_table` → 본문 타입 검사.
//! 첫 번째 에러에서 즉시 중단하는 fail-fast 방식이다 (다중 에러 수집/복구는
//! 향후 개선 과제로 남겨둔다).

pub mod class_table;
pub mod env;
pub mod types;

use std::collections::HashMap;

use crate::parser::ast::*;
use class_table::{build_class_table, ClassTable};
use env::{Env, VarBinding};
use types::{resolve_type, SemaType, Ty};

#[derive(Debug, Clone)]
pub struct SemaError {
    pub message: String,
    pub line: usize,
}

impl std::fmt::Display for SemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Semantic error at line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for SemaError {}

type SResult<T> = Result<T, SemaError>;

/// 최상위 함수(메서드가 아닌)의 시그니처.
#[derive(Debug, Clone)]
pub struct FuncSig {
    pub param_names: Vec<String>,
    pub param_types: Vec<SemaType>,
    pub ret: SemaType,
    pub line: usize,
}

/// 프로그램 전체에 대해 의미 분석을 수행한다. 성공하면 `Ok(())`,
/// 첫 번째로 발견된 오류가 있으면 `Err`를 반환한다.
pub fn check_program(program: &Program) -> SResult<()> {
    analyze(program).map(|_| ())
}

/// `check_program`과 동일한 검사를 수행하되, 이후 단계(Codegen)가 재사용할 수
/// 있도록 클래스/함수 테이블을 함께 반환한다.
pub fn analyze(program: &Program) -> SResult<(ClassTable, HashMap<String, FuncSig>)> {
    let classes = build_class_table(program)?;
    let funcs = build_func_table(program, &classes)?;
    let checker = Checker { classes: &classes, funcs: &funcs };

    let mut env = Env::new();
    for item in &program.items {
        match item {
            Stmt::ClassDecl(c) => checker.check_class(c)?,
            Stmt::FuncDecl(f) => checker.check_func(f)?,
            other => checker.check_stmt(other, &mut env, None, false, None)?,
        }
    }
    Ok((classes, funcs))
}

fn build_func_table(program: &Program, classes: &ClassTable) -> SResult<HashMap<String, FuncSig>> {
    let mut funcs = HashMap::new();
    for item in &program.items {
        if let Stmt::FuncDecl(f) = item {
            if funcs.contains_key(&f.name) {
                return Err(SemaError {
                    message: format!("함수 '{}'가 중복 선언되었습니다", f.name),
                    line: f.line,
                });
            }
            let mut param_types = Vec::new();
            let mut param_names = Vec::new();
            for p in &f.params {
                param_types.push(resolve_type(&p.ty, classes, f.line)?);
                param_names.push(p.name.clone());
            }
            let ret = match &f.ret_ty {
                Some(t) => resolve_type(t, classes, f.line)?,
                None => SemaType::non_null(Ty::Void),
            };
            funcs.insert(f.name.clone(), FuncSig { param_names, param_types, ret, line: f.line });
        }
    }
    Ok(funcs)
}

struct Checker<'a> {
    classes: &'a ClassTable,
    funcs: &'a HashMap<String, FuncSig>,
}

impl<'a> Checker<'a> {
    // -----------------------------------------------------------
    // 클래스 / 함수 단위 검사
    // -----------------------------------------------------------

    fn check_class(&self, class: &ClassDecl) -> SResult<()> {
        for method in &class.methods {
            self.check_method(class, method)?;
        }
        Ok(())
    }

    fn check_method(&self, class: &ClassDecl, method: &FuncDecl) -> SResult<()> {
        let mut env = Env::new();
        if method.has_self {
            env.define(
                "self",
                VarBinding { ty: SemaType::non_null(Ty::Class(class.name.clone())), mutable: false },
            );
        }
        for p in &method.params {
            let ty = resolve_type(&p.ty, self.classes, method.line)?;
            env.define(p.name.clone(), VarBinding { ty, mutable: true });
        }
        let ret = match &method.ret_ty {
            Some(t) => resolve_type(t, self.classes, method.line)?,
            None => SemaType::non_null(Ty::Void),
        };
        for s in &method.body {
            self.check_stmt(s, &mut env, Some(&ret), false, Some(&class.name))?;
        }
        if ret.ty != Ty::Void && !block_always_returns(&method.body) {
            return Err(SemaError {
                message: format!(
                    "메서드 '{}'의 모든 실행 경로가 값을 반환하지 않습니다 (반환 타입: {})",
                    method.name, ret
                ),
                line: method.line,
            });
        }
        Ok(())
    }

    fn check_func(&self, func: &FuncDecl) -> SResult<()> {
        let mut env = Env::new();
        for p in &func.params {
            let ty = resolve_type(&p.ty, self.classes, func.line)?;
            env.define(p.name.clone(), VarBinding { ty, mutable: true });
        }
        let ret = match &func.ret_ty {
            Some(t) => resolve_type(t, self.classes, func.line)?,
            None => SemaType::non_null(Ty::Void),
        };
        for s in &func.body {
            self.check_stmt(s, &mut env, Some(&ret), false, None)?;
        }
        if ret.ty != Ty::Void && !block_always_returns(&func.body) {
            return Err(SemaError {
                message: format!(
                    "함수 '{}'의 모든 실행 경로가 값을 반환하지 않습니다 (반환 타입: {})",
                    func.name, ret
                ),
                line: func.line,
            });
        }
        Ok(())
    }

    // -----------------------------------------------------------
    // 문장(statement) 검사
    // -----------------------------------------------------------

    fn check_stmt(
        &self,
        stmt: &Stmt,
        env: &mut Env,
        ret: Option<&SemaType>,
        in_loop: bool,
        current_class: Option<&str>,
    ) -> SResult<()> {
        match stmt {
            Stmt::ClassDecl(c) => Err(SemaError {
                message: "중첩된 class 선언은 아직 지원되지 않습니다 (최상위에서만 허용)".into(),
                line: c.line,
            }),
            Stmt::FuncDecl(f) => Err(SemaError {
                message: "중첩된 fn 선언은 아직 지원되지 않습니다 (최상위 또는 클래스 내부에서만 허용)".into(),
                line: f.line,
            }),
            Stmt::VarDecl(vd) => self.check_var_decl(vd, env, current_class),
            Stmt::If(if_stmt) => self.check_if(if_stmt, env, ret, in_loop, current_class),
            Stmt::While(w) => self.check_while(w, env, ret, current_class),
            Stmt::For(f) => self.check_for(f, env, ret, current_class),
            Stmt::Return { value, line } => self.check_return(value, *line, ret, env, current_class),
            Stmt::Break { line } => {
                if !in_loop {
                    return Err(SemaError { message: "루프 바깥에서 'break'를 사용할 수 없습니다".into(), line: *line });
                }
                Ok(())
            }
            Stmt::Continue { line } => {
                if !in_loop {
                    return Err(SemaError { message: "루프 바깥에서 'continue'를 사용할 수 없습니다".into(), line: *line });
                }
                Ok(())
            }
            Stmt::Expr(e) => {
                self.check_expr(e, env, current_class)?;
                Ok(())
            }
        }
    }

    fn check_var_decl(&self, vd: &VarDecl, env: &mut Env, current_class: Option<&str>) -> SResult<()> {
        let value_ty = self.check_expr(&vd.value, env, current_class)?;
        let final_ty = match &vd.ty {
            Some(ann) => {
                let declared = resolve_type(ann, self.classes, vd.line)?;
                if !assignable(&declared, &vd.value, &value_ty) {
                    return Err(SemaError {
                        message: format!(
                            "변수 '{}'의 선언 타입과 값의 타입이 일치하지 않습니다: {} vs {}",
                            vd.name, declared, value_ty
                        ),
                        line: vd.line,
                    });
                }
                declared
            }
            None => {
                if value_ty.ty == Ty::NullLit {
                    return Err(SemaError {
                        message: format!(
                            "'{}'의 타입을 추론할 수 없습니다 (null은 명시적 타입 주석이 필요합니다)",
                            vd.name
                        ),
                        line: vd.line,
                    });
                }
                if value_ty.ty == Ty::EmptyArrayLit {
                    return Err(SemaError {
                        message: format!(
                            "'{}'의 타입을 추론할 수 없습니다 (빈 배열 리터럴은 명시적 타입 주석이 필요합니다)",
                            vd.name
                        ),
                        line: vd.line,
                    });
                }
                value_ty
            }
        };
        env.define(vd.name.clone(), VarBinding { ty: final_ty, mutable: vd.kind == VarKind::Var });
        Ok(())
    }

    fn check_if(
        &self,
        if_stmt: &IfStmt,
        env: &mut Env,
        ret: Option<&SemaType>,
        in_loop: bool,
        current_class: Option<&str>,
    ) -> SResult<()> {
        for (cond, body) in &if_stmt.branches {
            self.expect_bool(cond, env, current_class)?;
            env.push_scope();
            for s in body {
                self.check_stmt(s, env, ret, in_loop, current_class)?;
            }
            env.pop_scope();
        }
        if let Some(body) = &if_stmt.else_body {
            env.push_scope();
            for s in body {
                self.check_stmt(s, env, ret, in_loop, current_class)?;
            }
            env.pop_scope();
        }
        Ok(())
    }

    fn check_while(&self, w: &WhileStmt, env: &mut Env, ret: Option<&SemaType>, current_class: Option<&str>) -> SResult<()> {
        self.expect_bool(&w.cond, env, current_class)?;
        env.push_scope();
        for s in &w.body {
            self.check_stmt(s, env, ret, true, current_class)?;
        }
        env.pop_scope();
        Ok(())
    }

    fn check_for(&self, f: &ForStmt, env: &mut Env, ret: Option<&SemaType>, current_class: Option<&str>) -> SResult<()> {
        let iter_ty = self.check_expr(&f.iter, env, current_class)?;
        let elem_ty = match iter_ty.ty {
            Ty::Array(elem) if !iter_ty.nullable => *elem,
            Ty::Array(_) => {
                return Err(SemaError { message: "nullable 배열은 null 검사 없이 순회할 수 없습니다".into(), line: f.line })
            }
            other => {
                return Err(SemaError {
                    message: format!("for 루프의 반복 대상은 배열이어야 합니다 (실제: {})", other),
                    line: f.line,
                })
            }
        };
        env.push_scope();
        env.define(f.var_name.clone(), VarBinding { ty: SemaType::non_null(elem_ty), mutable: true });
        for s in &f.body {
            self.check_stmt(s, env, ret, true, current_class)?;
        }
        env.pop_scope();
        Ok(())
    }

    fn check_return(
        &self,
        value: &Option<Expr>,
        line: usize,
        ret: Option<&SemaType>,
        env: &mut Env,
        current_class: Option<&str>,
    ) -> SResult<()> {
        let expected = ret.ok_or_else(|| SemaError {
            message: "함수/메서드 바깥에서는 'return'을 사용할 수 없습니다".into(),
            line,
        })?;
        match value {
            Some(e) => {
                let vty = self.check_expr(e, env, current_class)?;
                if !assignable(expected, e, &vty) {
                    return Err(SemaError {
                        message: format!("반환 타입이 일치하지 않습니다: 기대 {}, 실제 {}", expected, vty),
                        line,
                    });
                }
                Ok(())
            }
            None => {
                if expected.ty != Ty::Void || expected.nullable {
                    return Err(SemaError {
                        message: format!("값을 반환해야 합니다 (선언된 반환 타입: {})", expected),
                        line,
                    });
                }
                Ok(())
            }
        }
    }

    fn expect_bool(&self, expr: &Expr, env: &Env, current_class: Option<&str>) -> SResult<()> {
        let ty = self.check_expr(expr, env, current_class)?;
        if ty != SemaType::non_null(Ty::Bool) {
            return Err(SemaError {
                message: format!("조건식은 bool 타입이어야 합니다 (실제: {})", ty),
                line: expr_line(expr),
            });
        }
        Ok(())
    }

    // -----------------------------------------------------------
    // 표현식(expression) 검사
    // -----------------------------------------------------------

    fn check_expr(&self, expr: &Expr, env: &Env, current_class: Option<&str>) -> SResult<SemaType> {
        match expr {
            Expr::IntLit(_) => Ok(SemaType::non_null(Ty::I32)),
            Expr::FloatLit(_) => Ok(SemaType::non_null(Ty::F64)),
            Expr::StringLit(_) => Ok(SemaType::non_null(Ty::Str)),
            Expr::CharLit(_) => Ok(SemaType::non_null(Ty::Char)),
            Expr::BoolLit(_) => Ok(SemaType::non_null(Ty::Bool)),
            Expr::Null => Ok(SemaType::new(Ty::NullLit, true)),
            Expr::SelfExpr => env
                .lookup("self")
                .map(|b| b.ty.clone())
                .ok_or_else(|| SemaError { message: "'self'는 메서드 내부에서만 사용할 수 있습니다".into(), line: 0 }),
            Expr::Ident(name) => env.lookup(name).map(|b| b.ty.clone()).ok_or_else(|| SemaError {
                message: format!("정의되지 않은 변수입니다: '{}'", name),
                line: 0,
            }),
            Expr::Grouping(inner) => self.check_expr(inner, env, current_class),
            Expr::New { class_name, args, line } => self.check_new(class_name, args, *line, env, current_class),
            Expr::Call { callee, args, line } => self.check_call(callee, args, *line, env, current_class),
            Expr::Member { object, name, line } => self.check_member(object, name, *line, env, current_class),
            Expr::ArrayLit { elements, line } => self.check_array_lit(elements, *line, env, current_class),
            Expr::Index { object, index, line } => self.check_index(object, index, *line, env, current_class),
            Expr::Cast { expr, ty, line } => self.check_cast(expr, ty, *line, env, current_class),
            Expr::Unary { op, expr, line } => self.check_unary(*op, expr, *line, env, current_class),
            Expr::Binary { op, lhs, rhs, line } => self.check_binary(*op, lhs, rhs, *line, env, current_class),
            Expr::Assign { op, target, value, line } => {
                self.check_assign(*op, target, value, *line, env, current_class)
            }
        }
    }

    fn check_new(
        &self,
        class_name: &str,
        args: &[Expr],
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        let class = self.classes.get(class_name).ok_or_else(|| SemaError {
            message: format!("정의되지 않은 클래스입니다: '{}'", class_name),
            line,
        })?;
        match self.classes.resolve_method(class_name, "init") {
            Some(ctor) => {
                self.check_args(&ctor.param_types, args, line, env, current_class, "생성자")?;
            }
            None => {
                if !args.is_empty() {
                    return Err(SemaError {
                        message: format!("클래스 '{}'에는 생성자(init)가 없어 인자를 받을 수 없습니다", class.name),
                        line,
                    });
                }
            }
        }
        Ok(SemaType::non_null(Ty::Class(class_name.to_string())))
    }

    fn check_call(
        &self,
        callee: &Expr,
        args: &[Expr],
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        match callee {
            Expr::Member { object, name, .. } => {
                // "print"는 stdlib(6단계) 이전까지 임시로 제공되는 빌트인이 아니라
                // 인스턴스 메서드 호출 경로이므로 여기서는 처리하지 않는다.
                let obj_ty = self.check_expr(object, env, current_class)?;
                let cls = self.require_class(&obj_ty, line)?;
                let method = self.classes.resolve_method(cls, name).ok_or_else(|| SemaError {
                    message: format!("클래스 '{}'에 정의되지 않은 메서드입니다: '{}'", cls, name),
                    line,
                })?;
                // override가 pub을 생략해도 원래 선언의 가시성을 물려받는다.
                let is_pub = self.classes.resolve_method_base_visibility(cls, name).unwrap_or(method.is_pub);
                if !is_pub && current_class != Some(method.owner.as_str()) {
                    return Err(SemaError {
                        message: format!("private 메서드에 접근할 수 없습니다: '{}.{}'", method.owner, name),
                        line,
                    });
                }
                self.check_args(&method.param_types, args, line, env, current_class, name)?;
                Ok(method.ret.clone())
            }
            Expr::Ident(name) if name == "print" && !self.funcs.contains_key("print") => {
                // 임시 빌트인: stdlib(6단계)이 갖춰지기 전까지 print(value) 1개 인자를 허용.
                if args.len() != 1 {
                    return Err(SemaError {
                        message: "print()는 정확히 1개의 인자를 받습니다".into(),
                        line,
                    });
                }
                self.check_expr(&args[0], env, current_class)?;
                Ok(SemaType::non_null(Ty::Void))
            }
            Expr::Ident(name) => {
                let func = self.funcs.get(name).ok_or_else(|| SemaError {
                    message: format!("정의되지 않은 함수입니다: '{}'", name),
                    line,
                })?;
                self.check_args(&func.param_types, args, line, env, current_class, name)?;
                Ok(func.ret.clone())
            }
            _ => Err(SemaError { message: "호출할 수 없는 표현식입니다".into(), line }),
        }
    }

    fn check_args(
        &self,
        expected: &[SemaType],
        args: &[Expr],
        line: usize,
        env: &Env,
        current_class: Option<&str>,
        what: &str,
    ) -> SResult<()> {
        if expected.len() != args.len() {
            return Err(SemaError {
                message: format!(
                    "'{}' 호출의 인자 개수가 일치하지 않습니다: 기대 {}개, 실제 {}개",
                    what, expected.len(), args.len()
                ),
                line,
            });
        }
        for (i, (param_ty, arg)) in expected.iter().zip(args.iter()).enumerate() {
            let arg_ty = self.check_expr(arg, env, current_class)?;
            if !assignable(param_ty, arg, &arg_ty) {
                return Err(SemaError {
                    message: format!(
                        "'{}'의 {}번째 인자 타입이 일치하지 않습니다: 기대 {}, 실제 {}",
                        what, i + 1, param_ty, arg_ty
                    ),
                    line,
                });
            }
        }
        Ok(())
    }

    fn check_array_lit(&self, elements: &[Expr], line: usize, env: &Env, current_class: Option<&str>) -> SResult<SemaType> {
        if elements.is_empty() {
            // 빈 배열은 스스로 원소 타입을 알 수 없다 (null과 동일한 패턴) - 대입 위치의
            // 선언된 타입으로부터 결정되도록 마커 타입을 반환한다.
            return Ok(SemaType::new(Ty::EmptyArrayLit, false));
        }
        let first_ty = self.check_expr(&elements[0], env, current_class)?;
        if first_ty.nullable {
            return Err(SemaError { message: "배열 원소는 nullable 타입일 수 없습니다".into(), line });
        }
        for el in &elements[1..] {
            let el_ty = self.check_expr(el, env, current_class)?;
            if !assignable(&first_ty, el, &el_ty) {
                return Err(SemaError {
                    message: format!("배열 원소의 타입이 일치하지 않습니다: {} vs {}", first_ty, el_ty),
                    line,
                });
            }
        }
        Ok(SemaType::non_null(Ty::Array(Box::new(first_ty.ty))))
    }

    fn check_index(&self, object: &Expr, index: &Expr, line: usize, env: &Env, current_class: Option<&str>) -> SResult<SemaType> {
        let obj_ty = self.check_expr(object, env, current_class)?;
        let idx_ty = self.check_expr(index, env, current_class)?;
        if !idx_ty.is_integer() || idx_ty.nullable {
            return Err(SemaError { message: format!("배열 인덱스는 정수 타입이어야 합니다 (실제: {})", idx_ty), line });
        }
        match obj_ty.ty {
            Ty::Array(elem) if !obj_ty.nullable => Ok(SemaType::non_null(*elem)),
            Ty::Array(_) => Err(SemaError { message: "nullable 배열은 null 검사 없이 인덱싱할 수 없습니다".into(), line }),
            other => Err(SemaError { message: format!("배열 타입이 아니므로 인덱싱할 수 없습니다: {}", other), line }),
        }
    }

    fn check_member(
        &self,
        object: &Expr,
        name: &str,
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        let obj_ty = self.check_expr(object, env, current_class)?;
        if let Ty::Array(_) = &obj_ty.ty {
            if obj_ty.nullable {
                return Err(SemaError { message: "nullable 배열은 null 검사 없이 멤버에 접근할 수 없습니다".into(), line });
            }
            if name == "length" {
                return Ok(SemaType::non_null(Ty::I64));
            }
            return Err(SemaError { message: format!("배열에는 'length' 외의 필드가 없습니다: '{}'", name), line });
        }
        let cls = self.require_class(&obj_ty, line)?;
        match self.classes.resolve_field(cls, name) {
            Some(field) => {
                if !field.is_pub && current_class != Some(field.owner.as_str()) {
                    return Err(SemaError {
                        message: format!("private 필드에 접근할 수 없습니다: '{}.{}'", field.owner, name),
                        line,
                    });
                }
                Ok(field.ty.clone())
            }
            None => {
                if self.classes.resolve_method(cls, name).is_some() {
                    Err(SemaError {
                        message: format!("'{}'는 메서드입니다. 호출하려면 '()'가 필요합니다", name),
                        line,
                    })
                } else {
                    Err(SemaError {
                        message: format!("클래스 '{}'에 정의되지 않은 필드입니다: '{}'", cls, name),
                        line,
                    })
                }
            }
        }
    }

    fn check_cast(
        &self,
        expr: &Expr,
        ty: &TypeAnnotation,
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        let from = self.check_expr(expr, env, current_class)?;
        let to = resolve_type(ty, self.classes, line)?;
        if from.nullable || to.nullable {
            return Err(SemaError { message: "nullable 값은 캐스팅할 수 없습니다 (먼저 null 검사가 필요합니다)".into(), line });
        }
        if !from.is_numeric() || !to.is_numeric() {
            return Err(SemaError {
                message: format!("타입 캐스팅은 숫자 타입 간에만 허용됩니다: {} -> {}", from, to),
                line,
            });
        }
        Ok(to)
    }

    fn check_unary(&self, op: UnaryOp, expr: &Expr, line: usize, env: &Env, current_class: Option<&str>) -> SResult<SemaType> {
        let ty = self.check_expr(expr, env, current_class)?;
        match op {
            UnaryOp::Not => {
                if ty != SemaType::non_null(Ty::Bool) {
                    return Err(SemaError { message: format!("'not'/'!'은 bool 타입에만 사용할 수 있습니다 (실제: {})", ty), line });
                }
                Ok(SemaType::non_null(Ty::Bool))
            }
            UnaryOp::Neg => {
                if !ty.is_numeric() || ty.nullable {
                    return Err(SemaError { message: format!("단항 '-'는 숫자 타입에만 사용할 수 있습니다 (실제: {})", ty), line });
                }
                Ok(ty)
            }
        }
    }

    fn check_binary(
        &self,
        op: BinaryOp,
        lhs: &Expr,
        rhs: &Expr,
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        let lhs_ty = self.check_expr(lhs, env, current_class)?;
        let rhs_ty = self.check_expr(rhs, env, current_class)?;
        match op {
            // '+'는 숫자 타입뿐 아니라 str 타입의 연결(concatenation)도 허용한다 (SPEC.md §5 예제 참고).
            BinaryOp::Add if lhs_ty.ty == Ty::Str && rhs_ty.ty == Ty::Str && !lhs_ty.nullable && !rhs_ty.nullable => {
                Ok(SemaType::non_null(Ty::Str))
            }
            BinaryOp::Add => unify_numeric(lhs, &lhs_ty, rhs, &rhs_ty, line).map_err(|e| SemaError {
                message: format!(
                    "'+' 연산자는 숫자 타입 간 또는 str 타입 간에만 사용할 수 있습니다 (실제: {} + {})",
                    lhs_ty, rhs_ty
                ),
                line: e.line,
            }),
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                unify_numeric(lhs, &lhs_ty, rhs, &rhs_ty, line)
            }
            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                unify_numeric(lhs, &lhs_ty, rhs, &rhs_ty, line)?;
                Ok(SemaType::non_null(Ty::Bool))
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                check_equality_operands(lhs, &lhs_ty, rhs, &rhs_ty, line)?;
                Ok(SemaType::non_null(Ty::Bool))
            }
            BinaryOp::And | BinaryOp::Or => {
                let want = SemaType::non_null(Ty::Bool);
                if lhs_ty != want || rhs_ty != want {
                    return Err(SemaError {
                        message: format!("'and'/'or'의 피연산자는 bool 타입이어야 합니다 (실제: {} , {})", lhs_ty, rhs_ty),
                        line,
                    });
                }
                Ok(want)
            }
        }
    }

    fn check_assign(
        &self,
        op: AssignOp,
        target: &Expr,
        value: &Expr,
        line: usize,
        env: &Env,
        current_class: Option<&str>,
    ) -> SResult<SemaType> {
        let target_ty = match target {
            Expr::Ident(name) => {
                let binding = env.lookup(name).ok_or_else(|| SemaError {
                    message: format!("정의되지 않은 변수입니다: '{}'", name),
                    line,
                })?;
                if !binding.mutable {
                    return Err(SemaError {
                        message: format!("'{}'은(는) 재대입할 수 없습니다 (let/const로 선언됨, var를 사용하세요)", name),
                        line,
                    });
                }
                binding.ty.clone()
            }
            Expr::Member { object, name, .. } => {
                let obj_ty = self.check_expr(object, env, current_class)?;
                let cls = self.require_class(&obj_ty, line)?;
                let field = self.classes.resolve_field(cls, name).ok_or_else(|| SemaError {
                    message: format!("클래스 '{}'에 정의되지 않은 필드입니다: '{}'", cls, name),
                    line,
                })?;
                if !field.is_pub && current_class != Some(field.owner.as_str()) {
                    return Err(SemaError {
                        message: format!("private 필드에 접근할 수 없습니다: '{}.{}'", field.owner, name),
                        line,
                    });
                }
                field.ty.clone()
            }
            Expr::Index { object, index, .. } => self.check_index(object, index, line, env, current_class)?,
            _ => unreachable!("파서가 lvalue만 대입 대상으로 허용함"),
        };

        let value_ty = self.check_expr(value, env, current_class)?;
        match op {
            AssignOp::Assign => {
                if !assignable(&target_ty, value, &value_ty) {
                    return Err(SemaError {
                        message: format!("대입 타입이 일치하지 않습니다: {} <- {}", target_ty, value_ty),
                        line,
                    });
                }
            }
            AssignOp::AddAssign | AssignOp::SubAssign | AssignOp::MulAssign | AssignOp::DivAssign => {
                if !target_ty.is_numeric() {
                    return Err(SemaError {
                        message: format!("복합 대입 연산자는 숫자 타입에만 사용할 수 있습니다 (실제: {})", target_ty),
                        line,
                    });
                }
                if !assignable(&target_ty, value, &value_ty) {
                    return Err(SemaError {
                        message: format!("대입 타입이 일치하지 않습니다: {} <- {}", target_ty, value_ty),
                        line,
                    });
                }
            }
        }
        Ok(target_ty)
    }

    /// 값의 타입이 (non-null) 클래스 타입인지 확인하고 클래스 이름을 반환한다.
    fn require_class<'t>(&self, ty: &'t SemaType, line: usize) -> SResult<&'t str> {
        if ty.nullable {
            return Err(SemaError {
                message: format!("nullable 값 '{}'은(는) null 검사 없이 멤버에 접근할 수 없습니다", ty),
                line,
            });
        }
        match &ty.ty {
            Ty::Class(name) => Ok(name.as_str()),
            other => Err(SemaError { message: format!("'{}' 타입에는 멤버가 없습니다", other), line }),
        }
    }
}

// ---------------------------------------------------------------
// 타입 호환성 / 숫자 단일화 헬퍼 (자유 함수)
// ---------------------------------------------------------------

/// `value_expr`(타입 `value_ty`)가 `declared` 타입 위치에 대입될 수 있는지 판정한다.
/// 정수/실수 리터럴은 선언된 숫자 타입으로 자유롭게 승격(coercion)될 수 있다.
fn assignable(declared: &SemaType, value_expr: &Expr, value_ty: &SemaType) -> bool {
    if value_ty.ty == Ty::NullLit {
        return declared.nullable;
    }
    if value_ty.ty == Ty::EmptyArrayLit {
        return matches!(declared.ty, Ty::Array(_));
    }
    if declared.ty == value_ty.ty {
        // non-null 값을 nullable 위치에 넣는 것은 항상 허용된다.
        // nullable 값을 non-null 위치에 넣는 것은 금지된다.
        return !value_ty.nullable || declared.nullable;
    }
    match value_expr {
        Expr::IntLit(_) => declared.ty.is_numeric(),
        Expr::FloatLit(_) => declared.ty.is_float(),
        Expr::Grouping(inner) => assignable(declared, inner, value_ty),
        _ => false,
    }
}

fn is_literal_compatible(expr: &Expr, target: &Ty) -> bool {
    match expr {
        Expr::IntLit(_) => target.is_numeric(),
        Expr::FloatLit(_) => target.is_float(),
        Expr::Grouping(inner) => is_literal_compatible(inner, target),
        _ => false,
    }
}

/// 산술/비교 연산의 피연산자 타입을 단일화한다. 두 타입이 정확히 같으면
/// 그대로, 한쪽이 리터럴이면 다른 쪽 타입으로 승격해 반환한다.
fn unify_numeric(lhs_expr: &Expr, lhs_ty: &SemaType, rhs_expr: &Expr, rhs_ty: &SemaType, line: usize) -> SResult<SemaType> {
    if lhs_ty.nullable || rhs_ty.nullable {
        return Err(SemaError { message: "nullable 값은 산술/비교 연산에 직접 사용할 수 없습니다".into(), line });
    }
    if !lhs_ty.is_numeric() {
        return Err(SemaError { message: format!("숫자 타입이 아닙니다: {}", lhs_ty), line });
    }
    if !rhs_ty.is_numeric() {
        return Err(SemaError { message: format!("숫자 타입이 아닙니다: {}", rhs_ty), line });
    }
    if lhs_ty.ty == rhs_ty.ty {
        return Ok(SemaType::non_null(lhs_ty.ty.clone()));
    }
    if is_literal_compatible(lhs_expr, &rhs_ty.ty) {
        return Ok(SemaType::non_null(rhs_ty.ty.clone()));
    }
    if is_literal_compatible(rhs_expr, &lhs_ty.ty) {
        return Ok(SemaType::non_null(lhs_ty.ty.clone()));
    }
    Err(SemaError { message: format!("피연산자 타입이 일치하지 않습니다: {} vs {}", lhs_ty, rhs_ty), line })
}

fn check_equality_operands(lhs_expr: &Expr, lhs_ty: &SemaType, rhs_expr: &Expr, rhs_ty: &SemaType, line: usize) -> SResult<()> {
    if lhs_ty.ty == Ty::NullLit || rhs_ty.ty == Ty::NullLit {
        return Ok(()); // null과의 비교는 항상 허용 (엄격한 nullable 검사는 향후 개선 과제)
    }
    if lhs_ty.ty == rhs_ty.ty {
        return Ok(());
    }
    if is_literal_compatible(lhs_expr, &rhs_ty.ty) || is_literal_compatible(rhs_expr, &lhs_ty.ty) {
        return Ok(());
    }
    Err(SemaError { message: format!("'=='/'!='로 비교할 수 없는 타입입니다: {} vs {}", lhs_ty, rhs_ty), line })
}

/// 문장 목록이 실행되면 반드시 값을 반환하는지 보수적으로 판정한다.
/// (완전한 제어 흐름 분석이 아닌, "if/elif/else 전 분기가 모두 반환"하는
/// 단순한 경우만 인식하는 근사치다. while/for는 실행이 보장되지 않으므로 고려하지 않는다.)
fn block_always_returns(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_always_returns)
}

fn stmt_always_returns(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { value: Some(_), .. } => true,
        Stmt::If(if_stmt) => {
            if_stmt.else_body.is_some()
                && if_stmt.branches.iter().all(|(_, body)| block_always_returns(body))
                && block_always_returns(if_stmt.else_body.as_ref().unwrap())
        }
        _ => false,
    }
}

/// 에러 메시지에 쓸 대략적인 라인 번호를 표현식에서 추출한다.
/// 리터럴/식별자처럼 라인 정보를 직접 갖지 않는 노드는 0을 반환하며,
/// 이 경우 호출자가 문맥상 더 정확한 라인 번호로 감싸야 한다.
fn expr_line(expr: &Expr) -> usize {
    match expr {
        Expr::New { line, .. }
        | Expr::Call { line, .. }
        | Expr::Member { line, .. }
        | Expr::Index { line, .. }
        | Expr::Cast { line, .. }
        | Expr::Unary { line, .. }
        | Expr::Binary { line, .. }
        | Expr::Assign { line, .. } => *line,
        Expr::Grouping(inner) => expr_line(inner),
        _ => 0,
    }
}
