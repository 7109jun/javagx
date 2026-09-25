# JavaGX Language Specification v0.1

## 1. 렉시컬 구조

### 1.1 주석
```
# 한 줄 주석만 지원 (v0.1)
```

### 1.2 식별자
```
identifier = (letter | "_") , { letter | digit | "_" } ;
letter     = "a".."z" | "A".."Z" ;
digit      = "0".."9" ;
```

### 1.3 키워드 (예약어)
```
class   self    new
let     var     const
fn      return
if      elif    else
while   for     in      break   continue
true    false   null
import  as
pub     priv
and     or      not
i8 i16 i32 i64  u8 u16 u32 u64
f32 f64  bool  str  char  void
```

### 1.4 리터럴
```
int_literal    = digit , { digit } ;
float_literal  = digit , { digit } , "." , digit , { digit } ;
string_literal = '"' , { any_char_except('"') } , '"' ;
char_literal   = "'" , any_char , "'" ;
bool_literal   = "true" | "false" ;
```

### 1.5 들여쓰기 규칙 (Off-side rule)
- 공백(스페이스) 4칸을 표준 들여쓰기 단위로 강제. 탭 사용 시 컴파일 에러.
- 블록 시작은 `:` 로 표시, 다음 줄부터 들여쓰기 증가 → `INDENT` 토큰 발행
- 들여쓰기가 줄어들면 그 차이만큼 `DEDENT` 토큰 발행
- 같은 줄에 여러 문장은 `;` 로 구분 가능 (선택적)

---

## 2. 연산자 우선순위 (낮음 → 높음)

| 우선순위 | 연산자 | 결합성 |
|---|---|---|
| 1 | `or` | 좌결합 |
| 2 | `and` | 좌결합 |
| 3 | `not` (단항) | 우결합 |
| 4 | `==` `!=` | 좌결합 |
| 5 | `<` `<=` `>` `>=` | 좌결합 |
| 6 | `+` `-` | 좌결합 |
| 7 | `*` `/` `%` | 좌결합 |
| 8 | 단항 `-` `!` | 우결합 |
| 9 | `.` (멤버 접근) `()` (호출) `[]` (인덱싱) | 좌결합 |

대입 연산자(`=` `+=` `-=` `*=` `/=`)는 표현식이 아닌 **문장(statement)** 으로 취급 (파이썬과 동일, 체이닝 금지: `a = b = 1` 불가).

---

## 3. 타입 시스템

### 3.1 원시 타입
```
i8 i16 i32 i64   부호 있는 정수
u8 u16 u32 u64   부호 없는 정수
f32 f64          부동소수점
bool             참/거짓
str              UTF-8 문자열 (불변)
char             단일 UTF-8 코드포인트
void             함수 반환값 없음
```

### 3.1.1 배열 타입
`T[]`는 원소 타입이 `T`인 배열이다 (`i32[]`, `str[]`, `Dog[]` 등). 원소는 nullable일 수 없다.
```
let nums: i32[] = [1, 2, 3]
let empty: i32[] = []          # 빈 배열은 원소 타입을 알 수 없으므로 타입 주석 필수 (null과 동일)
let n = nums[0]                 # 인덱싱
let len = nums.length           # i64
for x in nums:
    print(x)
```

### 3.2 타입 추론과 명시
```
let x: i32 = 10      # 명시적 타입
let y = 10            # 추론 → i32 (정수 리터럴 기본값)
let z = 3.14          # 추론 → f64
const PI: f64 = 3.14159
```
- `let` : 재대입 불가 (immutable binding)
- `var` : 재대입 가능 (mutable binding)
- `const` : 컴파일타임 상수

### 3.3 Nullable / Optional
```
let x: i32? = null     # ? 접미사로 nullable 타입 선언
```
- Non-null 타입에 null 대입 시 컴파일 에러
- null 가능 타입 역참조 전 `if x != null:` 스코프에서만 강제 언랩 허용 (v0.2에서 flow typing 확정)

### 3.3.1 `+` 연산자
`+`는 숫자 타입 간 산술 덧셈뿐 아니라 `str` 타입 간 문자열 연결(concatenation)에도
사용된다 (예: `self.name + " barks"`). 그 외 타입 조합에는 사용할 수 없다.

### 3.4 타입 캐스팅
```
let a: i64 = 100
let b: i32 = a as i32   # 명시적 캐스팅만 허용, 암시적 축소 변환 금지
```

---

## 4. 문법 EBNF (핵심 구문)

```ebnf
program        = { statement } ;

statement      = class_decl
               | func_decl
               | var_decl
               | if_stmt
               | while_stmt
               | for_stmt
               | return_stmt
               | break_stmt
               | continue_stmt
               | expr_stmt ;

class_decl     = "class" , identifier , [ "(" , identifier , ")" ] , ":" , NEWLINE ,
                 INDENT , { field_decl | method_decl } , DEDENT ;

field_decl     = [ "pub" | "priv" ] , identifier , ":" , type , NEWLINE ;

method_decl    = "fn" , identifier , "(" , [ "self" , { "," , param } ] , ")" ,
                 [ "->" , type ] , ":" , block ;

func_decl      = "fn" , identifier , "(" , [ param_list ] , ")" ,
                 [ "->" , type ] , ":" , block ;

param_list     = param , { "," , param } ;
param          = identifier , ":" , type ;

var_decl       = ("let" | "var" | "const") , identifier , [ ":" , type ] ,
                 "=" , expr , NEWLINE ;

type           = base_type , { "[" "]" } , [ "?" ] ;
base_type      = "i8"|"i16"|"i32"|"i64"|"u8"|"u16"|"u32"|"u64"
                  |"f32"|"f64"|"bool"|"str"|"char"|"void"|identifier ;

if_stmt        = "if" , expr , ":" , block ,
                 { "elif" , expr , ":" , block } ,
                 [ "else" , ":" , block ] ;

while_stmt     = "while" , expr , ":" , block ;
for_stmt       = "for" , identifier , "in" , expr , ":" , block ;

return_stmt    = "return" , [ expr ] , NEWLINE ;
break_stmt     = "break" , NEWLINE ;
continue_stmt  = "continue" , NEWLINE ;

expr_stmt      = expr , NEWLINE ;

block          = NEWLINE , INDENT , { statement } , DEDENT ;

expr           = assign_expr ;
assign_expr    = logic_or , [ ("="|"+="|"-="|"*="|"/=") , assign_expr ] ;
logic_or       = logic_and , { "or" , logic_and } ;
logic_and      = equality , { "and" , equality } ;
equality       = comparison , { ("=="|"!=") , comparison } ;
comparison     = additive , { ("<"|"<="|">"|">=") , additive } ;
additive       = multiplicative , { ("+"|"-") , multiplicative } ;
multiplicative = unary , { ("*"|"/"|"%") , unary } ;
unary          = [ "not" | "-" | "!" ] , postfix ;
postfix        = primary , { "." , identifier
                            | "(" , [ arg_list ] , ")"
                            | "[" , expr , "]" } ;
primary        = int_literal | float_literal | string_literal | char_literal
               | bool_literal | "null" | "self" | identifier
               | "new" , identifier , "(" , [ arg_list ] , ")"
               | "[" , [ arg_list ] , "]"          (* 배열 리터럴 *)
               | "(" , expr , ")" ;

arg_list       = expr , { "," , expr } ;
```

---

## 5. 클래스 / OOP 규칙

```javagx
class Animal:
    pub name: str
    priv age: i32

    fn init(self, name: str, age: i32):
        self.name = name
        self.age = age

    pub fn speak(self) -> str:
        return self.name + " makes a sound"

class Dog(Animal):
    fn speak(self) -> str:
        return self.name + " barks"

fn main() -> i32:
    let d = new Dog("Rex", 3)
    print(d.speak())
    return 0
```

- `init` : 생성자 메서드 (문법 키워드가 아닌 관례적 이름 — 렉서는 일반 식별자로 처리하고 Sema 단계에서 `init`이라는 이름의 메서드를 생성자로 인식, `new ClassName(...)` 으로 호출)
- `self` : 첫 인자로 명시 필수 (파이썬 방식, 암시적 바인딩 없음)
- 상속: `class Sub(Base):` 단일 상속만 v0.1에서 지원 (다중 상속 제외)
- 접근 제어: `pub`(기본값 아님, 명시 안 하면 `priv`)
- 가상 메서드/오버라이드: 동일 시그니처로 재정의 시 자동 오버라이드 (별도 키워드 없음, v0.2에서 `override` 키워드 강제 여부 재검토)
- 엔트리포인트: `fn main() -> i32:` 필수

---

## 6. 열린 설계 이슈 (v0.2 이후 확정 예정)
- 제네릭 (`class Box<T>`) 지원 여부
- 인터페이스/트레이트 문법
- 예외 처리 (`try/except` vs Result 타입)
- 메모리 모델: **참조 카운팅(ARC)으로 확정** (v0.1, 3단계 진행 시 결정)
- 배열 타입은 `T[]`(고정 원소 타입, 1차원 구조는 `T[][]` 형태로 중첩 가능)로 확정 (6단계).
  제네릭 컬렉션(`List<T>` 등)은 여전히 미정 — 제네릭 자체가 열린 이슈이기 때문.
  배열 메서드는 `.length` 하나뿐 (push/pop/slice 등은 후속 버전).
- `Expr::Ident`/`Expr::SelfExpr` 등 일부 AST 리프 노드에 위치(line) 정보가 없어,
  해당 노드에서 직접 발생하는 일부 Sema 에러(예: 정의되지 않은 변수)가 정확한 줄 번호
  대신 0을 보고할 수 있음 — 이런 경우는 보통 상위 노드(Binary/Call/Assign 등)의
  에러로 함께 감지되어 실질적 영향은 제한적이나, 추후 AST에 위치 정보를 전면 추가하는
  리팩터링이 필요
- `print`는 전용 stdlib 모듈 이전까지 Sema/Codegen에 하드코딩된 임시 빌트인(인자 1개,
  반환 void). 6단계에서 모든 원시 타입(정수/실수/bool/char/str)까지 지원 범위를 넓혔으나
  class/array 값 출력은 아직 미지원(사용자 정의 `toString`/원소별 순회 출력 설계 필요).
- **Codegen 범위 한계** — 아래는 Sema는 통과하지만 Codegen이 명시적 에러로 거부한다:
  - nullable 원시 타입(`i32?` 등)의 `null` 대입: null 상태를 표현할 태그된 표현이 아직 없음
    (nullable 클래스/배열 타입은 포인터의 null로 자연스럽게 표현되어 지원됨)
  - 최상위(top-level)의 `class`/`fn` 선언 외 실행문: `main()`을 유일한 진입점으로 삼기 위한 의도적 제한
  - 가상 디스패치(vtable) 없음 — 서브클래스를 상위클래스 타입 변수에 대입(업캐스팅)하는 기능 자체가
    Sema에 없으므로 v0.1에서는 모든 메서드 호출이 정적으로 정확한 클래스에 바인딩됨
  - ARC(참조 카운팅) retain/release 삽입 미구현 — `new`/배열 리터럴은 `malloc`으로 힙에 할당만
    하고 해제하지 않음 (메모리 누수). 전용 ARC 삽입 패스는 이후 단계에서 별도로 추가 예정
  - 리터럴 정수가 더 작은 정수 타입으로 대입/캐스팅될 때 범위 검사 없음 (조용히 truncate)
