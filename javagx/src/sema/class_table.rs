//! 클래스 레지스트리: 클래스별 필드/메서드 시그니처와 상속 체인 조회를 담당한다.

use std::collections::{HashMap, HashSet};

use crate::parser::ast::{ClassDecl, Program, Stmt};
use crate::sema::types::{resolve_type, SemaType, Ty};
use crate::sema::SemaError;

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub ty: SemaType,
    pub is_pub: bool,
    /// 이 필드를 직접 선언한 클래스 이름 (가시성 검사에 사용).
    pub owner: String,
}

#[derive(Debug, Clone)]
pub struct MethodSig {
    pub has_self: bool,
    pub is_pub: bool,
    pub param_names: Vec<String>,
    pub param_types: Vec<SemaType>,
    pub ret: SemaType,
    /// 이 메서드를 직접 선언한 클래스 이름 (가시성 검사에 사용).
    pub owner: String,
}

#[derive(Debug, Clone)]
pub struct ClassInfo {
    pub name: String,
    pub superclass: Option<String>,
    /// 이 클래스에 "직접" 선언된 필드만 (상속된 필드 제외).
    pub fields: HashMap<String, FieldInfo>,
    /// 이 클래스에 "직접" 선언된 메서드만 (상속/오버라이드된 메서드 제외).
    pub methods: HashMap<String, MethodSig>,
    pub line: usize,
}

pub struct ClassTable {
    classes: HashMap<String, ClassInfo>,
}

impl ClassTable {
    pub fn new() -> Self {
        ClassTable { classes: HashMap::new() }
    }

    pub fn insert(&mut self, info: ClassInfo) {
        self.classes.insert(info.name.clone(), info);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.classes.contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<&ClassInfo> {
        self.classes.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.classes.keys().map(|s| s.as_str())
    }

    /// `class_name`부터 상속 체인을 따라 올라가며 필드를 찾는다.
    pub fn resolve_field(&self, class_name: &str, field_name: &str) -> Option<&FieldInfo> {
        let mut cur = Some(class_name);
        while let Some(name) = cur {
            let info = self.classes.get(name)?;
            if let Some(field) = info.fields.get(field_name) {
                return Some(field);
            }
            cur = info.superclass.as_deref();
        }
        None
    }

    /// `class_name`부터 상속 체인을 따라 올라가며 메서드를 찾는다.
    /// 서브클래스가 같은 이름의 메서드를 재정의했다면 서브클래스 쪽이 우선한다.
    pub fn resolve_method(&self, class_name: &str, method_name: &str) -> Option<&MethodSig> {
        let mut cur = Some(class_name);
        while let Some(name) = cur {
            let info = self.classes.get(name)?;
            if let Some(m) = info.methods.get(method_name) {
                return Some(m);
            }
            cur = info.superclass.as_deref();
        }
        None
    }

    /// `class_name`의 상속 체인에서 `method_name`이 "최초로" 선언된 지점의
    /// 가시성(pub 여부)을 반환한다. 서브클래스가 override하며 pub을 생략해도
    /// (SPEC.md §5 `Dog.speak` 예제처럼) 원래 선언의 가시성을 그대로 물려받도록
    /// 하기 위함 — override 시 가시성을 명시하지 않으면 축소되지 않는다.
    pub fn resolve_method_base_visibility(&self, class_name: &str, method_name: &str) -> Option<bool> {
        let mut cur = Some(class_name);
        let mut base_pub = None;
        while let Some(name) = cur {
            let info = self.classes.get(name)?;
            if let Some(m) = info.methods.get(method_name) {
                base_pub = Some(m.is_pub);
            }
            cur = info.superclass.as_deref();
        }
        base_pub
    }

    /// `class_name`이 `target`과 같거나 `target`의 서브클래스인지 판정한다.
    /// (필드/메서드 가시성 검사, 향후 업캐스팅 검사 등에 사용.)
    pub fn is_same_or_subclass(&self, class_name: &str, target: &str) -> bool {
        let mut cur = Some(class_name);
        while let Some(name) = cur {
            if name == target {
                return true;
            }
            cur = self.classes.get(name).and_then(|i| i.superclass.as_deref());
        }
        false
    }

    /// 상위 클래스 체인을 순회하며 사이클이 있으면 에러를 반환한다.
    pub fn check_no_cycles(&self) -> Result<(), SemaError> {
        for info in self.classes.values() {
            let mut seen = vec![info.name.clone()];
            let mut cur = info.superclass.clone();
            while let Some(name) = cur {
                if seen.contains(&name) {
                    return Err(SemaError {
                        message: format!(
                            "클래스 상속 관계에 순환이 있습니다: {} -> {}",
                            seen.join(" -> "),
                            name
                        ),
                        line: info.line,
                    });
                }
                seen.push(name.clone());
                cur = self.classes.get(&name).and_then(|i| i.superclass.clone());
            }
        }
        Ok(())
    }
}

impl Default for ClassTable {
    fn default() -> Self {
        Self::new()
    }
}

/// 프로그램에서 클래스 이름/상위클래스 존재 여부만 우선 검증한다.
/// (필드/메서드 타입 해석 전에, 상속 대상 클래스가 실재하는지 먼저 확인하기 위함.)
pub fn validate_superclass_refs(program: &Program) -> Result<(), SemaError> {
    let mut names = HashSet::new();
    for item in &program.items {
        if let Stmt::ClassDecl(c) = item {
            if !names.insert(c.name.clone()) {
                return Err(SemaError {
                    message: format!("클래스 '{}'가 중복 선언되었습니다", c.name),
                    line: c.line,
                });
            }
        }
    }
    for item in &program.items {
        if let Stmt::ClassDecl(c) = item {
            if let Some(sup) = &c.superclass {
                if !names.contains(sup) {
                    return Err(SemaError {
                        message: format!(
                            "클래스 '{}'가 정의되지 않은 클래스 '{}'를 상속합니다",
                            c.name, sup
                        ),
                        line: c.line,
                    });
                }
            }
        }
    }
    Ok(())
}

/// 프로그램의 모든 `class` 선언으로부터 완전한 `ClassTable`을 구축한다.
///
/// 1. 클래스 이름 중복 / 존재하지 않는 상위 클래스 참조를 검증
/// 2. 이름만 채운 스켈레톤을 등록해 상호 참조(타입으로서의 클래스 존재)를 가능케 함
/// 3. 상속 순환을 검사
/// 4. 상위 클래스 → 하위 클래스 순서로 필드/메서드 타입을 해석해 채움
pub fn build_class_table(program: &Program) -> Result<ClassTable, SemaError> {
    validate_superclass_refs(program)?;

    let mut decls: HashMap<String, &ClassDecl> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for item in &program.items {
        if let Stmt::ClassDecl(c) = item {
            decls.insert(c.name.clone(), c);
            order.push(c.name.clone());
        }
    }

    let mut table = ClassTable::new();
    for name in &order {
        let c = decls[name];
        table.insert(ClassInfo {
            name: c.name.clone(),
            superclass: c.superclass.clone(),
            fields: HashMap::new(),
            methods: HashMap::new(),
            line: c.line,
        });
    }
    table.check_no_cycles()?;

    let mut done: HashSet<String> = HashSet::new();
    for name in &order {
        fill_class(name, &decls, &mut table, &mut done)?;
    }
    Ok(table)
}

fn fill_class(
    name: &str,
    decls: &HashMap<String, &ClassDecl>,
    table: &mut ClassTable,
    done: &mut HashSet<String>,
) -> Result<(), SemaError> {
    if done.contains(name) {
        return Ok(());
    }
    let c = decls[name];
    if let Some(sup) = &c.superclass {
        fill_class(sup, decls, table, done)?;
    }

    let mut fields = HashMap::new();
    for f in &c.fields {
        if fields.contains_key(&f.name) {
            return Err(SemaError {
                message: format!("필드 '{}'가 클래스 '{}'에 중복 선언되었습니다", f.name, c.name),
                line: f.line,
            });
        }
        if let Some(sup) = &c.superclass {
            if table.resolve_field(sup, &f.name).is_some() {
                return Err(SemaError {
                    message: format!(
                        "필드 '{}'가 상위 클래스에 이미 선언되어 있어 재선언할 수 없습니다",
                        f.name
                    ),
                    line: f.line,
                });
            }
        }
        let ty = resolve_type(&f.ty, table, f.line)?;
        fields.insert(f.name.clone(), FieldInfo { ty, is_pub: f.is_pub, owner: c.name.clone() });
    }

    let mut methods = HashMap::new();
    for m in &c.methods {
        if methods.contains_key(&m.name) {
            return Err(SemaError {
                message: format!("메서드 '{}'가 클래스 '{}'에 중복 선언되었습니다", m.name, c.name),
                line: m.line,
            });
        }
        let mut param_types = Vec::new();
        let mut param_names = Vec::new();
        for p in &m.params {
            param_types.push(resolve_type(&p.ty, table, m.line)?);
            param_names.push(p.name.clone());
        }
        let ret = match &m.ret_ty {
            Some(t) => resolve_type(t, table, m.line)?,
            None => SemaType::non_null(Ty::Void),
        };
        methods.insert(
            m.name.clone(),
            MethodSig {
                has_self: m.has_self,
                is_pub: m.is_pub,
                param_names,
                param_types,
                ret,
                owner: c.name.clone(),
            },
        );
    }

    table.insert(ClassInfo {
        name: c.name.clone(),
        superclass: c.superclass.clone(),
        fields,
        methods,
        line: c.line,
    });
    done.insert(name.to_string());
    Ok(())
}
