use javagxc::lexer::Lexer;
use javagxc::parser::ast::*;
use javagxc::parser::Parser;

fn parse(src: &str) -> Program {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    Parser::new(tokens).parse_program().expect("parse failed")
}

fn parse_err(src: &str) -> String {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    Parser::new(tokens)
        .parse_program()
        .expect_err("parse가 성공하면 안 됨")
        .to_string()
}

#[test]
fn test_var_decl_with_type() {
    let prog = parse("let x: i32 = 10\n");
    assert_eq!(prog.items.len(), 1);
    match &prog.items[0] {
        Stmt::VarDecl(vd) => {
            assert_eq!(vd.kind, VarKind::Let);
            assert_eq!(vd.name, "x");
            assert_eq!(vd.ty, Some(TypeAnnotation { base: BaseType::I32, nullable: false }));
            assert_eq!(vd.value, Expr::IntLit(10));
        }
        other => panic!("expected VarDecl, got {:?}", other),
    }
}

#[test]
fn test_var_decl_inferred() {
    let prog = parse("var y = 3.5\n");
    match &prog.items[0] {
        Stmt::VarDecl(vd) => {
            assert_eq!(vd.kind, VarKind::Var);
            assert_eq!(vd.ty, None);
            assert_eq!(vd.value, Expr::FloatLit(3.5));
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_nullable_type() {
    let prog = parse("let x: i32? = null\n");
    match &prog.items[0] {
        Stmt::VarDecl(vd) => {
            assert_eq!(vd.ty, Some(TypeAnnotation { base: BaseType::I32, nullable: true }));
            assert_eq!(vd.value, Expr::Null);
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_operator_precedence() {
    // 1 + 2 * 3 == 7  →  Add(1, Mul(2,3))
    let prog = parse("let r = 1 + 2 * 3\n");
    let Stmt::VarDecl(vd) = &prog.items[0] else { panic!() };
    match &vd.value {
        Expr::Binary { op: BinaryOp::Add, lhs, rhs, .. } => {
            assert_eq!(**lhs, Expr::IntLit(1));
            match rhs.as_ref() {
                Expr::Binary { op: BinaryOp::Mul, lhs, rhs, .. } => {
                    assert_eq!(**lhs, Expr::IntLit(2));
                    assert_eq!(**rhs, Expr::IntLit(3));
                }
                other => panic!("unexpected rhs: {:?}", other),
            }
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_logical_precedence() {
    // a or b and c  →  Or(a, And(b, c))  (and가 or보다 우선순위 높음)
    let prog = parse("let r = a or b and c\n");
    let Stmt::VarDecl(vd) = &prog.items[0] else { panic!() };
    match &vd.value {
        Expr::Binary { op: BinaryOp::Or, lhs, rhs, .. } => {
            assert_eq!(**lhs, Expr::Ident("a".into()));
            assert!(matches!(rhs.as_ref(), Expr::Binary { op: BinaryOp::And, .. }));
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_class_with_field_and_method() {
    let src = "\
class Animal:
    pub name: str

    fn init(self, name: str):
        self.name = name

    pub fn speak(self) -> str:
        return self.name
";
    let prog = parse(src);
    assert_eq!(prog.items.len(), 1);
    let Stmt::ClassDecl(class) = &prog.items[0] else { panic!() };
    assert_eq!(class.name, "Animal");
    assert_eq!(class.superclass, None);
    assert_eq!(class.fields.len(), 1);
    assert_eq!(class.fields[0].name, "name");
    assert!(class.fields[0].is_pub);
    assert_eq!(class.methods.len(), 2);
    assert_eq!(class.methods[0].name, "init");
    assert!(class.methods[0].has_self);
    assert_eq!(class.methods[1].name, "speak");
    assert!(class.methods[1].is_pub);
    assert_eq!(
        class.methods[1].ret_ty,
        Some(TypeAnnotation { base: BaseType::Str, nullable: false })
    );
}

#[test]
fn test_class_inheritance() {
    let src = "class Dog(Animal):\n    fn speak(self) -> str:\n        return \"bark\"\n";
    let prog = parse(src);
    let Stmt::ClassDecl(class) = &prog.items[0] else { panic!() };
    assert_eq!(class.superclass, Some("Animal".to_string()));
}

#[test]
fn test_new_expr_and_call_chain() {
    let src = "let d = new Dog(\"Rex\", 3)\nd.speak()\n";
    let prog = parse(src);
    let Stmt::VarDecl(vd) = &prog.items[0] else { panic!() };
    match &vd.value {
        Expr::New { class_name, args, .. } => {
            assert_eq!(class_name, "Dog");
            assert_eq!(args.len(), 2);
        }
        other => panic!("unexpected: {:?}", other),
    }
    match &prog.items[1] {
        Stmt::Expr(Expr::Call { callee, args, .. }) => {
            assert!(matches!(callee.as_ref(), Expr::Member { name, .. } if name == "speak"));
            assert_eq!(args.len(), 0);
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_if_elif_else() {
    let src = "\
if a:
    let x = 1
elif b:
    let x = 2
else:
    let x = 3
";
    let prog = parse(src);
    let Stmt::If(if_stmt) = &prog.items[0] else { panic!() };
    assert_eq!(if_stmt.branches.len(), 2);
    assert!(if_stmt.else_body.is_some());
}

#[test]
fn test_while_and_for() {
    let prog = parse("while x:\n    x = x - 1\nfor i in items:\n    print(i)\n");
    assert!(matches!(prog.items[0], Stmt::While(_)));
    assert!(matches!(prog.items[1], Stmt::For(_)));
}

#[test]
fn test_assign_ops() {
    let prog = parse("x += 1\nx -= 1\nx *= 2\nx /= 2\n");
    for (item, expected) in prog.items.iter().zip([
        AssignOp::AddAssign,
        AssignOp::SubAssign,
        AssignOp::MulAssign,
        AssignOp::DivAssign,
    ]) {
        match item {
            Stmt::Expr(Expr::Assign { op, .. }) => assert_eq!(*op, expected),
            other => panic!("unexpected: {:?}", other),
        }
    }
}

#[test]
fn test_invalid_assign_target_is_error() {
    let msg = parse_err("1 + 2 = 3\n");
    assert!(msg.contains("대입"), "unexpected message: {msg}");
}

#[test]
fn test_index_and_member_chain() {
    let prog = parse("let v = arr[0].value\n");
    let Stmt::VarDecl(vd) = &prog.items[0] else { panic!() };
    match &vd.value {
        Expr::Member { object, name, .. } => {
            assert_eq!(name, "value");
            assert!(matches!(object.as_ref(), Expr::Index { .. }));
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_cast_expr() {
    let prog = parse("let b: i32 = a as i32\n");
    let Stmt::VarDecl(vd) = &prog.items[0] else { panic!() };
    assert!(matches!(vd.value, Expr::Cast { .. }));
}

#[test]
fn test_full_program_from_spec_example() {
    let src = "\
class Animal:
    pub name: str
    priv age: i32

    fn init(self, name: str, age: i32):
        self.name = name
        self.age = age

    pub fn speak(self) -> str:
        return self.name + \" makes a sound\"

class Dog(Animal):
    fn speak(self) -> str:
        return self.name + \" barks\"

fn main() -> i32:
    let d = new Dog(\"Rex\", 3)
    print(d.speak())
    return 0
";
    let prog = parse(src);
    assert_eq!(prog.items.len(), 3);
    assert!(matches!(prog.items[0], Stmt::ClassDecl(_)));
    assert!(matches!(prog.items[1], Stmt::ClassDecl(_)));
    match &prog.items[2] {
        Stmt::FuncDecl(f) => {
            assert_eq!(f.name, "main");
            assert!(!f.has_self);
            assert_eq!(f.body.len(), 3);
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn test_missing_colon_is_error() {
    let msg = parse_err("if x\n    let y = 1\n");
    assert!(msg.contains("':'"), "unexpected message: {msg}");
}
