//! AST → LLVM IR 코드 생성.
//!
//! **범위(v0.1)**: 최상위 `class`/`fn` 선언만 코드를 생성한다. 최상위의 그 외
//! 실행문(Sema는 문법적으로 허용)은 codegen 단계에서 명시적으로 거부한다 —
//! 네이티브 실행 파일은 `fn main() -> i32:` 진입점을 전제로 하기 때문이다.
//! 가상 디스패치(vtable), `for`/인덱싱(배열 타입 미정), nullable 원시 타입,
//! ARC retain/release 삽입은 모두 SPEC.md §6에 명시된 대로 이후 단계로 미룬다.

mod env;
mod expr;
mod runtime;
mod stmt;
pub mod types;

use inkwell::values::BasicValueEnum;

/// 코드 생성 중 표현식 하나의 결과: LLVM 값 + 그 값의 (Sema가 이미 검증한) 타입.
/// 타입을 함께 들고 다니는 이유는, LLVM IR 자체에는 부호/nullable 여부 같은
/// 정보가 없어 이후 연산(캐스팅, 산술 명령 선택)에 필요하기 때문이다.
#[derive(Clone)]
pub struct TypedValue<'ctx> {
    pub value: BasicValueEnum<'ctx>,
    pub ty: SemaType,
}

use std::collections::HashMap;
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::types::BasicType;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::values::FunctionValue;
use inkwell::OptimizationLevel;

use crate::parser::ast::*;
use crate::sema::class_table::ClassTable;
use crate::sema::types::{SemaType, Ty};
use crate::sema::FuncSig;
use types::{build_class_layouts, sema_ty_to_metadata, ClassLayout};

#[derive(Debug, Clone)]
pub struct CodegenError {
    pub message: String,
    pub line: usize,
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Codegen error at line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CodegenError {}

type CResult<T> = Result<T, CodegenError>;

pub struct Codegen<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    classes: &'ctx ClassTable,
    funcs: &'ctx HashMap<String, FuncSig>,
    class_layouts: HashMap<String, ClassLayout<'ctx>>,
    class_structs: HashMap<String, inkwell::types::StructType<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    methods: HashMap<(String, String), FunctionValue<'ctx>>,
    rt: runtime::Runtime<'ctx>,
}

/// `program`을 LLVM IR로 컴파일한다. Sema를 이미 통과한 프로그램이어야 한다
/// (타입 정합성은 검증되어 있다고 가정하고 codegen은 그 위에서 번역만 수행).
pub fn compile<'ctx>(
    context: &'ctx Context,
    module_name: &str,
    program: &Program,
    classes: &'ctx ClassTable,
    funcs: &'ctx HashMap<String, FuncSig>,
) -> CResult<Codegen<'ctx>> {
    let module = context.create_module(module_name);
    let builder = context.create_builder();
    let class_layouts = build_class_layouts(context, classes);
    let class_structs = class_layouts.iter().map(|(k, v)| (k.clone(), v.struct_ty)).collect();
    let rt = runtime::Runtime::declare(context, &module);

    let mut cg = Codegen {
        context,
        module,
        builder,
        classes,
        funcs,
        class_layouts,
        class_structs,
        functions: HashMap::new(),
        methods: HashMap::new(),
        rt,
    };

    cg.declare_all(program)?;
    cg.define_all(program)?;
    Ok(cg)
}

impl<'ctx> Codegen<'ctx> {
    fn void_or_basic_fn_type(
        &self,
        params: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
        ret: &SemaType,
    ) -> inkwell::types::FunctionType<'ctx> {
        if ret.ty == Ty::Void {
            self.context.void_type().fn_type(params, false)
        } else {
            let basic = types::sema_ty_to_llvm(self.context, &self.class_structs, ret);
            basic.fn_type(params, false)
        }
    }

    fn mangle_func(name: &str) -> String {
        if name == "main" {
            "main".to_string()
        } else {
            format!("jagx_fn_{}", name)
        }
    }

    fn mangle_method(class: &str, name: &str) -> String {
        format!("jagx_method_{}_{}", class, name)
    }

    /// 1단계: 모든 함수/메서드의 시그니처만 먼저 선언한다 (상호 재귀 호출 지원).
    fn declare_all(&mut self, program: &Program) -> CResult<()> {
        for item in &program.items {
            match item {
                Stmt::FuncDecl(f) => {
                    let param_tys: Vec<_> = f
                        .params
                        .iter()
                        .map(|p| {
                            let ty = sema_type_of(&p.ty);
                            sema_ty_to_metadata(self.context, &self.class_structs, &ty)
                        })
                        .collect();
                    let ret = f.ret_ty.as_ref().map(sema_type_of).unwrap_or(SemaType::non_null(Ty::Void));
                    let fn_ty = self.void_or_basic_fn_type(&param_tys, &ret);
                    let llvm_name = Self::mangle_func(&f.name);
                    let fv = self.module.add_function(&llvm_name, fn_ty, None);
                    self.functions.insert(f.name.clone(), fv);
                }
                Stmt::ClassDecl(c) => {
                    for m in &c.methods {
                        let mut param_tys = Vec::new();
                        if m.has_self {
                            // 오파크 포인터: pointee 타입 구분 없이 공통 포인터 타입을 사용한다.
                            let self_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                            param_tys.push(self_ty.into());
                        }
                        for p in &m.params {
                            let ty = sema_type_of(&p.ty);
                            param_tys.push(sema_ty_to_metadata(self.context, &self.class_structs, &ty));
                        }
                        let ret = m.ret_ty.as_ref().map(sema_type_of).unwrap_or(SemaType::non_null(Ty::Void));
                        let fn_ty = self.void_or_basic_fn_type(&param_tys, &ret);
                        let llvm_name = Self::mangle_method(&c.name, &m.name);
                        let fv = self.module.add_function(&llvm_name, fn_ty, None);
                        self.methods.insert((c.name.clone(), m.name.clone()), fv);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// 2단계: 본문을 채운다.
    fn define_all(&mut self, program: &Program) -> CResult<()> {
        for item in &program.items {
            match item {
                Stmt::FuncDecl(f) => self.define_func(f)?,
                Stmt::ClassDecl(c) => {
                    for m in &c.methods {
                        self.define_method(c, m)?;
                    }
                }
                other => {
                    return Err(CodegenError {
                        message: "최상위 실행문은 아직 코드 생성이 지원되지 않습니다 (class/fn 선언만 허용, main을 진입점으로 사용하세요)".into(),
                        line: top_level_stmt_line(other),
                    })
                }
            }
        }
        Ok(())
    }

    pub fn emit_object(&self, path: &Path) -> CResult<()> {
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|e| CodegenError { message: format!("타겟 초기화 실패: {}", e), line: 0 })?;
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple)
            .map_err(|e| CodegenError { message: format!("타겟 조회 실패: {}", e), line: 0 })?;
        let target_machine = target
            .create_target_machine(
                &triple,
                &TargetMachine::get_host_cpu_name().to_string(),
                &TargetMachine::get_host_cpu_features().to_string(),
                OptimizationLevel::Default,
                // 링커(cc)가 기본적으로 PIE 실행 파일을 생성하므로 위치 독립 코드로 생성해야 한다.
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| CodegenError { message: "타겟 머신 생성 실패".into(), line: 0 })?;

        self.module.verify().map_err(|e| CodegenError {
            message: format!("생성된 LLVM IR이 유효하지 않습니다 (컴파일러 내부 버그):\n{}", e),
            line: 0,
        })?;

        target_machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CodegenError { message: format!("오브젝트 파일 생성 실패: {}", e), line: 0 })
    }
}

/// `ast::TypeAnnotation` → `SemaType`. codegen 단계는 Sema를 통과한 입력만
/// 다루므로(클래스 존재 등은 이미 검증됨) 에러 없이 직접 변환한다.
pub(super) fn sema_type_of(ann: &TypeAnnotation) -> SemaType {
    SemaType::new(base_ty_of(&ann.base), ann.nullable)
}

fn base_ty_of(base: &BaseType) -> Ty {
    match base {
        BaseType::I8 => Ty::I8, BaseType::I16 => Ty::I16,
        BaseType::I32 => Ty::I32, BaseType::I64 => Ty::I64,
        BaseType::U8 => Ty::U8, BaseType::U16 => Ty::U16,
        BaseType::U32 => Ty::U32, BaseType::U64 => Ty::U64,
        BaseType::F32 => Ty::F32, BaseType::F64 => Ty::F64,
        BaseType::Bool => Ty::Bool,
        BaseType::Str => Ty::Str,
        BaseType::Char => Ty::Char,
        BaseType::Void => Ty::Void,
        BaseType::Named(n) => Ty::Class(n.clone()),
        BaseType::Array(elem) => Ty::Array(Box::new(base_ty_of(elem))),
    }
}

fn top_level_stmt_line(stmt: &Stmt) -> usize {
    match stmt {
        Stmt::VarDecl(v) => v.line,
        Stmt::If(i) => i.line,
        Stmt::While(w) => w.line,
        Stmt::For(f) => f.line,
        Stmt::Return { line, .. } | Stmt::Break { line } | Stmt::Continue { line } => *line,
        Stmt::Expr(e) => expr_line_pub(e),
        Stmt::ClassDecl(c) => c.line,
        Stmt::FuncDecl(f) => f.line,
    }
}

fn expr_line_pub(expr: &Expr) -> usize {
    match expr {
        Expr::New { line, .. }
        | Expr::Call { line, .. }
        | Expr::Member { line, .. }
        | Expr::Index { line, .. }
        | Expr::Cast { line, .. }
        | Expr::Unary { line, .. }
        | Expr::Binary { line, .. }
        | Expr::Assign { line, .. } => *line,
        Expr::Grouping(inner) => expr_line_pub(inner),
        _ => 0,
    }
}
