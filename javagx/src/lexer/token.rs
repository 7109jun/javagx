#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // 리터럴
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    CharLit(char),
    BoolLit(bool),
    Ident(String),

    // 키워드
    Class,
    SelfKw,
    New,
    Let,
    Var,
    Const,
    Fn,
    Return,
    If,
    Elif,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Null,
    Import,
    As,
    Pub,
    Priv,
    And,
    Or,
    Not,

    // 타입 키워드
    TyI8, TyI16, TyI32, TyI64,
    TyU8, TyU16, TyU32, TyU64,
    TyF32, TyF64,
    TyBool, TyStr, TyChar, TyVoid,

    // 연산자
    Plus, Minus, Star, Slash, Percent,
    Assign, PlusAssign, MinusAssign, StarAssign, SlashAssign,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    Bang, Question, Arrow,

    // 구두점
    Colon, Comma, Dot, Semicolon,
    LParen, RParen,
    LBracket, RBracket,

    // 구조
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

impl Token {
    pub fn new(kind: TokenKind, line: usize, col: usize) -> Self {
        Token { kind, line, col }
    }
}

pub fn lookup_keyword(ident: &str) -> Option<TokenKind> {
    use TokenKind::*;
    Some(match ident {
        "class" => Class,
        "self" => SelfKw,
        "new" => New,
        // "init"은 문법적 키워드가 아니라 생성자 메서드의 관례적 이름이다
        // (SPEC.md §5) — 렉서는 일반 식별자로 취급하고 Sema 단계에서 인식한다.
        "let" => Let,
        "var" => Var,
        "const" => Const,
        "fn" => Fn,
        "return" => Return,
        "if" => If,
        "elif" => Elif,
        "else" => Else,
        "while" => While,
        "for" => For,
        "in" => In,
        "break" => Break,
        "continue" => Continue,
        "null" => Null,
        "import" => Import,
        "as" => As,
        "pub" => Pub,
        "priv" => Priv,
        "and" => And,
        "or" => Or,
        "not" => Not,
        "true" => BoolLit(true),
        "false" => BoolLit(false),
        "i8" => TyI8, "i16" => TyI16, "i32" => TyI32, "i64" => TyI64,
        "u8" => TyU8, "u16" => TyU16, "u32" => TyU32, "u64" => TyU64,
        "f32" => TyF32, "f64" => TyF64,
        "bool" => TyBool, "str" => TyStr, "char" => TyChar, "void" => TyVoid,
        _ => return None,
    })
}
