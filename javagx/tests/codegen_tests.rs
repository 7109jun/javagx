//! 4단계(Codegen) 통합 테스트.
//!
//! 각 테스트는 JavaGX 소스를 실제로 LLVM IR → 오브젝트 파일 → 네이티브 실행 파일까지
//! 컴파일링크하고, 그 실행 파일을 직접 실행해 종료 코드/표준출력을 검증한다.
//! (유닛 테스트가 아니라 "진짜 동작하는 컴파일러"임을 증명하는 end-to-end 테스트.)

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use inkwell::context::Context;
use javagxc::codegen;
use javagxc::lexer::Lexer;
use javagxc::parser::Parser;
use javagxc::sema;

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct RunResult {
    exit_code: i32,
    stdout: String,
}

fn compile_and_run(src: &str) -> RunResult {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let base = std::env::temp_dir().join(format!("javagx_test_{}_{}", std::process::id(), n));
    let obj_path = base.with_extension("o");
    let exe_path = base.clone();

    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let program = Parser::new(tokens).parse_program().expect("parse failed");
    let (classes, funcs) = sema::analyze(&program).expect("sema failed");

    let context = Context::create();
    let cg = codegen::compile(&context, "test_module", &program, &classes, &funcs).expect("codegen failed");
    cg.emit_object(&obj_path).expect("object emission failed");

    let link = Command::new("cc")
        .arg(&obj_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("링커(cc) 실행 실패");
    assert!(link.success(), "링킹 실패");
    let _ = std::fs::remove_file(&obj_path);

    let output = Command::new(&exe_path).output().expect("실행 파일 구동 실패");
    let _ = std::fs::remove_file(&exe_path);

    RunResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
    }
}

fn compile_expect_err(src: &str) -> String {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let program = Parser::new(tokens).parse_program().expect("parse failed");
    let (classes, funcs) = sema::analyze(&program).expect("sema failed");
    let context = Context::create();
    let msg = match codegen::compile(&context, "test_module", &program, &classes, &funcs) {
        Ok(_) => panic!("codegen이 성공하면 안 됨"),
        Err(e) => e.to_string(),
    };
    msg
}

// -----------------------------------------------------------------
// 산술 / 제어 흐름
// -----------------------------------------------------------------

#[test]
fn test_return_constant() {
    let r = compile_and_run("fn main() -> i32:\n    return 42\n");
    assert_eq!(r.exit_code, 42);
}

#[test]
fn test_arithmetic() {
    let r = compile_and_run("fn main() -> i32:\n    return 2 + 3 * 4\n");
    assert_eq!(r.exit_code, 14);
}

#[test]
fn test_function_call_and_literal_widening() {
    let src = "\
fn add(a: i64, b: i64) -> i64:
    return a + b

fn main() -> i32:
    let r = add(3, 4)
    return r as i32
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 7);
}

#[test]
fn test_while_loop() {
    let src = "\
fn main() -> i32:
    var i = 0
    var sum = 0
    while i < 5:
        sum += i
        i += 1
    return sum
";
    let r = compile_and_run(src); // 0+1+2+3+4 = 10
    assert_eq!(r.exit_code, 10);
}

#[test]
fn test_if_elif_else_all_branches() {
    let src = "\
fn classify(x: i32) -> i32:
    if x < 0:
        return -1
    elif x == 0:
        return 0
    else:
        return 1

fn main() -> i32:
    var total = 0
    total += classify(-5)
    total += classify(0)
    total += classify(9)
    return total
";
    let r = compile_and_run(src); // -1 + 0 + 1 = 0
    assert_eq!(r.exit_code, 0);
}

#[test]
fn test_break_and_continue() {
    let src = "\
fn main() -> i32:
    var i = 0
    var sum = 0
    while i < 10:
        i += 1
        if i == 3:
            continue
        if i > 6:
            break
        sum += i
    return sum
";
    // i=1,2 (3 skipped),4,5,6 합산 = 1+2+4+5+6 = 18
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 18);
}

#[test]
fn test_bool_logic() {
    let src = "\
fn main() -> i32:
    let a = true
    let b = false
    if a and not b:
        return 1
    return 0
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 1);
}

// -----------------------------------------------------------------
// 클래스 / 상속 / 문자열
// -----------------------------------------------------------------

#[test]
fn test_spec_full_example_runs_and_prints() {
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
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 0);
    assert_eq!(r.stdout, "Rex barks\n");
}

#[test]
fn test_field_mutation_via_method() {
    let src = "\
class Counter:
    pub count: i32

    fn init(self):
        self.count = 0

    pub fn increment(self):
        self.count += 1

fn main() -> i32:
    let c = new Counter()
    c.increment()
    c.increment()
    c.increment()
    return c.count
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 3);
}

#[test]
fn test_inherited_field_and_method() {
    let src = "\
class Animal:
    pub name: str
    fn init(self, name: str):
        self.name = name

class Dog(Animal):
    pub fn bark(self) -> str:
        return self.name + \"!\"

fn main() -> i32:
    let d = new Dog(\"Rex\")
    print(d.bark())
    return 0
";
    let r = compile_and_run(src);
    assert_eq!(r.stdout, "Rex!\n");
}

#[test]
fn test_nullable_class_field() {
    let src = "\
class Node:
    pub value: i32
    pub next: Node?

    fn init(self, v: i32):
        self.value = v
        self.next = null

fn main() -> i32:
    let a = new Node(1)
    let b = new Node(2)
    a.next = b
    if a.next == null:
        return 100
    if b.next == null:
        return a.value
    return -1
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 1);
}

// -----------------------------------------------------------------
// 캐스팅
// -----------------------------------------------------------------

#[test]
fn test_numeric_cast_truncation() {
    let src = "\
fn main() -> i32:
    let a: i64 = 10
    let b: i32 = a as i32
    return b
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 10);
}

#[test]
fn test_int_to_float_cast_and_back() {
    let src = "\
fn main() -> i32:
    let a: i32 = 7
    let f: f64 = a as f64
    let back: i32 = f as i32
    return back
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 7);
}

// -----------------------------------------------------------------
// 코드 생성 단계에서 명시적으로 거부되는 미지원 기능 (SPEC.md §6)
// -----------------------------------------------------------------

#[test]
fn test_nullable_primitive_null_assignment_rejected() {
    let src = "fn f():\n    let x: i32? = null\n    return\n";
    let msg = compile_expect_err(src);
    assert!(msg.contains("nullable 원시 타입"), "unexpected: {msg}");
}

// -----------------------------------------------------------------
// 배열 / 문자열 비교 (6단계: stdlib 최소셋)
// -----------------------------------------------------------------

#[test]
fn test_str_equality_true_and_false() {
    let src = "\
fn main() -> i32:
    let a = \"hello\"
    let b = \"hello\"
    let c = \"world\"
    if a == b:
        if a != c:
            return 1
    return 0
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 1);
}

#[test]
fn test_array_literal_index_and_length() {
    let src = "\
fn main() -> i32:
    let nums = [10, 20, 30, 40]
    var sum = 0
    var i: i64 = 0
    while i < nums.length:
        sum += nums[i]
        i += 1
    return sum
";
    let r = compile_and_run(src); // 10+20+30+40 = 100
    assert_eq!(r.exit_code, 100);
}

#[test]
fn test_for_loop_over_array() {
    let src = "\
fn main() -> i32:
    var total = 0
    for x in [1, 2, 3, 4, 5]:
        total += x
    return total
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 15);
}

#[test]
fn test_array_index_assignment() {
    let src = "\
fn main() -> i32:
    let a = [1, 2, 3]
    a[1] = 99
    return a[0] + a[1] + a[2]
";
    let r = compile_and_run(src); // 1+99+3
    assert_eq!(r.exit_code, 103);
}

#[test]
fn test_empty_array_with_declared_type() {
    let src = "\
fn main() -> i32:
    let a: i32[] = []
    return a.length as i32
";
    let r = compile_and_run(src);
    assert_eq!(r.exit_code, 0);
}

#[test]
fn test_array_of_str_and_print() {
    let src = "\
fn main() -> i32:
    let names = [\"Rex\", \"Fido\"]
    print(names[0])
    return names.length as i32
";
    let r = compile_and_run(src);
    assert_eq!(r.stdout, "Rex\n");
    assert_eq!(r.exit_code, 2);
}

#[test]
fn test_print_numeric_and_bool_types() {
    let src = "\
fn main() -> i32:
    print(42)
    print(true)
    print(3.5)
    return 0
";
    let r = compile_and_run(src);
    assert_eq!(r.stdout, "42\ntrue\n3.500000\n");
}

#[test]
fn test_top_level_stray_statement_rejected() {
    let src = "let x = 1\nfn main() -> i32:\n    return 0\n";
    let msg = compile_expect_err(src);
    assert!(msg.contains("최상위 실행문"), "unexpected: {msg}");
}

