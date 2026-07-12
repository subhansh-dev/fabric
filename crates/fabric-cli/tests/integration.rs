use fabric_lexer::tokenize;
use fabric_parser::Parser;
use fabric_checker::{check_program, CheckError};
use fabric_codegen::{CodeEmitter, PythonEmitter, CEmitter};
use fabric_types::TypeEnv;

fn parse_source(source: &str) -> fabric_ast::Program {
    let tokens = tokenize(source).unwrap();
    let mut parser = Parser::new(tokens);
    parser.parse_program().unwrap_or_else(|errors| {
        for e in &errors {
            eprintln!("Parser error: {}", e.message);
        }
        panic!("parse failed");
    })
}

#[test]
fn test_sensor_declaration() {
    let prog = parse_source("sensor imu: Sensor<f32, ±0.1>");
    assert_eq!(prog.declarations.len(), 1);
    match &prog.declarations[0] {
        fabric_ast::Declaration::Sensor(s) => {
            assert_eq!(s.name.name, "imu");
        }
        _ => panic!("expected Sensor declaration"),
    }
}

#[test]
fn test_actuator_declaration() {
    let prog = parse_source("actuator motors: Motor[4]");
    assert_eq!(prog.declarations.len(), 1);
    match &prog.declarations[0] {
        fabric_ast::Declaration::Actuator(a) => {
            assert_eq!(a.name.name, "motors");
        }
        _ => panic!("expected Actuator declaration"),
    }
}

#[test]
fn test_fallback_declaration() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
when altitude unavailable for 200ms {
    fallback to 0.0
}"#;
    let prog = parse_source(source);
    assert_eq!(prog.declarations.len(), 2);
    match &prog.declarations[1] {
        fabric_ast::Declaration::Fallback(f) => {
            assert_eq!(f.sensor_name.name, "altitude");
            assert_eq!(f.timeout.value, 200.0);
        }
        _ => panic!("expected Fallback declaration"),
    }
}

#[test]
fn test_loop_declaration() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
loop control within 2ms {
    let x: f32 = 1.0
}"#;
    let prog = parse_source(source);
    assert_eq!(prog.declarations.len(), 4);
    match &prog.declarations[3] {
        fabric_ast::Declaration::Loop(l) => {
            assert_eq!(l.name.name, "control");
            assert_eq!(l.deadline.value, 2.0);
        }
        _ => panic!("expected Loop declaration"),
    }
}

#[test]
fn test_function_declaration() {
    let source = r#"
fn dead_reckoning(imu_val: f32) -> f32 {
    let result: f32 = imu_val * 0.95
    return result
}"#;
    let prog = parse_source(source);
    assert_eq!(prog.declarations.len(), 1);
    match &prog.declarations[0] {
        fabric_ast::Declaration::Function(f) => {
            assert_eq!(f.name.name, "dead_reckoning");
            assert_eq!(f.params.len(), 1);
        }
        _ => panic!("expected Function declaration"),
    }
}

#[test]
fn test_variable_declaration() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
let target_altitude: f32 = 10.0
"#;
    let prog = parse_source(source);
    assert_eq!(prog.declarations.len(), 4);
    match &prog.declarations[3] {
        fabric_ast::Declaration::Variable(v) => {
            assert_eq!(v.name.name, "target_altitude");
        }
        _ => panic!("expected Variable declaration"),
    }
}

#[test]
fn test_missing_fallback_detected() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
loop control within 2ms {
    let x: f32 = 1.0
}"#;
    let prog = parse_source(source);
    let errors = check_program(&prog, 72.0);
    assert!(errors.iter().any(|e| matches!(e, CheckError::MissingFallback { .. })));
}

#[test]
fn test_fallback_provided_no_error() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
loop control within 2ms {
    let x: f32 = 1.0
}"#;
    let prog = parse_source(source);
    let errors = check_program(&prog, 72.0);
    assert!(!errors.iter().any(|e| matches!(e, CheckError::MissingFallback { .. })));
}

#[test]
fn test_python_codegen_smoke() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
loop control within 2ms {
    let raw: f32 = read altitude
    write motors[0] = raw
}"#;
    let prog = parse_source(source);
    let code = PythonEmitter.emit_program(&prog);
    assert!(code.contains("class FabricController(Robot)"));
    assert!(code.contains("fallback_altitude"));
    assert!(code.contains("self.motors[0].setVelocity"));
}

#[test]
fn test_c_codegen_smoke() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
loop control within 2ms {
    let raw: f32 = read altitude
    write motors[0] = raw
}"#;
    let prog = parse_source(source);
    let code = CEmitter.emit_program(&prog);
    assert!(code.contains("int main(void)"));
    assert!(code.contains("fallback_altitude"));
    assert!(code.contains("hal_get_time_ms"));
}

#[test]
fn test_binary_expression_precedence() {
    let source = r#"
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when altitude unavailable for 200ms {
    fallback to 0.0
}
loop control within 2ms {
    let a: f32 = 1.0
    let b: f32 = 2.0
    let c: f32 = 3.0
    let result: f32 = a + b * c
}"#;
    let prog = parse_source(source);
    // Should parse without error - multiplication binds tighter than addition
    // 1 sensor + 1 actuator + 1 fallback + 1 loop = 4 declarations
    assert_eq!(prog.declarations.len(), 4);
}

#[test]
fn test_multiple_sensors_and_fallbacks() {
    let source = r#"
sensor imu: Sensor<f32, ±0.1>
sensor altitude: Sensor<f32, ±0.5>
sensor gps: Sensor<f32, ±1.0>
actuator motors: Motor[4]
when imu unavailable for 100ms {
    fallback to 0.0
}
when altitude unavailable for 200ms {
    fallback to 0.0
}
when gps unavailable for 500ms {
    fallback to 0.0
}
loop control within 2ms {
    let x: f32 = 1.0
}"#;
    let prog = parse_source(source);
    let errors = check_program(&prog, 72.0);
    // All sensors have fallbacks, no missing fallback errors
    assert!(!errors.iter().any(|e| matches!(e, CheckError::MissingFallback { .. })));
}

#[test]
fn test_merge_expression() {
    let source = r#"
sensor imu: Sensor<f32, ±0.1>
sensor altitude: Sensor<f32, ±0.5>
actuator motors: Motor[4]
when imu unavailable for 100ms { fallback to 0.0 }
when altitude unavailable for 200ms { fallback to 0.0 }
loop control within 2ms {
    let fused: f32 = merge imu altitude [0.6, 0.4]
    write motors[0] = fused
}"#;
    let prog = parse_source(source);
    // Parse succeeds
    assert_eq!(prog.declarations.len(), 6);
    // Type check passes
    let errors = check_program(&prog, 72.0);
    assert!(errors.is_empty(), "Type check failed: {:?}", errors);
    // Python codegen produces weighted average
    let py = PythonEmitter.emit_program(&prog);
    assert!(py.contains("imu * 0.6"), "Python merge broken: {}", py);
    assert!(py.contains("altitude * 0.4"), "Python merge broken: {}", py);
    // C codegen produces weighted average
    let c = CEmitter.emit_program(&prog);
    assert!(c.contains("imu * 0.6"), "C merge broken: {}", c);
    assert!(c.contains("altitude * 0.4"), "C merge broken: {}", c);
}

#[test]
fn test_match_expression() {
    let source = r#"
sensor imu: Sensor<f32, ±0.1>
actuator motors: Motor[4]
when imu unavailable for 100ms { fallback to 0.0 }
loop control within 2ms {
    let val: f32 = match imu {
        ok => read imu,
        timeout => 0.0,
        error => -1.0
    }
    write motors[0] = val
}"#;
    let prog = parse_source(source);
    assert_eq!(prog.declarations.len(), 4);
    let errors = check_program(&prog, 72.0);
    assert!(errors.is_empty(), "Type check failed: {:?}", errors);
    // Python codegen
    let py = PythonEmitter.emit_program(&prog);
    assert!(py.contains("is not None"), "Python match broken: {}", py);
    assert!(py.contains("timeout_imu"), "Python match broken: {}", py);
    // C codegen
    let c = CEmitter.emit_program(&prog);
    assert!(c.contains("isnan"), "C match broken: {}", c);
    assert!(c.contains("timeout_imu"), "C match broken: {}", c);
}

#[test]
fn test_function_arity_check() {
    let source = r#"
fn add(a: f32, b: f32) -> f32 {
    return a + b
}
sensor imu: Sensor<f32, ±0.1>
actuator motors: Motor[1]
when imu unavailable for 100ms { fallback to 0.0 }
loop control within 2ms {
    let x: f32 = add(1.0)
    write motors[0] = x
}"#;
    let prog = parse_source(source);
    let mut env = TypeEnv::new();
    let result = env.check_program(&prog);
    assert!(result.is_err(), "Expected arity error");
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| matches!(e, fabric_types::TypeError::WrongArity { .. })),
        "Expected WrongArity, got: {:?}", errors);
}

#[test]
fn test_function_return_type() {
    let source = r#"
fn double(x: f32) -> f32 {
    return x * 2.0
}
sensor imu: Sensor<f32, ±0.1>
actuator motors: Motor[1]
when imu unavailable for 100ms { fallback to 0.0 }
loop control within 2ms {
    let val: f32 = double(5.0)
    write motors[0] = val
}"#;
    let prog = parse_source(source);
    let mut env = TypeEnv::new();
    let result = env.check_program(&prog);
    assert!(result.is_ok(), "Expected no errors, got: {:?}", result.err());
}

#[test]
fn test_function_call_codegen() {
    let source = r#"
fn scale(x: f32) -> f32 {
    return x * 0.5
}
sensor imu: Sensor<f32, ±0.1>
actuator motors: Motor[1]
when imu unavailable for 100ms { fallback to 0.0 }
loop control within 2ms {
    let val: f32 = scale(10.0)
    write motors[0] = val
}"#;
    let prog = parse_source(source);
    let py = PythonEmitter.emit_program(&prog);
    assert!(py.contains("def scale(x):"), "Python fn missing: {}", py);
    assert!(py.contains("scale(10.00)"), "Python call missing: {}", py);
    let c = CEmitter.emit_program(&prog);
    assert!(c.contains("float scale(float x)"), "C fn missing: {}", c);
    assert!(c.contains("scale(10.00)"), "C call missing: {}", c);
}

#[test]
fn test_probe_expression() {
    let source = r#"
sensor imu: Sensor<f32, ±0.1>
actuator motors: Motor[2]
when imu unavailable for 100ms { fallback to 0.0 }
loop control within 2ms {
    let status: bool = probe imu
    if status {
        let val: f32 = read imu
        write motors[0] = val
    } else {
        write motors[0] = 0.0
    }
}"#;
    let prog = parse_source(source);
    let errors = check_program(&prog, 72.0);
    assert!(errors.is_empty(), "Check failed: {:?}", errors);
    let py = PythonEmitter.emit_program(&prog);
    assert!(py.contains("imu_raw is not None"), "Python probe missing: {}", py);
    let c = CEmitter.emit_program(&prog);
    assert!(c.contains("(!isnan(imu_raw))"), "C probe missing: {}", c);
}
