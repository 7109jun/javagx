use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{exit, Command};

use inkwell::context::Context;

use javagxc::codegen;
use javagxc::lexer::Lexer;
use javagxc::parser::Parser;
use javagxc::sema;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("사용법: javagxc <file.jagx> [-o <output>]");
        exit(1);
    }

    let path = &args[1];
    let mut output = PathBuf::from(path);
    output.set_extension("");
    let mut output_path = output.to_string_lossy().to_string();
    let mut i = 2;
    while i < args.len() {
        if args[i] == "-o" && i + 1 < args.len() {
            output_path = args[i + 1].clone();
            i += 2;
        } else {
            i += 1;
        }
    }

    let source = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("파일을 읽을 수 없습니다 '{}': {}", path, e);
        exit(1);
    });

    let tokens = Lexer::new(&source).tokenize().unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    });

    let program = Parser::new(tokens).parse_program().unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    });

    let (classes, funcs) = sema::analyze(&program).unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    });

    let context = Context::create();
    let module_name = PathBuf::from(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "javagx_module".to_string());

    let cg = codegen::compile(&context, &module_name, &program, &classes, &funcs).unwrap_or_else(|e| {
        eprintln!("{}", e);
        exit(1);
    });

    let obj_path = format!("{}.o", output_path);
    if let Err(e) = cg.emit_object(std::path::Path::new(&obj_path)) {
        eprintln!("{}", e);
        exit(1);
    }

    let status = Command::new("cc")
        .arg(&obj_path)
        .arg("-o")
        .arg(&output_path)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("링커(cc) 실행 실패: {}", e);
            exit(1);
        });

    let _ = fs::remove_file(&obj_path);

    if !status.success() {
        eprintln!("링킹에 실패했습니다 (종료 코드: {:?})", status.code());
        exit(1);
    }

    println!("네이티브 실행 파일을 생성했습니다: {}", output_path);
}
