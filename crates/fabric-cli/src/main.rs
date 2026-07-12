use fabric_lexer::tokenize;
use fabric_parser::Parser;
use fabric_checker::{check_program, ipet_check_timing, CheckError};
use fabric_codegen::{CodeEmitter, PythonEmitter, CEmitter};

use clap::{Parser as ClapParser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::fs;

fn report_errors(errors: &[CheckError], source: &str, filename: &str) {
    use ariadne::{Report, ReportKind, Label, Source};

    let source_cache = Source::from(source);

    for err in errors {
        let (msg, span) = match err {
            CheckError::MissingFallback { sensor, span } => {
                (format!("sensor '{}' has no fallback handler", sensor), span)
            }
            CheckError::TransitiveMissing { sensor, missing_dep, span } => {
                (format!("fallback for '{}' depends on '{}' which has no fallback", sensor, missing_dep), span)
            }
            CheckError::FallbackCycle { cycle, span } => {
                (format!("fallback cycle detected: {}", cycle.join(" -> ")), span)
            }
            CheckError::DeadlineExceeded { loop_name, deadline_ms, estimated_wcet_ms, span } => {
                (format!("loop '{}' proven worst-case {:.2}ms exceeds deadline {:.2}ms", loop_name, estimated_wcet_ms, deadline_ms), span)
            }
            CheckError::UnknownLoopBound { loop_name, span } => {
                (format!("cannot determine loop bound for '{}'", loop_name), span)
            }
            CheckError::UnboundedLoop { loop_name, span } => {
                (format!("UnboundedLoop: cannot compute WCET for '{}' without a static iteration bound", loop_name), span)
            }
            CheckError::TypeError { message, span } => {
                (format!("type error: {}", message), span)
            }
        };

        let start = span.start;
        let end = span.end;
        Report::build(ReportKind::Error, filename, start)
            .with_label(Label::new((filename, start..end)).with_message(&msg))
            .finish()
            .eprint((filename, source_cache.clone()))
            .ok();
    }
}

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
        /// Show detailed block-by-block IPET breakdown
        #[arg(long)]
        explain: bool,
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
        Commands::Timing { file, clock_mhz, explain } => cmd_timing(file, clock_mhz, explain),
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
        let filename = path.file_name().unwrap().to_string_lossy();
        report_errors(&errors, &source, &filename);
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
        let filename = path.file_name().unwrap().to_string_lossy();
        report_errors(&errors, &source, &filename);
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

fn cmd_timing(path: PathBuf, clock_mhz: f64, explain: bool) {
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
        // Handle errors (e.g., UnboundedLoop)
        if let Some(ref err) = analysis.error {
            eprintln!("Loop: {}", analysis.loop_name);
            eprintln!("  Error: {}", err);
            all_ok = false;
            println!();
            continue;
        }

        let result = analysis.result.as_ref().unwrap();
        let status = if analysis.meets_deadline { "PASS" } else { "FAIL" };
        let status_icon = if analysis.meets_deadline { "✓" } else { "✗" };

        println!("Loop: {}", analysis.loop_name);
        println!("  Status:    {} [{}]", status_icon, status);
        println!("  WCET:      {:.4}ms / {:.4}ms deadline", result.wcet_ms, analysis.deadline_ms);
        println!("  WCET:      {:.1} cycles", result.wcet_cycles);
        println!("  Margin:    {:.1}%", ((analysis.deadline_ms - result.wcet_ms) / analysis.deadline_ms) * 100.0);

        if explain && !result.execution_counts.is_empty() {
            println!("  Block execution counts (binding path highlighted):");
            for (label, count) in &result.execution_counts {
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
