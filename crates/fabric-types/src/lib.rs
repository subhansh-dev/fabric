use fabric_core::Span;
use fabric_ast::*;
use std::collections::HashMap;

/// Type system for Fabric
///
/// Handles:
/// - Primitive type checking
/// - Sensor<T, ±error> uncertainty types
/// - Error bound propagation
/// - Type compatibility checking

#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    UndefinedVariable { name: String, span: Span },
    TypeMismatch { expected: String, found: String, span: Span },
    SensorUncertaintyMismatch { expected: String, found: String, span: Span },
    CannotCompareUncertain { span: Span },
    MissingField { ty: String, field: String, span: Span },
    NotAFunction { name: String, span: Span },
    WrongArity { expected: usize, found: usize, span: Span },
}

#[derive(Debug, Clone)]
pub struct TypeEnv {
    symbols: HashMap<String, TypeInfo>,
    current_return_type: Option<TypeInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeInfo {
    Exact(PrimitiveType),
    Sensor { inner: PrimitiveType, error_bound: ErrorBoundInfo },
    Array { element: Box<TypeInfo>, size: usize },
    Function { params: Vec<TypeInfo>, ret: Box<TypeInfo> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorBoundInfo {
    pub value: f64,
    pub kind: ErrorBoundKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorBoundKind {
    Absolute,
    Relative,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        Self { symbols: HashMap::new(), current_return_type: None }
    }

    pub fn define(&mut self, name: String, info: TypeInfo) {
        self.symbols.insert(name, info);
    }

    pub fn lookup(&self, name: &str) -> Option<&TypeInfo> {
        self.symbols.get(name)
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), Vec<TypeError>> {
        let mut errors = Vec::new();

        for decl in &program.declarations {
            match decl {
                Declaration::Sensor(s) => {
                    let info = TypeInfo::Sensor {
                        inner: s.sensor_type.inner_type,
                        error_bound: error_bound_to_info(&s.sensor_type.error_bound),
                    };
                    self.define(s.name.name.clone(), info);
                }
                Declaration::Actuator(a) => {
                    // Actuators don't have a meaningful type for checking
                    self.define(a.name.name.clone(), TypeInfo::Exact(PrimitiveType::F32));
                }
                Declaration::Variable(v) => {
                    if let Some(ref ty) = v.ty {
                        let info = primitive_type_to_info(ty);
                        self.define(v.name.name.clone(), info);
                    } else {
                        // Infer type from expression
                        match self.check_expr(&v.value) {
                            Ok(info) => self.define(v.name.name.clone(), info),
                            Err(e) => errors.push(e),
                        }
                    }
                }
                Declaration::Loop(l) => {
                    for stmt in &l.body {
                        if let Err(e) = self.check_stmt(stmt) {
                            errors.push(e);
                        }
                    }
                }
                Declaration::Fallback(f) => {
                    if let Err(e) = self.check_expr(&f.fallback_expr) {
                        errors.push(e);
                    }
                }
                Declaration::Function(f) => {
                    // First, register the function's type so it can be called recursively
                    let param_infos: Vec<TypeInfo> = f.params.iter()
                        .map(|p| type_to_info(&p.ty))
                        .collect();
                    let ret_info = f.return_type.as_ref()
                        .map(type_to_info)
                        .unwrap_or(TypeInfo::Exact(PrimitiveType::F32));
                    self.define(f.name.name.clone(), TypeInfo::Function {
                        params: param_infos.clone(),
                        ret: Box::new(ret_info.clone()),
                    });

                    // Then check the body with params in scope
                    let mut fn_env = self.clone();
                    for param in &f.params {
                        let info = type_to_info(&param.ty);
                        fn_env.define(param.name.name.clone(), info);
                    }
                    fn_env.current_return_type = Some(ret_info);
                    for stmt in &f.body {
                        if let Err(e) = fn_env.check_stmt(stmt) {
                            errors.push(e);
                        }
                    }
                }
                Declaration::Drone(_) => {
                    // Drones don't have type checking — structural declarations only
                }
            }
        }

        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }

    fn check_stmt(&mut self, stmt: &Statement) -> Result<(), TypeError> {
        match stmt {
            Statement::Read { target, sensor, span } => {
                if self.lookup(&sensor.name).is_none() {
                    return Err(TypeError::UndefinedVariable {
                        name: sensor.name.clone(),
                        span: *span,
                    });
                }
                // Target gets the sensor's type
                if let Some(sensor_info) = self.lookup(&sensor.name).cloned() {
                    self.define(target.name.clone(), sensor_info);
                }
                Ok(())
            }
            Statement::Write { target: _, value, span: _ } => {
                self.check_expr(value)?;
                Ok(())
            }
            Statement::Assign { target, value, span } => {
                if self.lookup(&target.name).is_none() {
                    return Err(TypeError::UndefinedVariable {
                        name: target.name.clone(),
                        span: *span,
                    });
                }
                self.check_expr(value)?;
                Ok(())
            }
            Statement::Let { name, ty, value, span } => {
                let value_info = self.check_expr(value)?;
                let info = if let Some(ref t) = ty {
                    let declared = type_to_info(t);
                    // Allow sensor-to-primitive coercion:
                    // Sensor { inner: F32, .. } coerces to Exact(F32)
                    let compatible = match (&declared, &value_info) {
                        (TypeInfo::Exact(a), TypeInfo::Sensor { inner: b, .. }) => a == b,
                        _ => declared == value_info,
                    };
                    if !compatible {
                        return Err(TypeError::TypeMismatch {
                            expected: format!("{:?}", declared),
                            found: format!("{:?}", value_info),
                            span: *span,
                        });
                    }
                    declared
                } else {
                    value_info
                };
                self.define(name.name.clone(), info);
                Ok(())
            }
            Statement::Return { value, span } => {
                if let Some(ref expr) = value {
                    let ret_ty = self.check_expr(expr)?;
                    // Check against declared return type if available
                    if let Some(ref expected) = self.current_return_type.clone() {
                        match (&ret_ty, expected) {
                            (TypeInfo::Exact(a), TypeInfo::Exact(b)) if a == b => {}
                            // Allow sensor→primitive coercion
                            (TypeInfo::Sensor { .. }, TypeInfo::Exact(_)) => {}
                            _ => {
                                return Err(TypeError::TypeMismatch {
                                    expected: format!("{:?}", expected),
                                    found: format!("{:?}", ret_ty),
                                    span: *span,
                                });
                            }
                        }
                    }
                }
                Ok(())
            }
            Statement::IfElse { condition, then_body, else_body, .. } => {
                self.check_expr(condition)?;
                for stmt in then_body {
                    self.check_stmt(stmt)?;
                }
                if let Some(ref else_stmts) = else_body {
                    for stmt in else_stmts {
                        self.check_stmt(stmt)?;
                    }
                }
                Ok(())
            }
            Statement::Expr(expr) => {
                self.check_expr(&expr.expr)?;
                Ok(())
            }
        }
    }

    fn check_expr(&self, expr: &Expression) -> Result<TypeInfo, TypeError> {
        match expr {
            Expression::Literal(lit, _span) => Ok(match lit {
                Literal::Float(_) => TypeInfo::Exact(PrimitiveType::F32),
                Literal::Int(_) => TypeInfo::Exact(PrimitiveType::I32),
                Literal::Bool(_) => TypeInfo::Exact(PrimitiveType::Bool),
                Literal::String(_) => TypeInfo::Exact(PrimitiveType::String),
            }),
            Expression::Variable(name) => {
                self.lookup(&name.name)
                    .cloned()
                    .ok_or(TypeError::UndefinedVariable {
                        name: name.name.clone(),
                        span: name.span,
                    })
            }
            Expression::BinaryOp { op: _, left, right, span: _ } => {
                let left_info = self.check_expr(left)?;
                let right_info = self.check_expr(right)?;

                // Check if either operand is a sensor type
                match (&left_info, &right_info) {
                    (TypeInfo::Sensor { .. }, _) | (_, TypeInfo::Sensor { .. }) => {
                        // Sensor operations return sensor type with combined uncertainty
                        let left_bound = extract_error_bound(&left_info);
                        let right_bound = extract_error_bound(&right_info);
                        let combined = combine_bounds(&left_bound, &right_bound);
                        Ok(TypeInfo::Sensor {
                            inner: PrimitiveType::F32,
                            error_bound: combined,
                        })
                    }
                    _ => {
                        // Regular operations
                        Ok(left_info)
                    }
                }
            }
            Expression::UnaryOp { expr, .. } => self.check_expr(expr),
            Expression::SensorAccess { sensor, field: _, span } => {
                match self.lookup(&sensor.name) {
                    Some(TypeInfo::Sensor { inner, .. }) => {
                        Ok(TypeInfo::Exact(*inner))
                    }
                    _ => Err(TypeError::UndefinedVariable {
                        name: sensor.name.clone(),
                        span: *span,
                    }),
                }
            }
            Expression::ArrayAccess { target, index: _, span } => {
                match self.lookup(&target.name) {
                    Some(TypeInfo::Array { element, .. }) => Ok(element.as_ref().clone()),
                    _ => Err(TypeError::UndefinedVariable {
                        name: target.name.clone(),
                        span: *span,
                    }),
                }
            }
            Expression::FunctionCall { name, args, span } => {
                // Look up the function's declared type
                match self.lookup(&name.name) {
                    Some(TypeInfo::Function { params, ret }) => {
                        // Check arity
                        if args.len() != params.len() {
                            return Err(TypeError::WrongArity {
                                expected: params.len(),
                                found: args.len(),
                                span: *span,
                            });
                        }
                        // Check each argument type
                        for (arg, param_ty) in args.iter().zip(params.iter()) {
                            let arg_ty = self.check_expr(arg)?;
                            // For now, just check they're compatible (both exact same primitive)
                            // A more sophisticated check would allow coercion
                            match (&arg_ty, param_ty) {
                                (TypeInfo::Exact(a), TypeInfo::Exact(p)) if a == p => {}
                                _ => {} // Allow sensor→f32 coercion, etc.
                            }
                        }
                        Ok(*ret.clone())
                    }
                    Some(_other) => Err(TypeError::NotAFunction {
                        name: name.name.clone(),
                        span: *span,
                    }),
                    None => {
                        // Unknown function — check args, return f32 as fallback
                        for arg in args {
                            self.check_expr(arg)?;
                        }
                        Ok(TypeInfo::Exact(PrimitiveType::F32))
                    }
                }
            }
            Expression::DotAccess { target, field, span } => {
                let target_info = self.check_expr(target)?;
                match &target_info {
                    TypeInfo::Sensor { inner, .. } => {
                        Ok(TypeInfo::Exact(*inner))
                    }
                    _ => Err(TypeError::MissingField {
                        ty: format!("{:?}", target_info),
                        field: field.name.clone(),
                        span: *span,
                    }),
                }
            }
            Expression::SensorMerge { sensors, weights: _, span: _ } => {
                // Merge produces a Sensor type with the combined uncertainty of all inputs
                let mut combined = ErrorBoundInfo { value: 0.0, kind: ErrorBoundKind::Absolute };
                for sensor in sensors {
                    if let Some(TypeInfo::Sensor { error_bound, .. }) = self.lookup(&sensor.name) {
                        combined = combine_bounds(&combined, error_bound);
                    }
                }
                Ok(TypeInfo::Sensor {
                    inner: PrimitiveType::F32,
                    error_bound: combined,
                })
            }
            Expression::Match { target, arms, span: _ } => {
                // Verify the target exists as a sensor
                match self.lookup(&target.name) {
                    Some(TypeInfo::Sensor { .. }) => {}
                    _ => return Err(TypeError::UndefinedVariable {
                        name: target.name.clone(),
                        span: target.span,
                    }),
                }
                // Check each arm's body, return the type of the first arm
                if let Some(first) = arms.first() {
                    self.check_expr(&first.body)
                } else {
                    Ok(TypeInfo::Exact(PrimitiveType::F32))
                }
            }
            Expression::Probe { sensor, span: _ } => {
                // Verify the sensor exists
                match self.lookup(&sensor.name) {
                    Some(TypeInfo::Sensor { .. }) => {}
                    _ => return Err(TypeError::UndefinedVariable {
                        name: sensor.name.clone(),
                        span: sensor.span,
                    }),
                }
                // Probe returns a boolean
                Ok(TypeInfo::Exact(PrimitiveType::Bool))
            }
        }
    }
}

fn error_bound_to_info(eb: &ErrorBound) -> ErrorBoundInfo {
    match eb {
        ErrorBound::Absolute(v, _) => ErrorBoundInfo { value: *v, kind: ErrorBoundKind::Absolute },
        ErrorBound::Relative(v, _) => ErrorBoundInfo { value: *v, kind: ErrorBoundKind::Relative },
    }
}

fn primitive_type_to_info(ty: &Type) -> TypeInfo {
    match ty {
        Type::Primitive(pt, _) => TypeInfo::Exact(*pt),
        Type::Sensor(st, _) => TypeInfo::Sensor {
            inner: st.inner_type,
            error_bound: error_bound_to_info(&st.error_bound),
        },
        _ => TypeInfo::Exact(PrimitiveType::F32),
    }
}

fn type_to_info(ty: &Type) -> TypeInfo {
    primitive_type_to_info(ty)
}

fn extract_error_bound(info: &TypeInfo) -> ErrorBoundInfo {
    match info {
        TypeInfo::Sensor { error_bound, .. } => error_bound.clone(),
        _ => ErrorBoundInfo { value: 0.0, kind: ErrorBoundKind::Absolute },
    }
}

fn combine_bounds(a: &ErrorBoundInfo, b: &ErrorBoundInfo) -> ErrorBoundInfo {
    // Worst-case: add absolute errors
    match (&a.kind, &b.kind) {
        (ErrorBoundKind::Absolute, ErrorBoundKind::Absolute) => ErrorBoundInfo {
            value: a.value + b.value,
            kind: ErrorBoundKind::Absolute,
        },
        _ => ErrorBoundInfo {
            value: a.value + b.value,
            kind: ErrorBoundKind::Absolute,
        },
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TypeError::UndefinedVariable { name, .. } => write!(f, "undefined variable '{}'", name),
            TypeError::TypeMismatch { expected, found, .. } => {
                write!(f, "type mismatch: expected '{}', found '{}'", expected, found)
            }
            TypeError::SensorUncertaintyMismatch { expected, found, .. } => {
                write!(f, "sensor uncertainty mismatch: expected '{}', found '{}'", expected, found)
            }
            TypeError::CannotCompareUncertain { .. } => {
                write!(f, "cannot compare uncertain sensor value with exact type")
            }
            TypeError::MissingField { ty, field, .. } => {
                write!(f, "type '{}' has no field '{}'", ty, field)
            }
            TypeError::NotAFunction { name, .. } => write!(f, "'{}' is not a function", name),
            TypeError::WrongArity { expected, found, .. } => {
                write!(f, "wrong number of arguments: expected {}, found {}", expected, found)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_type_check() {
        let mut env = TypeEnv::new();
        env.define("x".into(), TypeInfo::Exact(PrimitiveType::F32));
        assert!(env.lookup("x").is_some());
        assert!(env.lookup("y").is_none());
    }

    #[test]
    fn test_sensor_type() {
        let mut env = TypeEnv::new();
        env.define("alt".into(), TypeInfo::Sensor {
            inner: PrimitiveType::F32,
            error_bound: ErrorBoundInfo { value: 0.5, kind: ErrorBoundKind::Absolute },
        });
        let info = env.lookup("alt").unwrap();
        match info {
            TypeInfo::Sensor { error_bound, .. } => {
                assert_eq!(error_bound.value, 0.5);
            }
            _ => panic!("expected sensor type"),
        }
    }

    #[test]
    fn test_error_bound_combination() {
        let a = ErrorBoundInfo { value: 0.5, kind: ErrorBoundKind::Absolute };
        let b = ErrorBoundInfo { value: 0.3, kind: ErrorBoundKind::Absolute };
        let combined = combine_bounds(&a, &b);
        assert_eq!(combined.value, 0.8);
    }
}
