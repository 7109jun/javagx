//! JavaGX AST(추상 구문 트리) 노드 정의.
//!
//! SPEC.md §4 (EBNF)에 대응하는 타입들이다. 파서는 이 모듈의 타입만 생성하며,
//! 이후 단계(Sema, Codegen)는 오직 이 타입에만 의존한다.

/// 컴파일 단위 전체 (하나의 .jagx 파일).
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub items: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    ClassDecl(ClassDecl),
    FuncDecl(FuncDecl),
    VarDecl(VarDecl),
    If(IfStmt),
    While(WhileStmt),
    For(ForStmt),
    Return { value: Option<Expr>, line: usize },
    Break { line: usize },
    Continue { line: usize },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub superclass: Option<String>,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FuncDecl>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeAnnotation,
    pub is_pub: bool,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDecl {
    pub name: String,
    pub has_self: bool,
    pub is_pub: bool,
    pub params: Vec<Param>,
    pub ret_ty: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: TypeAnnotation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub name: String,
    pub ty: Option<TypeAnnotation>,
    pub value: Expr,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Let,
    Var,
    Const,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    /// (조건, 본문) 쌍의 목록. 첫 원소가 `if`, 나머지가 `elif` 절이다.
    pub branches: Vec<(Expr, Vec<Stmt>)>,
    pub else_body: Option<Vec<Stmt>>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStmt {
    pub cond: Expr,
    pub body: Vec<Stmt>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStmt {
    pub var_name: String,
    pub iter: Expr,
    pub body: Vec<Stmt>,
    pub line: usize,
}

/// 타입 주석: 기본 타입 + nullable 여부 (`?` 접미사).
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAnnotation {
    pub base: BaseType,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BaseType {
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Str,
    Char,
    Void,
    /// 사용자 정의 클래스 타입.
    Named(String),
    /// 배열 타입 (`T[]`). 원소는 nullable을 갖지 않는다 (v0.1 제약, SPEC.md §6).
    Array(Box<BaseType>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    CharLit(char),
    BoolLit(bool),
    Null,
    SelfExpr,
    Ident(String),
    New {
        class_name: String,
        args: Vec<Expr>,
        line: usize,
    },
    /// 배열 리터럴 `[e1, e2, ...]`. 빈 배열(`[]`)은 원소 타입을 스스로 알 수 없으므로
    /// 반드시 타입 주석이 있는 위치(`let x: i32[] = []`)에서만 쓰일 수 있다 (`null`과 동일한 패턴).
    ArrayLit {
        elements: Vec<Expr>,
        line: usize,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        line: usize,
    },
    Member {
        object: Box<Expr>,
        name: String,
        line: usize,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        line: usize,
    },
    Cast {
        expr: Box<Expr>,
        ty: TypeAnnotation,
        line: usize,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        line: usize,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        line: usize,
    },
    Assign {
        op: AssignOp,
        target: Box<Expr>,
        value: Box<Expr>,
        line: usize,
    },
    Grouping(Box<Expr>),
}

impl Expr {
    /// 이 표현식이 대입문의 좌변(lvalue)으로 쓰일 수 있는지 판정한다.
    pub fn is_lvalue(&self) -> bool {
        matches!(self, Expr::Ident(_) | Expr::Member { .. } | Expr::Index { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add, Sub, Mul, Div, Mod,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign, AddAssign, SubAssign, MulAssign, DivAssign,
}
