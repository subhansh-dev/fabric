use fabric_core::{Ident, Span, Duration};

/// A complete Fabric program
#[derive(Debug, Clone)]
pub struct Program {
    pub declarations: Vec<Declaration>,
    pub span: Span,
}

/// Top-level declarations
#[derive(Debug, Clone)]
pub enum Declaration {
    Sensor(SensorDecl),
    Actuator(ActuatorDecl),
    Variable(VarDecl),
    Loop(LoopDecl),
    Fallback(FallbackDecl),
    Function(FunctionDecl),
}

impl Declaration {
    pub fn span(&self) -> Span {
        match self {
            Declaration::Sensor(d) => d.span,
            Declaration::Actuator(d) => d.span,
            Declaration::Variable(d) => d.span,
            Declaration::Loop(d) => d.span,
            Declaration::Fallback(d) => d.span,
            Declaration::Function(d) => d.span,
        }
    }
}

/// sensor imu: IMU
/// sensor altitude: Sensor<f32, ±0.5m>
#[derive(Debug, Clone)]
pub struct SensorDecl {
    pub name: Ident,
    pub sensor_type: SensorType,
    pub span: Span,
}

/// Actuator declaration
/// actuator motors: Motor[4]
#[derive(Debug, Clone)]
pub struct ActuatorDecl {
    pub name: Ident,
    pub actuator_type: ActuatorType,
    pub span: Span,
}

/// Variable declaration
/// let target_altitude: f32 = 10.0
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: Ident,
    pub ty: Option<Type>,
    pub value: Expression,
    pub span: Span,
}

/// Loop with deadline
/// loop stabilize() within 2ms { ... }
#[derive(Debug, Clone)]
pub struct LoopDecl {
    pub name: Ident,
    pub deadline: Duration,
    pub body: Vec<Statement>,
    pub span: Span,
}

/// Fallback declaration
/// when sensor(altitude) unavailable for 200ms {
///     fallback to estimated_altitude
/// }
#[derive(Debug, Clone)]
pub struct FallbackDecl {
    pub sensor_name: Ident,
    pub timeout: Duration,
    pub fallback_expr: Expression,
    pub span: Span,
}

/// Function declaration
/// fn dead_reckoning() { ... }
#[derive(Debug, Clone)]
pub struct FunctionDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Ident,
    pub ty: Type,
    pub span: Span,
}

/// Statements
#[derive(Debug, Clone)]
pub enum Statement {
    Read {
        target: Ident,
        sensor: Ident,
        span: Span,
    },
    Write {
        target: Ident,
        value: Expression,
        span: Span,
    },
    Assign {
        target: Ident,
        value: Expression,
        span: Span,
    },
    Let {
        name: Ident,
        ty: Option<Type>,
        value: Expression,
        span: Span,
    },
    Return {
        value: Option<Expression>,
        span: Span,
    },
    IfElse {
        condition: Expression,
        then_body: Vec<Statement>,
        else_body: Option<Vec<Statement>>,
        span: Span,
    },
    Expr(StatementExpr),
}

#[derive(Debug, Clone)]
pub struct StatementExpr {
    pub expr: Expression,
    pub span: Span,
}

/// Expressions
#[derive(Debug, Clone)]
pub enum Expression {
    Literal(Literal, Span),
    Variable(Ident),
    BinaryOp {
        op: BinOp,
        left: Box<Expression>,
        right: Box<Expression>,
        span: Span,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expression>,
        span: Span,
    },
    SensorAccess {
        sensor: Ident,
        field: Ident,
        span: Span,
    },
    ArrayAccess {
        target: Ident,
        index: Box<Expression>,
        span: Span,
    },
    FunctionCall {
        name: Ident,
        args: Vec<Expression>,
        span: Span,
    },
    /// altitude.value — explicit unwrap from uncertainty
    DotAccess {
        target: Box<Expression>,
        field: Ident,
        span: Span,
    },
    /// merge sensor_a sensor_b [w1, w2] — weighted sensor fusion
    SensorMerge {
        sensors: Vec<Ident>,
        weights: Vec<Expression>,
        span: Span,
    },
    /// match sensor { ok => ..., timeout => ..., error => ... }
    Match {
        target: Ident,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// probe sensor_name — returns true if sensor is responding
    Probe {
        sensor: Ident,
        span: Span,
    },
}

/// A single arm in a match expression: `ok => expr`
#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: MatchPattern,
    pub body: Expression,
    pub span: Span,
}

/// Sensor state patterns for match expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPattern {
    Ok,
    Timeout,
    Error,
}

impl Expression {
    pub fn span(&self) -> Span {
        match self {
            Expression::Literal(_, s) => *s,
            Expression::Variable(i) => i.span,
            Expression::BinaryOp { span, .. } => *span,
            Expression::UnaryOp { span, .. } => *span,
            Expression::SensorAccess { span, .. } => *span,
            Expression::ArrayAccess { span, .. } => *span,
            Expression::FunctionCall { span, .. } => *span,
            Expression::DotAccess { span, .. } => *span,
            Expression::SensorMerge { span, .. } => *span,
            Expression::Match { span, .. } => *span,
            Expression::Probe { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Types
#[derive(Debug, Clone)]
pub enum Type {
    /// Primitive: f32, i32, bool, string
    Primitive(PrimitiveType, Span),
    /// Sensor type: Sensor<f32, ±0.5m>
    Sensor(SensorType, Span),
    /// Array type: Motor[4]
    Array(Box<Type>, ArraySize, Span),
    /// Function type: (f32, f32) -> f32
    Function(Vec<Type>, Box<Type>, Span),
    /// Custom type name
    Named(Ident),
}

impl Type {
    pub fn span(&self) -> Span {
        match self {
            Type::Primitive(_, s) => *s,
            Type::Sensor(_, s) => *s,
            Type::Array(_, _, s) => *s,
            Type::Function(_, _, s) => *s,
            Type::Named(i) => i.span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    F32,
    F64,
    I32,
    I64,
    Bool,
    String,
}

impl PrimitiveType {
    pub fn name(&self) -> &str {
        match self {
            PrimitiveType::F32 => "f32",
            PrimitiveType::F64 => "f64",
            PrimitiveType::I32 => "i32",
            PrimitiveType::I64 => "i64",
            PrimitiveType::Bool => "bool",
            PrimitiveType::String => "string",
        }
    }
}

/// Sensor type with uncertainty bound
#[derive(Debug, Clone)]
pub struct SensorType {
    pub inner_type: PrimitiveType,
    pub error_bound: ErrorBound,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ErrorBound {
    /// ±0.5 (absolute error in same units as sensor)
    Absolute(f64, Span),
    /// ±5% (relative error)
    Relative(f64, Span),
}

impl ErrorBound {
    pub fn display_unit(&self) -> String {
        match self {
            ErrorBound::Absolute(v, _) => format!("±{}", v),
            ErrorBound::Relative(v, _) => format!("±{}%", v),
        }
    }
}

/// Actuator type
#[derive(Debug, Clone)]
pub enum ActuatorType {
    Motor,
    Servo,
    Led,
    Custom(Ident),
    Array(Box<ActuatorType>, ArraySize, Span),
}

#[derive(Debug, Clone)]
pub enum ArraySize {
    Fixed(usize, Span),
    Named(Ident),
}
