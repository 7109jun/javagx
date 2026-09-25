//! JavaGX 재귀하강 파서.
//!
//! `Vec<Token>` (Lexer 출력)을 입력받아 `ast::Program`을 생성한다.
//! 문법 정의는 SPEC.md §4 EBNF를 따른다.

pub mod ast;

use crate::lexer::token::{Token, TokenKind};
use ast::*;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parse error at {}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

type PResult<T> = Result<T, ParseError>;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    pub fn parse_program(mut self) -> PResult<Program> {
        let mut items = Vec::new();
        self.skip_newlines();
        while !self.is_at_end() {
            items.push(self.parse_top_level_stmt()?);
            self.skip_newlines();
        }
        Ok(Program { items })
    }

    // ---------------------------------------------------------------
    // 토큰 스트림 유틸리티
    // ---------------------------------------------------------------

    fn peek(&self) -> &Token {
        // Eof는 항상 마지막에 존재하므로 인덱스 초과가 없다.
        self.tokens.get(self.pos).unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if !self.is_at_end() {
            self.pos += 1;
        }
        tok
    }

    /// 판별자(variant)만 비교하고 내부 데이터는 무시한다 (Ident, IntLit 등).
    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(self.peek_kind()) == std::mem::discriminant(kind)
    }

    fn match_kind(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind, what: &str) -> PResult<Token> {
        if self.check(kind) {
            Ok(self.advance())
        } else {
            let tok = self.peek();
            Err(ParseError {
                message: format!("{}를(을) 기대했지만 {:?}를 발견했습니다", what, tok.kind),
                line: tok.line,
                col: tok.col,
            })
        }
    }

    fn err_here(&self, msg: impl Into<String>) -> ParseError {
        let tok = self.peek();
        ParseError { message: msg.into(), line: tok.line, col: tok.col }
    }

    fn skip_newlines(&mut self) {
        while self.check(&TokenKind::Newline) {
            self.advance();
        }
    }

    fn current_line(&self) -> usize {
        self.peek().line
    }

    /// 식별자 토큰을 소비하고 문자열을 반환한다.
    fn expect_ident(&mut self, what: &str) -> PResult<String> {
        match self.peek_kind().clone() {
            TokenKind::Ident(s) => {
                self.advance();
                Ok(s)
            }
            _ => {
                let tok = self.peek();
                Err(ParseError {
                    message: format!("{}를(을) 기대했습니다", what),
                    line: tok.line,
                    col: tok.col,
                })
            }
        }
    }

    // ---------------------------------------------------------------
    // 최상위 / 문장(statement)
    // ---------------------------------------------------------------

    fn parse_top_level_stmt(&mut self) -> PResult<Stmt> {
        self.parse_stmt()
    }

    fn parse_stmt(&mut self) -> PResult<Stmt> {
        match self.peek_kind() {
            TokenKind::Class => self.parse_class_decl().map(Stmt::ClassDecl),
            TokenKind::Fn => self.parse_func_decl(false).map(Stmt::FuncDecl),
            TokenKind::Pub | TokenKind::Priv if self.peek_is_fn_after_visibility() => {
                self.parse_func_decl(false).map(Stmt::FuncDecl)
            }
            TokenKind::Let | TokenKind::Var | TokenKind::Const => {
                self.parse_var_decl().map(Stmt::VarDecl)
            }
            TokenKind::If => self.parse_if_stmt().map(Stmt::If),
            TokenKind::While => self.parse_while_stmt().map(Stmt::While),
            TokenKind::For => self.parse_for_stmt().map(Stmt::For),
            TokenKind::Return => self.parse_return_stmt(),
            TokenKind::Break => {
                let line = self.current_line();
                self.advance();
                self.expect_stmt_end()?;
                Ok(Stmt::Break { line })
            }
            TokenKind::Continue => {
                let line = self.current_line();
                self.advance();
                self.expect_stmt_end()?;
                Ok(Stmt::Continue { line })
            }
            _ => {
                let expr = self.parse_expr()?;
                self.expect_stmt_end()?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    /// `pub`/`priv` 다음이 `fn`인지 미리 살펴본다 (클래스 밖 최상위 함수의 가시성 표시 허용).
    fn peek_is_fn_after_visibility(&self) -> bool {
        matches!(self.tokens.get(self.pos + 1).map(|t| &t.kind), Some(TokenKind::Fn))
    }

    fn expect_stmt_end(&mut self) -> PResult<()> {
        if self.check(&TokenKind::Newline) || self.is_at_end() {
            self.skip_newlines();
            Ok(())
        } else {
            Err(self.err_here("줄바꿈(NEWLINE)을 기대했습니다"))
        }
    }

    // ---------------------------------------------------------------
    // 클래스 선언
    // ---------------------------------------------------------------

    fn parse_class_decl(&mut self) -> PResult<ClassDecl> {
        let line = self.current_line();
        self.expect(&TokenKind::Class, "'class'")?;
        let name = self.expect_ident("클래스 이름")?;

        let superclass = if self.match_kind(&TokenKind::LParen) {
            let sup = self.expect_ident("상위 클래스 이름")?;
            self.expect(&TokenKind::RParen, "')'")?;
            Some(sup)
        } else {
            None
        };

        self.expect(&TokenKind::Colon, "':'")?;
        self.expect(&TokenKind::Newline, "줄바꿈")?;
        self.expect(&TokenKind::Indent, "들여쓰기")?;

        let mut fields = Vec::new();
        let mut methods = Vec::new();

        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            let is_pub = self.match_kind(&TokenKind::Pub);
            let is_priv_explicit = if !is_pub { self.match_kind(&TokenKind::Priv) } else { false };
            let _ = is_priv_explicit; // priv는 기본값이므로 별도 플래그 불필요

            if self.check(&TokenKind::Fn) {
                methods.push(self.parse_func_decl(true).map(|mut f| { f.is_pub = is_pub; f })?);
            } else {
                fields.push(self.parse_field_decl(is_pub)?);
            }
        }

        self.expect(&TokenKind::Dedent, "들여쓰기 해제")?;
        Ok(ClassDecl { name, superclass, fields, methods, line })
    }

    fn parse_field_decl(&mut self, is_pub: bool) -> PResult<FieldDecl> {
        let line = self.current_line();
        let name = self.expect_ident("필드 이름")?;
        self.expect(&TokenKind::Colon, "':'")?;
        let ty = self.parse_type()?;
        self.expect_stmt_end()?;
        Ok(FieldDecl { name, ty, is_pub, line })
    }

    // ---------------------------------------------------------------
    // 함수 / 메서드 선언
    // ---------------------------------------------------------------

    fn parse_func_decl(&mut self, allow_self: bool) -> PResult<FuncDecl> {
        let line = self.current_line();
        // 클래스 본문 밖에서 pub/priv가 이미 소비됐을 수 있으므로 여기서는 fn부터.
        self.expect(&TokenKind::Fn, "'fn'")?;
        let name = self.expect_ident("함수 이름")?;
        self.expect(&TokenKind::LParen, "'('")?;

        let mut has_self = false;
        let mut params = Vec::new();

        if !self.check(&TokenKind::RParen) {
            if allow_self && self.check(&TokenKind::SelfKw) {
                self.advance();
                has_self = true;
                if self.match_kind(&TokenKind::Comma) {
                    params = self.parse_param_list()?;
                }
            } else {
                params = self.parse_param_list()?;
            }
        }
        self.expect(&TokenKind::RParen, "')'")?;

        let ret_ty = if self.match_kind(&TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&TokenKind::Colon, "':'")?;
        let body = self.parse_block()?;

        Ok(FuncDecl { name, has_self, is_pub: false, params, ret_ty, body, line })
    }

    fn parse_param_list(&mut self) -> PResult<Vec<Param>> {
        let mut params = vec![self.parse_param()?];
        while self.match_kind(&TokenKind::Comma) {
            params.push(self.parse_param()?);
        }
        Ok(params)
    }

    fn parse_param(&mut self) -> PResult<Param> {
        let name = self.expect_ident("매개변수 이름")?;
        self.expect(&TokenKind::Colon, "':'")?;
        let ty = self.parse_type()?;
        Ok(Param { name, ty })
    }

    // ---------------------------------------------------------------
    // 변수 선언
    // ---------------------------------------------------------------

    fn parse_var_decl(&mut self) -> PResult<VarDecl> {
        let line = self.current_line();
        let kind = match self.advance().kind {
            TokenKind::Let => VarKind::Let,
            TokenKind::Var => VarKind::Var,
            TokenKind::Const => VarKind::Const,
            _ => unreachable!("parse_var_decl은 let/var/const에서만 호출됨"),
        };
        let name = self.expect_ident("변수 이름")?;
        let ty = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::Assign, "'='")?;
        let value = self.parse_expr()?;
        self.expect_stmt_end()?;
        Ok(VarDecl { kind, name, ty, value, line })
    }

    // ---------------------------------------------------------------
    // 제어 흐름
    // ---------------------------------------------------------------

    fn parse_if_stmt(&mut self) -> PResult<IfStmt> {
        let line = self.current_line();
        self.expect(&TokenKind::If, "'if'")?;
        let mut branches = Vec::new();
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Colon, "':'")?;
        let body = self.parse_block()?;
        branches.push((cond, body));

        while self.check(&TokenKind::Elif) {
            self.advance();
            let cond = self.parse_expr()?;
            self.expect(&TokenKind::Colon, "':'")?;
            let body = self.parse_block()?;
            branches.push((cond, body));
        }

        let else_body = if self.match_kind(&TokenKind::Else) {
            self.expect(&TokenKind::Colon, "':'")?;
            Some(self.parse_block()?)
        } else {
            None
        };

        Ok(IfStmt { branches, else_body, line })
    }

    fn parse_while_stmt(&mut self) -> PResult<WhileStmt> {
        let line = self.current_line();
        self.expect(&TokenKind::While, "'while'")?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::Colon, "':'")?;
        let body = self.parse_block()?;
        Ok(WhileStmt { cond, body, line })
    }

    fn parse_for_stmt(&mut self) -> PResult<ForStmt> {
        let line = self.current_line();
        self.expect(&TokenKind::For, "'for'")?;
        let var_name = self.expect_ident("반복 변수 이름")?;
        self.expect(&TokenKind::In, "'in'")?;
        let iter = self.parse_expr()?;
        self.expect(&TokenKind::Colon, "':'")?;
        let body = self.parse_block()?;
        Ok(ForStmt { var_name, iter, body, line })
    }

    fn parse_return_stmt(&mut self) -> PResult<Stmt> {
        let line = self.current_line();
        self.expect(&TokenKind::Return, "'return'")?;
        let value = if self.check(&TokenKind::Newline) || self.is_at_end() {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect_stmt_end()?;
        Ok(Stmt::Return { value, line })
    }

    /// `NEWLINE INDENT { statement } DEDENT` 를 소비한다. 호출자는 이미 ':' 을 소비한 상태여야 한다.
    fn parse_block(&mut self) -> PResult<Vec<Stmt>> {
        self.expect(&TokenKind::Newline, "블록 시작 줄바꿈")?;
        self.expect(&TokenKind::Indent, "들여쓰기")?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::Dedent) && !self.is_at_end() {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::Dedent, "들여쓰기 해제")?;
        Ok(stmts)
    }

    // ---------------------------------------------------------------
    // 타입
    // ---------------------------------------------------------------

    fn parse_type(&mut self) -> PResult<TypeAnnotation> {
        let base = match self.peek_kind().clone() {
            TokenKind::TyI8 => { self.advance(); BaseType::I8 }
            TokenKind::TyI16 => { self.advance(); BaseType::I16 }
            TokenKind::TyI32 => { self.advance(); BaseType::I32 }
            TokenKind::TyI64 => { self.advance(); BaseType::I64 }
            TokenKind::TyU8 => { self.advance(); BaseType::U8 }
            TokenKind::TyU16 => { self.advance(); BaseType::U16 }
            TokenKind::TyU32 => { self.advance(); BaseType::U32 }
            TokenKind::TyU64 => { self.advance(); BaseType::U64 }
            TokenKind::TyF32 => { self.advance(); BaseType::F32 }
            TokenKind::TyF64 => { self.advance(); BaseType::F64 }
            TokenKind::TyBool => { self.advance(); BaseType::Bool }
            TokenKind::TyStr => { self.advance(); BaseType::Str }
            TokenKind::TyChar => { self.advance(); BaseType::Char }
            TokenKind::TyVoid => { self.advance(); BaseType::Void }
            TokenKind::Ident(name) => { self.advance(); BaseType::Named(name) }
            _ => return Err(self.err_here("타입 이름을 기대했습니다")),
        };
        // 배열 타입: `T[]` (다차원 `T[][]`도 반복 적용으로 허용).
        let mut base = base;
        while self.check(&TokenKind::LBracket)
            && matches!(self.tokens.get(self.pos + 1).map(|t| &t.kind), Some(TokenKind::RBracket))
        {
            self.advance(); // '['
            self.advance(); // ']'
            base = BaseType::Array(Box::new(base));
        }
        let nullable = self.match_kind(&TokenKind::Question);
        Ok(TypeAnnotation { base, nullable })
    }

    // ---------------------------------------------------------------
    // 표현식 (우선순위 climbing, SPEC.md §2)
    // ---------------------------------------------------------------

    fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> PResult<Expr> {
        let line = self.current_line();
        let lhs = self.parse_or()?;

        let op = match self.peek_kind() {
            TokenKind::Assign => Some(AssignOp::Assign),
            TokenKind::PlusAssign => Some(AssignOp::AddAssign),
            TokenKind::MinusAssign => Some(AssignOp::SubAssign),
            TokenKind::StarAssign => Some(AssignOp::MulAssign),
            TokenKind::SlashAssign => Some(AssignOp::DivAssign),
            _ => None,
        };

        if let Some(op) = op {
            if !lhs.is_lvalue() {
                return Err(self.err_here("대입 연산자의 좌변은 변수, 필드 또는 인덱스 표현식이어야 합니다"));
            }
            self.advance();
            let value = self.parse_assign()?; // 우결합
            return Ok(Expr::Assign { op, target: Box::new(lhs), value: Box::new(value), line });
        }
        Ok(lhs)
    }

    fn parse_or(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_and()?;
        while self.check(&TokenKind::Or) {
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_and()?;
            lhs = Expr::Binary { op: BinaryOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_equality()?;
        while self.check(&TokenKind::And) {
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_equality()?;
            lhs = Expr::Binary { op: BinaryOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_comparison()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_comparison()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_comparison(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => break,
            };
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_additive()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_multiplicative()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> PResult<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            let line = self.current_line();
            self.advance();
            let rhs = self.parse_unary()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), line };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> PResult<Expr> {
        let line = self.current_line();
        let op = match self.peek_kind() {
            TokenKind::Not | TokenKind::Bang => Some(UnaryOp::Not),
            TokenKind::Minus => Some(UnaryOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary { op, expr: Box::new(expr), line });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            let line = self.current_line();
            match self.peek_kind() {
                TokenKind::Dot => {
                    self.advance();
                    let name = self.expect_ident("멤버 이름")?;
                    expr = Expr::Member { object: Box::new(expr), name, line };
                }
                TokenKind::LParen => {
                    self.advance();
                    let args = self.parse_arg_list()?;
                    self.expect(&TokenKind::RParen, "')'")?;
                    expr = Expr::Call { callee: Box::new(expr), args, line };
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket, "']'")?;
                    expr = Expr::Index { object: Box::new(expr), index: Box::new(index), line };
                }
                TokenKind::As => {
                    self.advance();
                    let ty = self.parse_type()?;
                    expr = Expr::Cast { expr: Box::new(expr), ty, line };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_arg_list(&mut self) -> PResult<Vec<Expr>> {
        let mut args = Vec::new();
        if !self.check(&TokenKind::RParen) {
            args.push(self.parse_expr()?);
            while self.match_kind(&TokenKind::Comma) {
                args.push(self.parse_expr()?);
            }
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> PResult<Expr> {
        let line = self.current_line();
        match self.peek_kind().clone() {
            TokenKind::IntLit(v) => { self.advance(); Ok(Expr::IntLit(v)) }
            TokenKind::FloatLit(v) => { self.advance(); Ok(Expr::FloatLit(v)) }
            TokenKind::StringLit(s) => { self.advance(); Ok(Expr::StringLit(s)) }
            TokenKind::CharLit(c) => { self.advance(); Ok(Expr::CharLit(c)) }
            TokenKind::BoolLit(b) => { self.advance(); Ok(Expr::BoolLit(b)) }
            TokenKind::Null => { self.advance(); Ok(Expr::Null) }
            TokenKind::SelfKw => { self.advance(); Ok(Expr::SelfExpr) }
            TokenKind::New => {
                self.advance();
                let class_name = self.expect_ident("클래스 이름")?;
                self.expect(&TokenKind::LParen, "'('")?;
                let args = self.parse_arg_list()?;
                self.expect(&TokenKind::RParen, "')'")?;
                Ok(Expr::New { class_name, args, line })
            }
            TokenKind::Ident(name) => { self.advance(); Ok(Expr::Ident(name)) }
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "')'")?;
                Ok(Expr::Grouping(Box::new(inner)))
            }
            TokenKind::LBracket => {
                self.advance();
                let mut elements = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    elements.push(self.parse_expr()?);
                    while self.match_kind(&TokenKind::Comma) {
                        elements.push(self.parse_expr()?);
                    }
                }
                self.expect(&TokenKind::RBracket, "']'")?;
                Ok(Expr::ArrayLit { elements, line })
            }
            other => Err(ParseError {
                message: format!("표현식을 기대했지만 {:?}를 발견했습니다", other),
                line,
                col: self.peek().col,
            }),
        }
    }
}
