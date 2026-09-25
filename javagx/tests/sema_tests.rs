use javagxc::lexer::Lexer;
use javagxc::parser::Parser;
use javagxc::sema;

fn check(src: &str) -> Result<(), String> {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let program = Parser::new(tokens).parse_program().expect("parse failed");
    sema::check_program(&program).map_err(|e| e.to_string())
}

fn assert_ok(src: &str) {
    if let Err(e) = check(src) {
        panic!("expected OK, got error: {e}\n--- source ---\n{src}");
    }
}

fn assert_err_contains(src: &str, needle: &str) {
    match check(src) {
        Ok(()) => panic!("expected error containing '{needle}', got OK\n--- source ---\n{src}"),
        Err(e) => assert!(e.contains(needle), "error '{e}' does not contain '{needle}'"),
    }
}

// -----------------------------------------------------------------
// SPEC 전체 예제 / 기본 통과 케이스
// -----------------------------------------------------------------

#[test]
fn test_spec_full_example_passes() {
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
    assert_ok(src);
}

#[test]
fn test_basic_var_decl_and_arithmetic() {
    assert_ok("let x: i32 = 10\nlet y = x + 5\n");
}

#[test]
fn test_literal_coercion_to_declared_int_type() {
    assert_ok("let x: i64 = 10\n"); // i32 기본 리터럴이 i64 선언 위치에 승격 허용
}

#[test]
fn test_nullable_assignment() {
    assert_ok("let x: i32? = null\nlet y: i32? = 5\n");
}

// -----------------------------------------------------------------
// 타입 오류
// -----------------------------------------------------------------

#[test]
fn test_type_mismatch_var_decl() {
    assert_err_contains("let x: str = 10\n", "일치하지 않습니다");
}

#[test]
fn test_undefined_variable() {
    assert_err_contains("let x = y + 1\n", "정의되지 않은 변수");
}

#[test]
fn test_null_without_declared_type_is_error() {
    assert_err_contains("let x = null\n", "타입을 추론할 수 없습니다");
}

#[test]
fn test_non_null_cannot_receive_nullable() {
    let src = "let a: i32? = null\nlet b: i32 = a\n";
    assert_err_contains(src, "일치하지 않습니다");
}

#[test]
fn test_binary_type_mismatch() {
    assert_err_contains("let x = \"a\" + 1\n", "'+' 연산자는");
}

#[test]
fn test_if_condition_must_be_bool() {
    assert_err_contains("if 1:\n    let x = 1\n", "bool 타입이어야 합니다");
}

#[test]
fn test_and_or_require_bool_operands() {
    assert_err_contains("let x = 1 and 2\n", "bool 타입이어야 합니다");
}

// -----------------------------------------------------------------
// 클래스 / 상속
// -----------------------------------------------------------------

#[test]
fn test_unknown_class_in_new() {
    assert_err_contains("let x = new Ghost()\n", "정의되지 않은 클래스");
}

#[test]
fn test_unknown_superclass() {
    assert_err_contains("class A(NoSuchClass):\n    fn f(self):\n        return\n", "정의되지 않은 클래스");
}

#[test]
fn test_duplicate_class_name() {
    let src = "class A:\n    fn f(self):\n        return\nclass A:\n    fn g(self):\n        return\n";
    assert_err_contains(src, "중복 선언");
}

#[test]
fn test_inheritance_cycle() {
    let src = "class A(B):\n    fn f(self):\n        return\nclass B(A):\n    fn g(self):\n        return\n";
    assert_err_contains(src, "순환");
}

#[test]
fn test_inherited_field_access() {
    let src = "\
class Animal:
    pub name: str
    fn init(self, name: str):
        self.name = name

class Dog(Animal):
    fn bark(self) -> str:
        return self.name

fn main() -> i32:
    let d = new Dog(\"Rex\")
    return 0
";
    assert_ok(src);
}

#[test]
fn test_private_field_access_outside_class_is_error() {
    let src = "\
class Box:
    priv value: i32
    fn init(self, v: i32):
        self.value = v

fn main() -> i32:
    let b = new Box(1)
    let x = b.value
    return 0
";
    assert_err_contains(src, "private 필드");
}

#[test]
fn test_private_method_access_outside_class_is_error() {
    let src = "\
class Box:
    fn secret(self) -> i32:
        return 1

fn main() -> i32:
    let b = new Box()
    let x = b.secret()
    return 0
";
    assert_err_contains(src, "private 메서드");
}

#[test]
fn test_constructor_arg_count_mismatch() {
    let src = "\
class Box:
    priv value: i32
    fn init(self, v: i32):
        self.value = v

fn main() -> i32:
    let b = new Box()
    return 0
";
    assert_err_contains(src, "인자 개수");
}

#[test]
fn test_no_constructor_with_args_is_error() {
    let src = "class Empty:\n    fn f(self):\n        return\nlet e = new Empty(1)\n";
    assert_err_contains(src, "생성자(init)가 없어");
}

// -----------------------------------------------------------------
// 대입 / 가변성
// -----------------------------------------------------------------

#[test]
fn test_reassign_let_is_error() {
    assert_err_contains("let x = 1\nx = 2\n", "재대입할 수 없습니다");
}

#[test]
fn test_reassign_var_is_ok() {
    assert_ok("var x = 1\nx = 2\n");
}

#[test]
fn test_invalid_assign_target_caught_at_parse() {
    // 1 + 2 = 3 은 파서 단계에서 이미 거부되므로 sema까지 도달하지 않는다.
    let tokens = Lexer::new("1 + 2 = 3\n").tokenize().unwrap();
    assert!(Parser::new(tokens).parse_program().is_err());
}

// -----------------------------------------------------------------
// 제어 흐름 / 반환
// -----------------------------------------------------------------

#[test]
fn test_return_outside_function_is_error() {
    assert_err_contains("return 1\n", "return");
}

#[test]
fn test_break_outside_loop_is_error() {
    assert_err_contains("break\n", "break");
}

#[test]
fn test_break_inside_while_is_ok() {
    assert_ok("var x = 0\nwhile x < 10:\n    x += 1\n    break\n");
}

#[test]
fn test_missing_return_in_non_void_function_is_error() {
    let src = "fn f() -> i32:\n    let x = 1\n";
    assert_err_contains(src, "값을 반환");
}

#[test]
fn test_all_branches_return_satisfies_check() {
    let src = "\
fn f(x: bool) -> i32:
    if x:
        return 1
    else:
        return 2
";
    assert_ok(src);
}

#[test]
fn test_return_type_mismatch() {
    let src = "fn f() -> i32:\n    return \"oops\"\n";
    assert_err_contains(src, "반환 타입이 일치하지 않습니다");
}

// -----------------------------------------------------------------
// 함수 호출
// -----------------------------------------------------------------

#[test]
fn test_call_undefined_function() {
    assert_err_contains("ghost()\n", "정의되지 않은 함수");
}

#[test]
fn test_call_arg_type_mismatch() {
    let src = "fn f(x: i32):\n    return\nf(\"str\")\n";
    assert_err_contains(src, "인자");
}

#[test]
fn test_call_with_correct_args_ok() {
    let src = "fn f(x: i32) -> i32:\n    return x + 1\nlet r = f(10)\n";
    assert_ok(src);
}

// -----------------------------------------------------------------
// 캐스팅
// -----------------------------------------------------------------

#[test]
fn test_numeric_cast_ok() {
    assert_ok("let a: i64 = 100\nlet b: i32 = a as i32\n");
}

#[test]
fn test_cast_non_numeric_is_error() {
    assert_err_contains("let a = \"x\" as i32\n", "숫자 타입 간에만");
}

// -----------------------------------------------------------------
// 미지원 기능 (명시적 에러로 처리됨을 확인)
// -----------------------------------------------------------------

#[test]
fn test_indexing_not_yet_supported() {
    assert_err_contains("let arr = 1\nlet x = arr[0]\n", "인덱싱");
}

// -----------------------------------------------------------------
// 배열 (6단계)
// -----------------------------------------------------------------

#[test]
fn test_array_literal_and_index_ok() {
    assert_ok("let a = [1, 2, 3]\nlet x = a[0]\n");
}

#[test]
fn test_array_length_is_i64() {
    assert_ok("let a = [1, 2, 3]\nlet len: i64 = a.length\n");
}

#[test]
fn test_empty_array_needs_declared_type() {
    assert_err_contains("let a = []\n", "타입을 추론할 수 없습니다");
}

#[test]
fn test_empty_array_with_type_ok() {
    assert_ok("let a: i32[] = []\n");
}

#[test]
fn test_array_element_type_mismatch() {
    assert_err_contains("let a = [1, \"x\"]\n", "배열 원소");
}

#[test]
fn test_index_non_array_is_error() {
    assert_err_contains("let x = 1\nlet y = x[0]\n", "인덱싱할 수 없습니다");
}

#[test]
fn test_index_with_non_integer_is_error() {
    assert_err_contains("let a = [1, 2]\nlet x = a[\"0\"]\n", "정수 타입");
}

#[test]
fn test_for_over_array_binds_element_type() {
    let src = "fn f():\n    var total = 0\n    for x in [1, 2, 3]:\n        total += x\n    return\n";
    assert_ok(src);
}

#[test]
fn test_for_over_non_array_is_error() {
    assert_err_contains("let y = 1\nfor x in y:\n    x\n", "배열이어야 합니다");
}

#[test]
fn test_array_index_assignment_ok() {
    assert_ok("let a = [1, 2, 3]\na[0] = 99\n");
}

#[test]
fn test_nested_function_decl_is_error() {
    let src = "fn outer():\n    fn inner():\n        return\n    return\n";
    assert_err_contains(src, "중첩된");
}
