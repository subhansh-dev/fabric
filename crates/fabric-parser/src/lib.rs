use fabric_ast::*;
use fabric_core::{Ident, Span, Duration, TimeUnit};
use fabric_lexer::{Token, SpannedToken};

pub struct Parser<'tokens> {
    tokens: Vec<SpannedToken<'tokens>>,
    pos: usize,
    errors: Vec<ParseError>,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl<'tokens> Parser<'tokens> {
    pub fn new(tokens: Vec<SpannedToken<'tokens>>) -> Self {
        Self { tokens, pos: 0, errors: Vec::new() }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.token)
    }

    fn peek_span(&self) -> Span {
        self.tokens.get(self.pos).map(|t| t.span).unwrap_or(Span::dummy())
    }

    fn advance(&mut self) -> Option<SpannedToken<'tokens>> {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(token)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<Span, ParseError> {
        let span = self.peek_span();
        match self.advance() {
            Some(token) if token.token == *expected => Ok(span),
            Some(token) => Err(ParseError {
                message: format!("Expected {:?}, got {:?}", expected, token.token),
                span: token.span,
            }),
            None => Err(ParseError {
                message: format!("Expected {:?}, got EOF", expected),
                span,
            }),
        }
    }

    fn expect_ident(&mut self) -> Result<(Ident, Span), ParseError> {
        let span = self.peek_span();
        match self.advance() {
            Some(token) => match token.token {
                Token::Ident => {
                    let name = token.text.to_string();
                    Ok((Ident::new(name, span), span))
                }
                _ => Err(ParseError {
                    message: format!("Expected identifier, got {:?}", token.token),
                    span: token.span,
                }),
            },
            None => Err(ParseError {
                message: "Expected identifier, got EOF".into(),
                span,
            }),
        }
    }

    fn expect_number(&mut self) -> Result<(f64, Span), ParseError> {
        let span = self.peek_span();
        match self.advance() {
            Some(token) => match token.token {
                Token::Float(v) => Ok((v, span)),
                Token::Integer(v) => Ok((v, span)),
                _ => Err(ParseError {
                    message: format!("Expected number, got {:?}", token.token),
                    span: token.span,
                }),
            },
            None => Err(ParseError {
                message: "Expected number, got EOF".into(),
                span,
            }),
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    // ─── Top-level parsing ──────────────────────────────────────────────

    pub fn parse_program(&mut self) -> Result<Program, Vec<ParseError>> {
        let mut declarations = Vec::new();
        let _start = self.peek_span();

        while !self.at_end() {
            match self.parse_declaration() {
                Ok(decl) => declarations.push(decl),
                Err(e) => {
                    self.errors.push(e);
                    // Try to recover by skipping to next declaration
                    self.recover_to_declaration();
                }
            }
        }

        if !self.errors.is_empty() {
            return Err(self.errors.clone());
        }

        let span = if declarations.is_empty() {
            Span::dummy()
        } else {
            declarations.first().unwrap().span().merge(&declarations.last().unwrap().span())
        };

        Ok(Program { declarations, span })
    }

    fn recover_to_declaration(&mut self) {
        while !self.at_end() {
            match self.peek() {
                Some(Token::Sensor) | Some(Token::Actuator) | Some(Token::Let)
                | Some(Token::Loop) | Some(Token::When) | Some(Token::Fn) => break,
                _ => { self.advance(); }
            }
        }
    }

    // ─── Declarations ───────────────────────────────────────────────────

    fn parse_declaration(&mut self) -> Result<Declaration, ParseError> {
        match self.peek() {
            Some(Token::Sensor) => self.parse_sensor_decl(),
            Some(Token::Actuator) => self.parse_actuator_decl(),
            Some(Token::Let) => self.parse_var_decl(),
            Some(Token::Loop) => self.parse_loop_decl(),
            Some(Token::When) => self.parse_fallback_decl(),
            Some(Token::Fn) => self.parse_fn_decl(),
            Some(Token::Drone) => self.parse_drone_decl(),
            Some(tok) => Err(ParseError {
                message: format!("Unexpected token {:?}, expected declaration", tok),
                span: self.peek_span(),
            }),
            None => Err(ParseError {
                message: "Unexpected EOF, expected declaration".into(),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_sensor_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Sensor)?;
        let (name, _name_span) = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let (sensor_type, type_span) = self.parse_sensor_type()?;
        let span = start.merge(&type_span);
        Ok(Declaration::Sensor(SensorDecl { name, sensor_type, span }))
    }

    fn parse_actuator_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Actuator)?;
        let (name, _name_span) = self.expect_ident()?;
        self.expect(&Token::Colon)?;
        let (actuator_type, type_span) = self.parse_actuator_type()?;
        let span = start.merge(&type_span);
        Ok(Declaration::Actuator(ActuatorDecl { name, actuator_type, span }))
    }

    fn parse_var_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Let)?;
        let (name, _name_span) = self.expect_ident()?;
        let ty = if self.peek() == Some(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Token::Equals)?;
        let value = self.parse_expr()?;
        let span = start.merge(&value.span());
        Ok(Declaration::Variable(VarDecl { name, ty, value, span }))
    }

    fn parse_loop_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Loop)?;
        let (name, _name_span) = self.expect_ident()?;
        // Optional parens (e.g., `loop control() within`)
        if self.peek() == Some(&Token::LParen) {
            self.advance();
            self.expect(&Token::RParen)?;
        }
        self.expect(&Token::Within)?;
        let deadline = self.parse_duration()?;
        self.expect(&Token::LBrace)?;
        let mut body = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            body.push(self.parse_statement()?);
        }
        let end = self.expect(&Token::RBrace)?;
        let span = start.merge(&end);
        Ok(Declaration::Loop(LoopDecl { name, deadline, body, span }))
    }

    fn parse_fallback_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::When)?;
        // Optional "sensor" keyword
        if self.peek() == Some(&Token::Sensor) {
            self.advance();
        }
        // Parse sensor name — can be with or without parens
        let sensor_name = if self.peek() == Some(&Token::LParen) {
            self.advance();
            let (name, _) = self.expect_ident()?;
            self.expect(&Token::RParen)?;
            name
        } else {
            let (name, _) = self.expect_ident()?;
            name
        };
        self.expect(&Token::Unavailable)?;
        if self.peek() == Some(&Token::For) {
            self.advance();
        }
        let timeout = self.parse_duration()?;
        self.expect(&Token::LBrace)?;
        self.expect(&Token::Fallback)?;
        self.expect(&Token::To)?;
        let fallback_expr = self.parse_expr()?;
        let end = self.expect(&Token::RBrace)?;
        let span = start.merge(&end);
        Ok(Declaration::Fallback(FallbackDecl { sensor_name, timeout, fallback_expr, span }))
    }

    fn parse_fn_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Fn)?;
        let (name, _name_span) = self.expect_ident()?;
        self.expect(&Token::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(&Token::RParen)?;
        let return_type = if self.peek() == Some(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Token::LBrace)?;
        let mut body = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            body.push(self.parse_statement()?);
        }
        let end = self.expect(&Token::RBrace)?;
        let span = start.merge(&end);
        Ok(Declaration::Function(FunctionDecl { name, params, return_type, body, span }))
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, ParseError> {
        let mut params = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            return Ok(params);
        }
        loop {
            let (name, name_span) = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;
            let span = name_span.merge(&ty.span());
            params.push(Param { name, ty, span });
            if self.peek() == Some(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_drone_decl(&mut self) -> Result<Declaration, ParseError> {
        let start = self.expect(&Token::Drone)?;
        let (name, _name_span) = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut count = 4u32;
        let mut spacing = 2.0f64;
        let mut formation = Formation::Grid;
        let mut fallback_expr = None;

        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            match self.peek() {
                Some(Token::Count) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    if let Some(Token::Integer(n)) = self.peek() {
                        count = *n as u32;
                        self.advance();
                    }
                }
                Some(Token::Spacing) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    if let Some(Token::Integer(n)) = self.peek() {
                        spacing = *n;
                        self.advance();
                    }
                }
                Some(Token::Formation) => {
                    self.advance();
                    self.expect(&Token::Colon)?;
                    if let Some(Token::Ident) = self.peek() {
                        let (fname, _) = self.expect_ident()?;
                        formation = match fname.name.as_str() {
                            "circle" => Formation::Circle,
                            "line" => Formation::Line,
                            "diamond" => Formation::Diamond,
                            _ => Formation::Grid,
                        };
                    }
                }
                _ => {
                    self.advance(); // skip unknown tokens
                }
            }
        }

        let end = self.expect(&Token::RBrace)?;
        let span = start.merge(&end);

        Ok(Declaration::Drone(DroneDecl {
            name,
            count,
            spacing,
            formation,
            fallback_expr,
            span,
        }))
    }

    // ─── Statements ─────────────────────────────────────────────────────

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        match self.peek() {
            Some(Token::Read) => self.parse_read_stmt(),
            Some(Token::Write) => self.parse_write_stmt(),
            Some(Token::Let) => self.parse_let_stmt(),
            Some(Token::Return) => self.parse_return_stmt(),
            Some(Token::If) => self.parse_if_else_stmt(),
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_if_else_stmt(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(&Token::If)?;
        let condition = self.parse_expr()?;
        self.expect(&Token::LBrace)?;
        let mut then_body = Vec::new();
        while self.peek() != Some(&Token::RBrace) && !self.at_end() {
            then_body.push(self.parse_statement()?);
        }
        self.expect(&Token::RBrace)?;
        let else_body = if self.peek() == Some(&Token::Else) {
            self.advance();
            self.expect(&Token::LBrace)?;
            let mut body = Vec::new();
            while self.peek() != Some(&Token::RBrace) && !self.at_end() {
                body.push(self.parse_statement()?);
            }
            self.expect(&Token::RBrace)?;
            Some(body)
        } else {
            None
        };
        let end_span = self.peek_span();
        let span = start.merge(&end_span);
        Ok(Statement::IfElse { condition, then_body, else_body, span })
    }

    fn parse_read_stmt(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(&Token::Read)?;
        let (sensor, sensor_span) = self.expect_ident()?;
        let field = if self.peek() == Some(&Token::Dot) {
            self.advance();
            Some(self.expect_ident()?.0)
        } else {
            None
        };
        let span = start.merge(&sensor_span);
        let target = field.clone().unwrap_or_else(|| sensor.clone());
        Ok(Statement::Read { target, sensor, span })
    }

    fn parse_write_stmt(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(&Token::Write)?;
        let (target, target_span) = self.expect_ident()?;
        // Check for array index: write motors[0]
        let target = if self.peek() == Some(&Token::LBracket) {
            self.advance();
            let idx_expr = self.parse_expr()?;
            self.expect(&Token::RBracket)?;
            let idx_str = match &idx_expr {
                Expression::Literal(Literal::Int(n), _) => n.to_string(),
                Expression::Literal(Literal::Float(n), _) => n.to_string(),
                Expression::Variable(i) => i.name.clone(),
                _ => "...".to_string(),
            };
            Ident::new(format!("{}[{}]", target.name, idx_str), target_span)
        } else {
            target
        };
        self.expect(&Token::Equals)?;
        let value = self.parse_expr()?;
        let span = start.merge(&value.span());
        Ok(Statement::Write { target, value, span })
    }

    fn parse_let_stmt(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(&Token::Let)?;
        let (name, _name_span) = self.expect_ident()?;
        let ty = if self.peek() == Some(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Token::Equals)?;
        let value = self.parse_expr()?;
        let span = start.merge(&value.span());
        Ok(Statement::Let { name, ty, value, span })
    }

    fn parse_return_stmt(&mut self) -> Result<Statement, ParseError> {
        let start = self.expect(&Token::Return)?;
        let value = if self.peek() == Some(&Token::Semicolon) || self.peek() == Some(&Token::RBrace) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        let span = match &value {
            Some(e) => start.merge(&e.span()),
            None => start,
        };
        Ok(Statement::Return { value, span })
    }

    fn parse_expr_stmt(&mut self) -> Result<Statement, ParseError> {
        let expr = self.parse_expr()?;
        let span = expr.span();
        Ok(Statement::Expr(StatementExpr { expr, span }))
    }

    // ─── Expressions (Pratt parser for precedence) ──────────────────────

    fn parse_expr(&mut self) -> Result<Expression, ParseError> {
        self.parse_expr_bp(0)
    }

    /// Pratt parser with binding power
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expression, ParseError> {
        let mut lhs = self.parse_unary()?;

        loop {
            let op = match self.peek() {
                Some(Token::Plus) => Some(BinOp::Add),
                Some(Token::Minus) => Some(BinOp::Sub),
                Some(Token::Star) => Some(BinOp::Mul),
                Some(Token::Slash) => Some(BinOp::Div),
                Some(Token::Percent) => Some(BinOp::Mod),
                Some(Token::EqEq) => Some(BinOp::Eq),
                Some(Token::Ne) => Some(BinOp::Ne),
                Some(Token::Lt) => Some(BinOp::Lt),
                Some(Token::Gt) => Some(BinOp::Gt),
                Some(Token::Le) => Some(BinOp::Le),
                Some(Token::Ge) => Some(BinOp::Ge),
                Some(Token::AmpAmp) => Some(BinOp::And),
                Some(Token::PipePipe) => Some(BinOp::Or),
                _ => None,
            };

            let op = match op {
                Some(op) => op,
                None => break,
            };

            let (l_bp, r_bp) = infix_binding_power(op);
            if l_bp < min_bp {
                break;
            }

            let _op_span = self.peek_span();
            self.advance();
            let rhs = self.parse_expr_bp(r_bp)?;
            let span = lhs.span().merge(&rhs.span());
            lhs = Expression::BinaryOp {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
                span,
            };
        }

        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expression, ParseError> {
        match self.peek() {
            Some(Token::Minus) => {
                let start = self.peek_span();
                self.advance();
                let expr = self.parse_unary()?;
                let span = start.merge(&expr.span());
                Ok(Expression::UnaryOp { op: UnaryOp::Neg, expr: Box::new(expr), span })
            }
            Some(Token::Bang) => {
                let start = self.peek_span();
                self.advance();
                let expr = self.parse_unary()?;
                let span = start.merge(&expr.span());
                Ok(Expression::UnaryOp { op: UnaryOp::Not, expr: Box::new(expr), span })
            }
            _ => self.parse_atom(),
        }
    }

    fn parse_atom(&mut self) -> Result<Expression, ParseError> {
        match self.peek() {
            Some(Token::Float(_)) | Some(Token::Integer(_)) => self.parse_literal(),
            Some(Token::True) | Some(Token::False) => self.parse_literal(),
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            Some(Token::Merge) => self.parse_merge_expr(),
            Some(Token::Match) => self.parse_match_expr(),
            Some(Token::Probe) => self.parse_probe_expr(),
            Some(Token::Read) => {
                self.advance();
                let (sensor, sensor_span) = self.expect_ident()?;
                let field = if self.peek() == Some(&Token::Dot) {
                    self.advance();
                    Some(self.expect_ident()?.0)
                } else {
                    None
                };
                match field {
                    Some(field) => {
                        let span = sensor_span.merge(&field.span);
                        Ok(Expression::SensorAccess { sensor, field, span })
                    }
                    None => Ok(Expression::Variable(sensor)),
                }
            }
            Some(Token::Ident) => {
                let (name, name_span) = self.expect_ident()?;
                // Check for function call
                if self.peek() == Some(&Token::LParen) {
                    self.advance();
                    let args = self.parse_expr_list()?;
                    self.expect(&Token::RParen)?;
                    let span = name_span; // simplified
                    return Ok(Expression::FunctionCall { name, args, span });
                }
                // Check for array access
                if self.peek() == Some(&Token::LBracket) {
                    self.advance();
                    let index = Box::new(self.parse_expr()?);
                    self.expect(&Token::RBracket)?;
                    let span = name_span;
                    return Ok(Expression::ArrayAccess { target: name, index, span });
                }
                // Check for dot access
                if self.peek() == Some(&Token::Dot) {
                    self.advance();
                    let (field, field_span) = self.expect_ident()?;
                    let span = name_span.merge(&field_span);
                    let target = Box::new(Expression::Variable(name));
                    return Ok(Expression::DotAccess { target, field, span });
                }
                Ok(Expression::Variable(name))
            }
            _ => Err(ParseError {
                message: format!("Unexpected token {:?} in expression", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_merge_expr(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect(&Token::Merge)?;
        let mut sensors = Vec::new();
        let mut weights = Vec::new();
        // Parse at least two sensor names
        let (name1, _) = self.expect_ident()?;
        sensors.push(name1);
        let (name2, _) = self.expect_ident()?;
        sensors.push(name2);
        // Optional weights: [0.7, 0.3]
        if self.peek() == Some(&Token::LBracket) {
            self.advance();
            loop {
                let expr = self.parse_expr()?;
                weights.push(expr);
                if self.peek() == Some(&Token::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(&Token::RBracket)?;
        }
        let end_span = self.peek_span();
        let span = start.merge(&end_span);
        Ok(Expression::SensorMerge { sensors, weights, span })
    }

    fn parse_match_expr(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect(&Token::Match)?;
        let (target, _) = self.expect_ident()?;
        self.expect(&Token::LBrace)?;
        let mut arms = Vec::new();
        while self.peek() != Some(&Token::RBrace) {
            let pattern = match self.peek() {
                Some(Token::OkKw) => { self.advance(); MatchPattern::Ok }
                Some(Token::TimeoutKw) => { self.advance(); MatchPattern::Timeout }
                Some(Token::ErrorKw) => { self.advance(); MatchPattern::Error }
                _ => return Err(ParseError {
                    message: "Expected ok, timeout, or error".into(),
                    span: self.peek_span(),
                }),
            };
            self.expect(&Token::FatArrow)?;
            let body = self.parse_expr()?;
            let arm_span = self.peek_span();
            arms.push(MatchArm { pattern, body, span: arm_span });
            if self.peek() == Some(&Token::Comma) {
                self.advance();
            }
        }
        self.expect(&Token::RBrace)?;
        let span = start.merge(&self.peek_span());
        Ok(Expression::Match { target, arms, span })
    }

    fn parse_probe_expr(&mut self) -> Result<Expression, ParseError> {
        let start = self.expect(&Token::Probe)?;
        let (sensor, sensor_span) = self.expect_ident()?;
        let span = start.merge(&sensor_span);
        Ok(Expression::Probe { sensor, span })
    }

    fn parse_literal(&mut self) -> Result<Expression, ParseError> {
        let span = self.peek_span();
        match self.advance() {
            Some(token) => match token.token {
                Token::Float(v) => Ok(Expression::Literal(Literal::Float(v), span)),
                Token::Integer(v) => Ok(Expression::Literal(Literal::Int(v as i64), span)),
                Token::True => Ok(Expression::Literal(Literal::Bool(true), span)),
                Token::False => Ok(Expression::Literal(Literal::Bool(false), span)),
                _ => Err(ParseError {
                    message: format!("Expected literal, got {:?}", token.token),
                    span: token.span,
                }),
            },
            None => Err(ParseError {
                message: "Expected literal, got EOF".into(),
                span,
            }),
        }
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expression>, ParseError> {
        let mut exprs = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            return Ok(exprs);
        }
        loop {
            exprs.push(self.parse_expr()?);
            if self.peek() == Some(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(exprs)
    }

    // ─── Types ──────────────────────────────────────────────────────────

    fn parse_type(&mut self) -> Result<Type, ParseError> {
        match self.peek() {
            Some(Token::SensorTypeOpen) => {
                let (st, span) = self.parse_sensor_type()?;
                Ok(Type::Sensor(st, span))
            }
            Some(Token::Ident) => {
                let (name, name_span) = self.expect_ident()?;
                // Check for array type: Motor[4]
                if self.peek() == Some(&Token::LBracket) {
                    self.advance();
                    let _size = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    let span = name_span;
                    return Ok(Type::Array(Box::new(Type::Named(name)), ArraySize::Named(Ident::new("size_placeholder", span)), span));
                }
                // Map primitive type names
                let pt = match name.name.as_str() {
                    "f32" => PrimitiveType::F32,
                    "f64" => PrimitiveType::F64,
                    "i32" => PrimitiveType::I32,
                    "i64" => PrimitiveType::I64,
                    "bool" => PrimitiveType::Bool,
                    "string" => PrimitiveType::String,
                    _ => return Ok(Type::Named(name)),
                };
                Ok(Type::Primitive(pt, name_span))
            }
            _ => Err(ParseError {
                message: format!("Expected type, got {:?}", self.peek()),
                span: self.peek_span(),
            }),
        }
    }

    fn parse_sensor_type(&mut self) -> Result<(SensorType, Span), ParseError> {
        let start = self.expect(&Token::SensorTypeOpen)?;
        // Parse inner type (e.g., f32)
        let (inner_name, _) = self.expect_ident()?;
        let inner_type = match inner_name.name.as_str() {
            "f32" => PrimitiveType::F32,
            "f64" => PrimitiveType::F64,
            "i32" => PrimitiveType::I32,
            _ => PrimitiveType::F32,
        };
        self.expect(&Token::Comma)?;
        // Parse error bound (e.g., ±0.5m or ±5%)
        let error_bound = self.parse_error_bound()?;
        let end = self.expect(&Token::Gt)?;
        let span = start.merge(&end);
        Ok((SensorType { inner_type, error_bound, span }, span))
    }

    fn parse_error_bound(&mut self) -> Result<ErrorBound, ParseError> {
        let span = self.peek_span();
        match self.advance() {
            Some(token) => match token.token {
                Token::ErrorBound(Some(v)) => {
                    if self.peek() == Some(&Token::Percent) {
                        self.advance();
                        Ok(ErrorBound::Relative(v, span))
                    } else {
                        Ok(ErrorBound::Absolute(v, span))
                    }
                }
                _ => Err(ParseError {
                    message: format!("Expected error bound (±value), got {:?}", token.token),
                    span: token.span,
                }),
            },
            None => Err(ParseError {
                message: "Expected error bound, got EOF".into(),
                span,
            }),
        }
    }

    fn parse_actuator_type(&mut self) -> Result<(ActuatorType, Span), ParseError> {
        let (name, name_span) = self.expect_ident()?;
        let base = match name.name.as_str() {
            "Motor" => ActuatorType::Motor,
            "Servo" => ActuatorType::Servo,
            "Led" => ActuatorType::Led,
            _ => ActuatorType::Custom(name),
        };
        // Check for array: Motor[4]
        if self.peek() == Some(&Token::LBracket) {
            self.advance();
            let (size_val, size_span) = self.expect_number()?;
            self.expect(&Token::RBracket)?;
            let span = name_span.merge(&size_span);
            return Ok((ActuatorType::Array(Box::new(base), ArraySize::Fixed(size_val as usize, size_span), span), span));
        }
        Ok((base, name_span))
    }

    // ─── Duration ───────────────────────────────────────────────────────

    fn parse_duration(&mut self) -> Result<Duration, ParseError> {
        let span = self.peek_span();
        // The DurationValue token contains the full text like "2ms" or "500us"
        let token = self.advance().ok_or_else(|| ParseError {
            message: "Expected duration value".into(),
            span,
        })?;
        let text = token.text;
        // Parse the numeric part
        let num_part: String = text.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let value: f64 = num_part.parse().unwrap_or(0.0);
        // Determine unit from the suffix
        let unit = if text.ends_with("us") {
            TimeUnit::Microseconds
        } else if text.ends_with("ms") {
            TimeUnit::Milliseconds
        } else {
            TimeUnit::Seconds
        };
        Ok(Duration { value, unit, span })
    }
}

/// Returns (left binding power, right binding power) for an infix operator
fn infix_binding_power(op: BinOp) -> (u8, u8) {
    match op {
        BinOp::Or => (1, 2),
        BinOp::And => (3, 4),
        BinOp::Eq | BinOp::Ne => (5, 6),
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => (7, 8),
        BinOp::Add | BinOp::Sub => (9, 10),
        BinOp::Mul | BinOp::Div | BinOp::Mod => (11, 12),
    }
}

/// Public API: parse tokens into a Program
pub fn parse(tokens: Vec<SpannedToken<'_>>) -> Result<Program, Vec<ParseError>> {
    let mut parser = Parser::new(tokens);
    parser.parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_lexer::tokenize;

    fn parse_str(source: &str) -> Result<Program, Vec<ParseError>> {
        let tokens = tokenize(source).unwrap();
        parse(tokens)
    }

    #[test]
    fn test_parse_var_decl() {
        let result = parse_str("let x: f32 = 10.0");
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let prog = result.unwrap();
        assert_eq!(prog.declarations.len(), 1);
    }

    #[test]
    fn test_parse_loop_decl() {
        let result = parse_str("loop stabilize() within 2ms { let x = 1 }");
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let prog = result.unwrap();
        assert_eq!(prog.declarations.len(), 1);
        match &prog.declarations[0] {
            Declaration::Loop(l) => {
                assert_eq!(l.name.name, "stabilize");
                assert_eq!(l.deadline.value, 2.0);
                assert_eq!(l.body.len(), 1);
            }
            _ => panic!("Expected loop declaration"),
        }
    }

    #[test]
    fn test_pratt_precedence() {
        let result = parse_str("let x = 1 + 2 * 3");
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let prog = result.unwrap();
        match &prog.declarations[0] {
            Declaration::Variable(v) => {
                // Should parse as 1 + (2 * 3), not (1 + 2) * 3
                match &v.value {
                    Expression::BinaryOp { op: BinOp::Add, right, .. } => {
                        matches!(right.as_ref(), Expression::BinaryOp { op: BinOp::Mul, .. });
                    }
                    _ => panic!("Expected addition"),
                }
            }
            _ => panic!("Expected variable"),
        }
    }

    #[test]
    fn test_parse_expr_simple() {
        let result = parse_str("let x = 1 + 2");
        assert!(result.is_ok(), "Failed: {:?}", result.err());
    }

    #[test]
    fn test_parse_let_no_type() {
        let result = parse_str("let x = 5");
        assert!(result.is_ok(), "Failed: {:?}", result.err());
    }
}
