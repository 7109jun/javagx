pub mod token;

use token::{lookup_keyword, Token, TokenKind};

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lex error at {}:{}: {}", self.line, self.col, self.message)
    }
}

pub struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    at_line_start: bool,
    paren_depth: i32, // ( [ 안에서는 줄바꿈이 NEWLINE을 발행하지 않음
}

const INDENT_WIDTH: usize = 4;

impl Lexer {
    pub fn new(source: &str) -> Self {
        Lexer {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
            indent_stack: vec![0],
            at_line_start: true,
            paren_depth: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn err(&self, msg: impl Into<String>) -> LexError {
        LexError { message: msg.into(), line: self.line, col: self.col }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            if self.at_line_start && self.paren_depth == 0 && self.handle_line_start(&mut tokens)? {
                // handle_line_start가 true를 반환하면 빈 줄/주석 줄이므로 재시도
                continue;
            }

            self.skip_inline_whitespace();

            let (line, col) = (self.line, self.col);
            let c = match self.peek() {
                Some(c) => c,
                None => break,
            };

            if c == '#' {
                self.skip_comment();
                continue;
            }

            if c == '\n' {
                self.advance();
                if self.paren_depth == 0 {
                    tokens.push(Token::new(TokenKind::Newline, line, col));
                    self.at_line_start = true;
                }
                continue;
            }

            if c.is_ascii_digit() {
                tokens.push(self.lex_number()?);
                continue;
            }

            if c == '"' {
                tokens.push(self.lex_string()?);
                continue;
            }

            if c == '\'' {
                tokens.push(self.lex_char()?);
                continue;
            }

            if c.is_alphabetic() || c == '_' {
                tokens.push(self.lex_ident_or_keyword());
                continue;
            }

            tokens.push(self.lex_operator()?);
        }

        // EOF: 마지막 줄 NEWLINE 보정 + 남은 DEDENT 모두 발행
        if let Some(last) = tokens.last() {
            if last.kind != TokenKind::Newline {
                tokens.push(Token::new(TokenKind::Newline, self.line, self.col));
            }
        }
        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            tokens.push(Token::new(TokenKind::Dedent, self.line, self.col));
        }
        tokens.push(Token::new(TokenKind::Eof, self.line, self.col));

        Ok(tokens)
    }

    /// 줄 시작에서 들여쓰기를 측정해 INDENT/DEDENT를 발행한다.
    /// 빈 줄이거나 주석만 있는 줄이면 true를 반환해 상위 루프가 재시도하게 한다.
    fn handle_line_start(&mut self, tokens: &mut Vec<Token>) -> Result<bool, LexError> {
        let start_line = self.line;
        let mut width = 0usize;

        loop {
            match self.peek() {
                Some(' ') => { width += 1; self.advance(); }
                Some('\t') => {
                    return Err(self.err("탭 문자는 들여쓰기에 사용할 수 없습니다 (스페이스 4칸 사용)"));
                }
                _ => break,
            }
        }

        // 빈 줄이거나 주석만 있는 줄은 들여쓰기 판정에서 제외
        match self.peek() {
            None => {
                self.at_line_start = false;
                return Ok(false);
            }
            Some('\n') => {
                self.advance();
                return Ok(true);
            }
            Some('#') => {
                self.skip_comment();
                if self.peek() == Some('\n') {
                    self.advance();
                }
                return Ok(true);
            }
            _ => {}
        }

        if width % INDENT_WIDTH != 0 {
            return Err(LexError {
                message: format!(
                    "들여쓰기는 4칸 단위여야 합니다 (현재 {}칸)",
                    width
                ),
                line: start_line,
                col: width + 1,
            });
        }

        let current = *self.indent_stack.last().unwrap();
        match width.cmp(&current) {
            std::cmp::Ordering::Greater => {
                self.indent_stack.push(width);
                tokens.push(Token::new(TokenKind::Indent, start_line, width + 1));
            }
            std::cmp::Ordering::Less => {
                while *self.indent_stack.last().unwrap() > width {
                    self.indent_stack.pop();
                    tokens.push(Token::new(TokenKind::Dedent, start_line, width + 1));
                }
                if *self.indent_stack.last().unwrap() != width {
                    return Err(LexError {
                        message: "들여쓰기 레벨이 일치하지 않습니다".into(),
                        line: start_line,
                        col: width + 1,
                    });
                }
            }
            std::cmp::Ordering::Equal => {}
        }

        self.at_line_start = false;
        Ok(false)
    }

    fn skip_inline_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' { break; }
            self.advance();
        }
    }

    fn lex_number(&mut self) -> Result<Token, LexError> {
        let (line, col) = (self.line, self.col);
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() { s.push(c); self.advance(); } else { break; }
        }
        if self.peek() == Some('.') && self.peek_at(1).map_or(false, |c| c.is_ascii_digit()) {
            s.push('.');
            self.advance();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() { s.push(c); self.advance(); } else { break; }
            }
            let val: f64 = s.parse().map_err(|_| self.err("잘못된 실수 리터럴"))?;
            return Ok(Token::new(TokenKind::FloatLit(val), line, col));
        }
        let val: i64 = s.parse().map_err(|_| self.err("잘못된 정수 리터럴"))?;
        Ok(Token::new(TokenKind::IntLit(val), line, col))
    }

    fn lex_string(&mut self) -> Result<Token, LexError> {
        let (line, col) = (self.line, self.col);
        self.advance(); // opening "
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('\n') => return Err(self.err("문자열이 닫히지 않았습니다")),
                Some('"') => { self.advance(); break; }
                Some('\\') => {
                    self.advance();
                    let esc = self.advance().ok_or_else(|| self.err("잘못된 이스케이프"))?;
                    s.push(match esc {
                        'n' => '\n', 't' => '\t', 'r' => '\r',
                        '\\' => '\\', '"' => '"', '0' => '\0',
                        other => return Err(self.err(format!("알 수 없는 이스케이프: \\{}", other))),
                    });
                }
                Some(c) => { s.push(c); self.advance(); }
            }
        }
        Ok(Token::new(TokenKind::StringLit(s), line, col))
    }

    fn lex_char(&mut self) -> Result<Token, LexError> {
        let (line, col) = (self.line, self.col);
        self.advance(); // opening '
        let c = match self.advance() {
            Some('\\') => {
                let esc = self.advance().ok_or_else(|| self.err("잘못된 이스케이프"))?;
                match esc {
                    'n' => '\n', 't' => '\t', 'r' => '\r',
                    '\\' => '\\', '\'' => '\'', '0' => '\0',
                    other => return Err(self.err(format!("알 수 없는 이스케이프: \\{}", other))),
                }
            }
            Some(c) => c,
            None => return Err(self.err("문자 리터럴이 닫히지 않았습니다")),
        };
        if self.advance() != Some('\'') {
            return Err(self.err("문자 리터럴은 단일 문자여야 합니다"));
        }
        Ok(Token::new(TokenKind::CharLit(c), line, col))
    }

    fn lex_ident_or_keyword(&mut self) -> Token {
        let (line, col) = (self.line, self.col);
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' { s.push(c); self.advance(); } else { break; }
        }
        let kind = lookup_keyword(&s).unwrap_or(TokenKind::Ident(s));
        Token::new(kind, line, col)
    }

    fn lex_operator(&mut self) -> Result<Token, LexError> {
        use TokenKind::*;
        let (line, col) = (self.line, self.col);
        let c = self.advance().unwrap();

        macro_rules! two {
            ($next:expr, $two_kind:expr, $one_kind:expr) => {
                if self.peek() == Some($next) {
                    self.advance();
                    $two_kind
                } else {
                    $one_kind
                }
            };
        }

        let kind = match c {
            '+' => two!('=', PlusAssign, Plus),
            '-' => {
                if self.peek() == Some('>') { self.advance(); Arrow }
                else { two!('=', MinusAssign, Minus) }
            }
            '*' => two!('=', StarAssign, Star),
            '/' => two!('=', SlashAssign, Slash),
            '%' => Percent,
            '=' => two!('=', Eq, Assign),
            '!' => two!('=', NotEq, Bang),
            '<' => two!('=', LtEq, Lt),
            '>' => two!('=', GtEq, Gt),
            '?' => Question,
            ':' => Colon,
            ',' => Comma,
            '.' => Dot,
            ';' => Semicolon,
            '(' => { self.paren_depth += 1; LParen }
            ')' => { self.paren_depth -= 1; RParen }
            '[' => { self.paren_depth += 1; LBracket }
            ']' => { self.paren_depth -= 1; RBracket }
            other => return Err(LexError {
                message: format!("알 수 없는 문자: '{}'", other),
                line, col,
            }),
        };
        Ok(Token::new(kind, line, col))
    }
}
