use fabric_lexer::tokenize;
use fabric_parser::Parser;
use fabric_checker::{check_program, ipet_check_timing};
use fabric_codegen::{CodeEmitter, PythonEmitter, CEmitter};

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::fs;

#[derive(ClapParser)]
#[command(name = "fabric")]
#[command(about = "DSL for real-time robotics control — compiles safety guarantees")]
#[command(version = "0.2.0 — IPET timing")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Type-check and validate a Fabric program
    Check {
        /// Path to .fab source file
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Generate Python (Webots) or C (ARM) code
    Build {
        /// Path to .fab source file
        #[arg(short, long)]
        file: PathBuf,
        /// Target language
        #[arg(short, long, value_enum, default_value_t = Target::Python)]
        target: Target,
        /// Output file (defaults to stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Print the AST in debug format
    Ast {
        /// Path to .fab source file
        #[arg(short, long)]
        file: PathBuf,
    },
    /// Analyze timing/WCET using IPET (proven bounds)
    Timing {
        /// Path to .fab source file
        #[arg(short, long)]
        file: PathBuf,
        /// Clock speed in MHz (default: 72 for STM32F4)
        #[arg(short, long, default_value_t = 72.0)]
        clock_mhz: f64,
    },
}

#[derive(Clone, ValueEnum)]
enum Target {
    Python,
    C,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Check { file } => cmd_check(file),
        Commands::Build { file, target, output } => cmd_build(file, target, output),
        Commands::Ast { file } => cmd_ast(file),
        Commands::Timing { file, clock_mhz } => cmd_timing(file, clock_mhz),
    }
}

fn load_source(path: &PathBuf) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", path.display(), e);
        std::process::exit(1);
    })
}

fn cmd_check(path: PathBuf) {
    let source = load_source(&path);

    // Phase 1: Lex
    let tokens = match tokenize(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lexer error: {}", e);
            std::process::exit(1);
        }
    };

    // Phase 2: Parse
    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for e in &errors {
                eprintln!("Parser error: {}", e.message);
            }
            std::process::exit(1);
        }
    };

    // Phase 3: Check
    let errors = check_program(&program, 72.0); // 72 MHz default clock
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("Check error: {}", e);
        }
        std::process::exit(1);
    }

    println!("ok — program passed all checks");
}

fn cmd_build(path: PathBuf, target: Target, output: Option<PathBuf>) {
    let source = load_source(&path);

    // Lex
    let tokens = match tokenize(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lexer error: {}", e);
            std::process::exit(1);
        }
    };

    // Parse
    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for e in &errors {
                eprintln!("Parser error: {}", e.message);
            }
            std::process::exit(1);
        }
    };

    // Check
    let errors = check_program(&program, 72.0);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("Check error: {}", e);
        }
        std::process::exit(1);
    }

    // Codegen
    let code = match target {
        Target::Python => PythonEmitter.emit_program(&program),
        Target::C => CEmitter.emit_program(&program),
    };

    match output {
        Some(path) => {
            fs::write(&path, &code).unwrap_or_else(|e| {
                eprintln!("error: cannot write {}: {}", path.display(), e);
                std::process::exit(1);
            });
            println!("generated {} -> {}", path.display(), match target {
                Target::Python => "python",
                Target::C => "c",
            });
        }
        None => print!("{}", code),
    }
}

fn cmd_ast(path: PathBuf) {
    let source = load_source(&path);

    let tokens = match tokenize(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lexer error: {}", e);
            std::process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for e in &errors {
                eprintln!("Parser error: {}", e.message);
            }
            std::process::exit(1);
        }
    };

    println!("{:#?}", program);
}

fn cmd_timing(path: PathBuf, clock_mhz: f64) {
    let source = load_source(&path);

    let tokens = match tokenize(&source) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Lexer error: {}", e);
            std::process::exit(1);
        }
    };

    let mut parser = Parser::new(tokens);
    let program = match parser.parse_program() {
        Ok(p) => p,
        Err(errors) => {
            for e in &errors {
                eprintln!("Parser error: {}", e.message);
            }
            std::process::exit(1);
        }
    };

    // Run IPET analysis
    let results = ipet_check_timing(&program, clock_mhz);

    if results.is_empty() {
        println!("No loops found — nothing to analyze");
        return;
    }

    println!("═══ IPET Timing Analysis (clock: {} MHz) ═══\n", clock_mhz);

    let mut all_ok = true;
    for analysis in &results {
        let status = if analysis.meets_deadline { "PASS" } else { "FAIL" };
        let status_icon = if analysis.meets_deadline { "✓" } else { "✗" };

        println!("Loop: {}", analysis.loop_name);
        println!("  Status:    {} [{}]", status_icon, status);
        println!("  WCET:      {:.4}ms / {:.4}ms deadline", analysis.result.wcet_ms, analysis.deadline_ms);
        println!("  WCET:      {:.1} cycles", analysis.result.wcet_cycles);
        println!("  Margin:    {:.1}%", ((analysis.deadline_ms - analysis.result.wcet_ms) / analysis.deadline_ms) * 100.0);

        if !analysis.result.execution_counts.is_empty() {
            println!("  Block execution counts:");
            for (label, count) in &analysis.result.execution_counts {
                if *count > 0.0 {
                    println!("    {}: {:.1}x", label, count);
                }
            }
        }
        println!();

        if !analysis.meets_deadline {
            all_ok = false;
        }
    }

    if all_ok {
        println!("Result: All loops meet their deadlines");
    } else {
        eprintln!("Result: Some loops EXCEED their deadlines");
        std::process::exit(1);
    }
}
