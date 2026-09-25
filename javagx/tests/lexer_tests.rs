use javagxc::lexer::token::TokenKind::*;
use javagxc::lexer::Lexer;

fn kinds(src: &str) -> Vec<javagxc::lexer::token::TokenKind> {
    Lexer::new(src)
        .tokenize()
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

#[test]
fn test_basic_var_decl() {
    let toks = kinds("let x: i32 = 10\n");
    assert_eq!(
        toks,
        vec![Let, Ident("x".into()), Colon, TyI32, Assign, IntLit(10), Newline, Eof]
    );
}

#[test]
fn test_indent_dedent() {
    let src = "if x:\n    let y = 1\nlet z = 2\n";
    let toks = kinds(src);
    assert_eq!(
        toks,
        vec![
            If, Ident("x".into()), Colon, Newline,
            Indent,
            Let, Ident("y".into()), Assign, IntLit(1), Newline,
            Dedent,
            Let, Ident("z".into()), Assign, IntLit(2), Newline,
            Eof
        ]
    );
}

#[test]
fn test_nested_indent() {
    let src = "class A:\n    fn f(self):\n        return 1\n    return 2\n";
    let toks = kinds(src);
    assert_eq!(
        toks,
        vec![
            Class, Ident("A".into()), Colon, Newline,
            Indent,
            Fn, Ident("f".into()), LParen, SelfKw, RParen, Colon, Newline,
            Indent,
            Return, IntLit(1), Newline,
            Dedent,
            Return, IntLit(2), Newline,
            Dedent,
            Eof
        ]
    );
}

#[test]
fn test_class_and_self() {
    let toks = kinds("self.name = \"Rex\"\n");
    assert_eq!(
        toks,
        vec![SelfKw, Dot, Ident("name".into()), Assign, StringLit("Rex".into()), Newline, Eof]
    );
}

#[test]
fn test_operators() {
    let toks = kinds("a += 1\nb == c\nd -> e\nf: i32?\n");
    assert_eq!(
        toks,
        vec![
            Ident("a".into()), PlusAssign, IntLit(1), Newline,
            Ident("b".into()), Eq, Ident("c".into()), Newline,
            Ident("d".into()), Arrow, Ident("e".into()), Newline,
            Ident("f".into()), Colon, TyI32, Question, Newline,
            Eof
        ]
    );
}

#[test]
fn test_float_literal() {
    let toks = kinds("let ratio = 2.5\n");
    assert_eq!(toks, vec![Let, Ident("ratio".into()), Assign, FloatLit(2.5), Newline, Eof]);
}

#[test]
fn test_comment_and_blank_lines_ignored() {
    let src = "# comment\n\nlet x = 1\n\n# another\nlet y = 2\n";
    let toks = kinds(src);
    assert_eq!(
        toks,
        vec![
            Let, Ident("x".into()), Assign, IntLit(1), Newline,
            Let, Ident("y".into()), Assign, IntLit(2), Newline,
            Eof
        ]
    );
}

#[test]
fn test_bool_and_null() {
    let toks = kinds("let a = true\nlet b = false\nlet c = null\n");
    assert_eq!(
        toks,
        vec![
            Let, Ident("a".into()), Assign, BoolLit(true), Newline,
            Let, Ident("b".into()), Assign, BoolLit(false), Newline,
            Let, Ident("c".into()), Assign, Null, Newline,
            Eof
        ]
    );
}

#[test]
fn test_tab_indent_error() {
    let src = "if x:\n\tlet y = 1\n";
    let result = Lexer::new(src).tokenize();
    assert!(result.is_err());
}

#[test]
fn test_misaligned_indent_error() {
    let src = "if x:\n   let y = 1\n"; // 3칸 (4의 배수 아님)
    let result = Lexer::new(src).tokenize();
    assert!(result.is_err());
}
