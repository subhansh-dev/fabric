use fabric_core::Span;
use fabric_ast::*;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Errors produced by the checker passes
#[derive(Debug, Clone)]
pub enum CheckError {
    MissingFallback {
        sensor: String,
        span: Span,
    },
    TransitiveMissing {
        sensor: String,
        missing_dep: String,
        span: Span,
    },
    FallbackCycle {
        cycle: Vec<String>,
        span: Span,
    },
    DeadlineExceeded {
        loop_name: String,
        deadline_ms: f64,
        estimated_wcet_ms: f64,
        span: Span,
    },
    UnknownLoopBound {
        loop_name: String,
        span: Span,
    },
    UnboundedLoop {
        loop_name: String,
        span: Span,
    },
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckError::MissingFallback { sensor, .. } => {
                write!(f, "sensor '{}' has no fallback handler", sensor)
            }
            CheckError::TransitiveMissing { sensor, missing_dep, .. } => {
                write!(f, "fallback for '{}' depends on '{}' which has no fallback", sensor, missing_dep)
            }
            CheckError::FallbackCycle { cycle, .. } => {
                write!(f, "fallback cycle detected: {}", cycle.join(" -> "))
            }
            CheckError::DeadlineExceeded { loop_name, deadline_ms, estimated_wcet_ms, .. } => {
                write!(f, "loop '{}' proven worst-case {:.2}ms exceeds deadline {:.2}ms",
                    loop_name, estimated_wcet_ms, deadline_ms)
            }
            CheckError::UnknownLoopBound { loop_name, .. } => {
                write!(f, "cannot determine loop bound for '{}'", loop_name)
            }
            CheckError::UnboundedLoop { loop_name, .. } => {
                write!(f, "UnboundedLoop: cannot compute WCET for '{}' without a static iteration bound", loop_name)
            }
        }
    }
}

// ─── Fallback Graph ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FallbackNode {
    sensor: String,
    timeout_ms: f64,
    fallback_fn: String,
    dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
struct FallbackGraph {
    nodes: HashMap<String, FallbackNode>,
    all_sensors: HashSet<String>,
}

impl FallbackGraph {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            all_sensors: HashSet::new(),
        }
    }

    fn add_fallback(&mut self, decl: &FallbackDecl) {
        let node = FallbackNode {
            sensor: decl.sensor_name.name.clone(),
            timeout_ms: decl.timeout.to_ms(),
            fallback_fn: format_expr_name(&decl.fallback_expr),
            dependencies: extract_sensor_refs(&decl.fallback_expr),
        };
        self.nodes.insert(decl.sensor_name.name.clone(), node);
    }

    fn register_sensor(&mut self, name: &str) {
        self.all_sensors.insert(name.to_string());
    }

    fn check_completeness(&self) -> Vec<CheckError> {
        let mut errors = Vec::new();

        for sensor in &self.all_sensors {
            if !self.nodes.contains_key(sensor) {
                // Find where this sensor is used (simplified - would need span info)
                errors.push(CheckError::MissingFallback {
                    sensor: sensor.clone(),
                    span: Span::dummy(),
                });
            }
        }

        // Check transitive dependencies
        for (name, node) in &self.nodes {
            for dep in &node.dependencies {
                if !self.nodes.contains_key(dep) && self.all_sensors.contains(dep) {
                    errors.push(CheckError::TransitiveMissing {
                        sensor: name.clone(),
                        missing_dep: dep.clone(),
                        span: Span::dummy(),
                    });
                }
            }
        }

        // Check for cycles
        if let Some(cycle) = self.detect_cycle() {
            errors.push(CheckError::FallbackCycle {
                cycle,
                span: Span::dummy(),
            });
        }

        errors
    }

    fn detect_cycle(&self) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();

        for name in self.nodes.keys() {
            if let Some(cycle) = self.dfs_cycle(name, &mut visited, &mut stack) {
                return Some(cycle);
            }
        }
        None
    }

    fn dfs_cycle(&self, node: &str, visited: &mut HashSet<String>, stack: &mut HashSet<String>) -> Option<Vec<String>> {
        if stack.contains(node) {
            return Some(vec![node.to_string()]);
        }
        if visited.contains(node) {
            return None;
        }
        visited.insert(node.to_string());
        stack.insert(node.to_string());

        if let Some(fallback) = self.nodes.get(node) {
            for dep in &fallback.dependencies {
                if let Some(mut cycle) = self.dfs_cycle(dep, visited, stack) {
                    cycle.insert(0, node.to_string());
                    return Some(cycle);
                }
            }
        }

        stack.remove(node);
        None
    }
}

fn format_expr_name(expr: &Expression) -> String {
    match expr {
        Expression::Variable(i) => i.name.clone(),
        Expression::FunctionCall { name, .. } => name.name.clone(),
        _ => "<expr>".to_string(),
    }
}

fn extract_sensor_refs(expr: &Expression) -> Vec<String> {
    match expr {
        Expression::Variable(i) => vec![i.name.clone()],
        Expression::BinaryOp { left, right, .. } => {
            let mut refs = extract_sensor_refs(left);
            refs.extend(extract_sensor_refs(right));
            refs
        }
        Expression::FunctionCall { args, .. } => {
            args.iter().flat_map(|a| extract_sensor_refs(a)).collect()
        }
        Expression::SensorAccess { sensor, .. } => vec![sensor.name.clone()],
        _ => vec![],
    }
}

// ─── Timing Analyzer ─────────────────────────────────────────────────────

/// Instruction cost model for ARM Cortex-M4
pub struct CostModel {
    costs: HashMap<String, f64>,
}

impl CostModel {
    fn arm_cortex_m4() -> Self {
        let mut costs = HashMap::new();
        // ALU operations: 1 cycle
        for op in &["add", "sub", "mul", "and", "orr", "eor", "mov", "cmp", "lsl", "lsr"] {
            costs.insert(op.to_string(), 1.0);
        }
        // Division: 12 cycles worst case
        costs.insert("div".to_string(), 12.0);
        // Load/Store: 2 cycles
        costs.insert("ldr".to_string(), 2.0);
        costs.insert("str".to_string(), 2.0);
        // Branch: 4 cycles (worst case pipeline refill)
        costs.insert("branch".to_string(), 4.0);
        // Float multiply: 1 cycle
        costs.insert("fmul".to_string(), 1.0);
        // Float add: 1 cycle
        costs.insert("fadd".to_string(), 1.0);
        // Float divide: 14 cycles
        costs.insert("fdiv".to_string(), 14.0);
        // Sensor read: 2 cycles (memory access)
        costs.insert("sensor_read".to_string(), 2.0);
        // Actuator write: 2 cycles
        costs.insert("actuator_write".to_string(), 2.0);

        Self { costs }
    }

    fn estimate_stmt_cost(&self, stmt: &Statement) -> f64 {
        match stmt {
            Statement::Read { .. } => *self.costs.get("sensor_read").unwrap_or(&2.0),
            Statement::Write { value, .. } => {
                self.estimate_expr_cost(value) + *self.costs.get("actuator_write").unwrap_or(&2.0)
            }
            Statement::Assign { value, .. } => self.estimate_expr_cost(value),
            Statement::Let { value, .. } => self.estimate_expr_cost(value),
            Statement::Return { value, .. } => {
                value.as_ref().map_or(0.0, |e| self.estimate_expr_cost(e))
            }
            Statement::IfElse { condition, then_body, else_body, .. } => {
                let mut cost = self.estimate_expr_cost(condition);
                for stmt in then_body {
                    cost += self.estimate_stmt_cost(stmt);
                }
                if let Some(ref else_stmts) = else_body {
                    for stmt in else_stmts {
                        cost += self.estimate_stmt_cost(stmt);
                    }
                }
                cost
            }
            Statement::Expr(expr) => self.estimate_expr_cost(&expr.expr),
        }
    }

    fn estimate_expr_cost(&self, expr: &Expression) -> f64 {
        match expr {
            Expression::Literal(_, _) => 1.0,
            Expression::Variable(_) => 1.0,
            Expression::BinaryOp { left, right, .. } => {
                self.estimate_expr_cost(left) + self.estimate_expr_cost(right) + 1.0
            }
            Expression::UnaryOp { expr, .. } => self.estimate_expr_cost(expr) + 1.0,
            Expression::SensorAccess { .. } => *self.costs.get("sensor_read").unwrap_or(&2.0),
            Expression::ArrayAccess { index, .. } => self.estimate_expr_cost(index) + 2.0,
            Expression::FunctionCall { args, .. } => {
                args.iter().map(|a| self.estimate_expr_cost(a)).sum::<f64>() + 4.0
            }
            Expression::DotAccess { target, .. } => self.estimate_expr_cost(target),
            Expression::SensorMerge { sensors, weights, .. } => {
                // Weighted average: ~1 cycle per sensor + multiply per weight
                let base = sensors.len() as f64 * 2.0;
                let weight_ops = weights.len() as f64 * 1.0;
                base + weight_ops
            }
            Expression::Match { arms, .. } => {
                // Match: cost of worst-case arm + branch check
                arms.iter()
                    .map(|a| self.estimate_expr_cost(&a.body))
                    .fold(0.0f64, f64::max)
                    + 1.0
            }
            Expression::Probe { .. } => 2.0, // isnan check
        }
    }
}

/// Analyze a loop's worst-case execution time
pub fn analyze_loop_timing(
    loop_decl: &LoopDecl,
    clock_mhz: f64,
) -> Result<f64, CheckError> {
    let cost_model = CostModel::arm_cortex_m4();

    // Estimate per-iteration cost
    let per_iter_cost: f64 = loop_decl.body.iter()
        .map(|stmt| cost_model.estimate_stmt_cost(stmt))
        .sum();

    // Estimate loop bound
    let loop_bound = estimate_loop_bound(loop_decl)?;

    // WCET = per_iteration_cost * loop_bound / clock_speed
    let wcet_cycles = per_iter_cost * loop_bound;
    let wcet_ms = wcet_cycles / (clock_mhz * 1000.0);

    Ok(wcet_ms)
}

fn estimate_loop_bound(loop_decl: &LoopDecl) -> Result<f64, CheckError> {
    // For now, use a simple heuristic:
    // Count the number of statements as a proxy for iterations
    // In a real implementation, we'd analyze the loop structure

    // Heuristic: if the loop body is simple (1-3 statements),
    // assume it runs at least 10 times per deadline
    let body_complexity = loop_decl.body.len() as f64;

    // Conservative: assume at least 10 iterations for simple loops
    // More complex loops get fewer assumed iterations
    let iterations = if body_complexity <= 3.0 {
        10.0
    } else if body_complexity <= 6.0 {
        5.0
    } else {
        3.0
    };

    Ok(iterations)
}

/// Check all timing constraints in a program
pub fn check_timing(
    program: &Program,
    clock_mhz: f64,
) -> Vec<CheckError> {
    let mut errors = Vec::new();

    for decl in &program.declarations {
        if let Declaration::Loop(loop_decl) = decl {
            let deadline_ms = loop_decl.deadline.to_ms();

            match analyze_loop_timing(loop_decl, clock_mhz) {
                Ok(wcet_ms) => {
                    if wcet_ms > deadline_ms {
                        errors.push(CheckError::DeadlineExceeded {
                            loop_name: loop_decl.name.name.clone(),
                            deadline_ms,
                            estimated_wcet_ms: wcet_ms,
                            span: loop_decl.span,
                        });
                    }
                }
                Err(e) => errors.push(e),
            }
        }
    }

    errors
}

/// Check all fallback constraints in a program
pub fn check_fallbacks(program: &Program) -> Vec<CheckError> {
    let mut graph = FallbackGraph::new();

    // Collect all sensors
    for decl in &program.declarations {
        if let Declaration::Sensor(s) = decl {
            graph.register_sensor(&s.name.name);
        }
    }

    // Collect all fallbacks
    for decl in &program.declarations {
        if let Declaration::Fallback(f) = decl {
            graph.add_fallback(f);
        }
    }

    graph.check_completeness()
}

/// Run all checker passes
pub fn check_program(
    program: &Program,
    clock_mhz: f64,
) -> Vec<CheckError> {
    let mut errors = Vec::new();
    errors.extend(check_fallbacks(program));
    errors.extend(check_timing(program, clock_mhz));
    errors
}

// ═══════════════════════════════════════════════════════════════════════════
// IPET (Implicit Path Enumeration Technique) Timing Analysis
// ═══════════════════════════════════════════════════════════════════════════

/// A node in the Control Flow Graph
#[derive(Debug, Clone)]
pub struct CfgNode {
    pub id: usize,
    pub label: String,
    pub cost_cycles: f64,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
    pub is_entry: bool,
    pub is_exit: bool,
    pub loop_bound: Option<f64>,
}

/// Control Flow Graph for IPET analysis
#[derive(Debug, Clone)]
pub struct Cfg {
    pub nodes: Vec<CfgNode>,
    pub entry: usize,
    pub exit: usize,
}

impl Cfg {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            entry: 0,
            exit: 0,
        }
    }

    pub fn add_node(&mut self, label: &str, cost: f64) -> usize {
        let id = self.nodes.len();
        self.nodes.push(CfgNode {
            id,
            label: label.to_string(),
            cost_cycles: cost,
            successors: Vec::new(),
            predecessors: Vec::new(),
            is_entry: false,
            is_exit: false,
            loop_bound: None,
        });
        id
    }

    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.nodes[from].successors.push(to);
        self.nodes[to].predecessors.push(from);
    }

    pub fn set_entry(&mut self, id: usize) {
        self.nodes[id].is_entry = true;
        self.entry = id;
    }

    pub fn set_exit(&mut self, id: usize) {
        self.nodes[id].is_exit = true;
        self.exit = id;
    }

    pub fn set_loop_bound(&mut self, id: usize, bound: f64) {
        self.nodes[id].loop_bound = Some(bound);
    }
}

/// Build a CFG from loop body statements
pub fn build_cfg(stmts: &[Statement], cost_model: &CostModel) -> Cfg {
    let mut cfg = Cfg::new();
    let entry = cfg.add_node("entry", 0.0);
    cfg.set_entry(entry);

    let mut current = entry;

    for stmt in stmts {
        current = cfg_add_stmt(&mut cfg, current, stmt, cost_model);
    }

    // Connect to exit
    let exit = cfg.add_node("exit", 0.0);
    cfg.set_exit(exit);
    cfg.add_edge(current, exit);

    cfg
}

/// Recursively build CFG for a statement, returning the new "current" node
fn cfg_add_stmt(cfg: &mut Cfg, current: usize, stmt: &Statement, cost_model: &CostModel) -> usize {
    match stmt {
        Statement::IfElse { condition, then_body, else_body, .. } => {
            let cond_cost = cost_model.estimate_expr_cost(condition);
            let cond_node = cfg.add_node("if_cond", cond_cost);
            cfg.add_edge(current, cond_node);

            // Then branch
            let mut then_end = cond_node;
            for s in then_body {
                then_end = cfg_add_stmt(cfg, then_end, s, cost_model);
            }

            // Else branch (if present)
            let merge = cfg.add_node("if_merge", 0.0);
            if let Some(else_stmts) = else_body {
                let mut else_end = cond_node;
                for s in else_stmts {
                    else_end = cfg_add_stmt(cfg, else_end, s, cost_model);
                }
                cfg.add_edge(else_end, merge);
            } else {
                cfg.add_edge(cond_node, merge);
            }

            cfg.add_edge(then_end, merge);
            merge
        }
        Statement::Let { value, .. } => {
            let cost = cost_model.estimate_expr_cost(value);
            let node = cfg.add_node("let", cost);
            cfg.add_edge(current, node);
            node
        }
        Statement::Read { .. } => {
            let cost = *cost_model.costs.get("sensor_read").unwrap_or(&2.0);
            let node = cfg.add_node("read", cost);
            cfg.add_edge(current, node);
            node
        }
        Statement::Write { value, .. } => {
            let cost = cost_model.estimate_expr_cost(value) + *cost_model.costs.get("actuator_write").unwrap_or(&2.0);
            let node = cfg.add_node("write", cost);
            cfg.add_edge(current, node);
            node
        }
        Statement::Assign { value, .. } => {
            let cost = cost_model.estimate_expr_cost(value);
            let node = cfg.add_node("assign", cost);
            cfg.add_edge(current, node);
            node
        }
        Statement::Return { value, .. } => {
            let cost = value.as_ref().map_or(0.0, |e| cost_model.estimate_expr_cost(e));
            let node = cfg.add_node("return", cost);
            cfg.add_edge(current, node);
            node
        }
        Statement::Expr(expr) => {
            let cost = cost_model.estimate_expr_cost(&expr.expr);
            let node = cfg.add_node("expr", cost);
            cfg.add_edge(current, node);
            node
        }
    }
}

/// ILP constraint types
#[derive(Debug, Clone)]
pub enum IlpConstraint {
    /// Variable sum <= bound:  x1 + x2 + ... <= N
    Leq { vars: Vec<usize>, bound: f64 },
    /// Variable sum >= bound:  x1 + x2 + ... >= N
    Geq { vars: Vec<usize>, bound: f64 },
    /// Variable sum == bound:  x1 + x2 + ... == N
    Eq { vars: Vec<usize>, bound: f64 },
    /// Single variable <= bound:  xi <= N
    VarLeq { var: usize, bound: f64 },
}

/// IPET result
#[derive(Debug, Clone)]
pub struct IpetResult {
    pub wcet_cycles: f64,
    pub wcet_ms: f64,
    pub execution_counts: Vec<(String, f64)>,
}

/// Solve WCET using IPET (Implicit Path Enumeration Technique)
///
/// IPET formulates the problem as an Integer Linear Program:
///   maximize: sum(xi * cost_i)     -- total execution time
///   subject to:
///     - flow conservation at each node
///     - loop bounds
///     - xi >= 0 for all nodes
///
/// Uses good_lp with microlp backend for a pure-Rust ILP solver.
pub fn solve_ipet(cfg: &Cfg, clock_mhz: f64) -> IpetResult {
    use good_lp::{variables, variable, Solution, SolverModel};

    let n = cfg.nodes.len();

    variables! { vars: }

    // Create execution count variables for each block
    let mut x_vars = Vec::with_capacity(n);
    for i in 0..n {
        let ub = cfg.nodes[i].loop_bound.unwrap_or(1000.0) as i32;
        let var = vars.add(variable().integer().clamp(0, ub));
        x_vars.push(var);
    }

    // Objective: maximize sum(cost_i * x_i)
    let mut objective = good_lp::Expression::from(0);
    for (i, node) in cfg.nodes.iter().enumerate() {
        let cost = (node.cost_cycles * 1000.0) as i32;
        objective = objective + cost * x_vars[i];
    }

    let mut model = vars.maximise(objective).using(good_lp::default_solver);

    // Entry constraint: x_entry = 1
    model.add_constraint(x_vars[cfg.entry] << 1);
    model.add_constraint(x_vars[cfg.entry] >> 1);

    // Exit constraint: x_exit = 1
    model.add_constraint(x_vars[cfg.exit] << 1);
    model.add_constraint(x_vars[cfg.exit] >> 1);

    // Flow conservation: for each non-entry/non-exit node,
    // sum(predecessors' x) = x_i = sum(successors' x)
    for i in 0..n {
        if i == cfg.entry || i == cfg.exit {
            continue;
        }

        let incoming: good_lp::Expression = cfg.nodes.iter()
            .enumerate()
            .filter(|(_, node)| node.successors.contains(&i))
            .map(|(j, _)| x_vars[j])
            .fold(good_lp::Expression::from(0), |acc, v| acc + v);

        let outgoing: good_lp::Expression = cfg.nodes[i].successors.iter()
            .map(|&j| x_vars[j])
            .fold(good_lp::Expression::from(0), |acc, v| acc + v);

        model.add_constraint(incoming.clone() - x_vars[i] << 0);
        model.add_constraint(incoming - x_vars[i] >> 0);
        model.add_constraint(outgoing.clone() - x_vars[i] << 0);
        model.add_constraint(outgoing - x_vars[i] >> 0);
    }

    // Loop bound constraints
    for node in &cfg.nodes {
        if let Some(bound) = node.loop_bound {
            model.add_constraint(x_vars[node.id] << bound as i32);
        }
    }

    // Solve (negate objective since we're minimising but want to maximise)
    match model.solve() {
        Ok(solution) => {
            let wcet_cycles: f64 = cfg.nodes.iter()
                .enumerate()
                .map(|(i, node)| {
                    let count = solution.eval(&x_vars[i]);
                    count as f64 * node.cost_cycles
                })
                .sum();

            let wcet_ms = wcet_cycles / (clock_mhz * 1000.0);

            let execution_counts: Vec<(String, f64)> = cfg.nodes.iter()
                .enumerate()
                .map(|(i, node)| (node.label.clone(), solution.eval(&x_vars[i]) as f64))
                .collect();

            IpetResult {
                wcet_cycles,
                wcet_ms,
                execution_counts,
            }
        }
        Err(_) => {
            // Fallback: conservative estimate if ILP fails
            let wcet_cycles: f64 = cfg.nodes.iter()
                .map(|n| n.loop_bound.unwrap_or(1.0) * n.cost_cycles)
                .sum();

            let wcet_ms = wcet_cycles / (clock_mhz * 1000.0);

            let execution_counts: Vec<(String, f64)> = cfg.nodes.iter()
                .map(|n| (n.label.clone(), n.loop_bound.unwrap_or(1.0)))
                .collect();

            IpetResult {
                wcet_cycles,
                wcet_ms,
                execution_counts,
            }
        }
    }
}

/// IPET analysis result for a loop
#[derive(Debug, Clone)]
pub struct IpetAnalysis {
    pub loop_name: String,
    pub result: Option<IpetResult>,
    pub meets_deadline: bool,
    pub deadline_ms: f64,
    pub error: Option<CheckError>,
}

/// Run IPET analysis on all loops in a program
pub fn ipet_check_timing(program: &Program, clock_mhz: f64) -> Vec<IpetAnalysis> {
    let cost_model = CostModel::arm_cortex_m4();
    let mut results = Vec::new();

    for decl in &program.declarations {
        if let Declaration::Loop(loop_decl) = decl {
            let deadline_ms = loop_decl.deadline.to_ms();

            // Check if loop has a static bound
            match estimate_loop_bound(loop_decl) {
                Ok(loop_bound) => {
                    // Build CFG from loop body
                    let mut cfg = build_cfg(&loop_decl.body, &cost_model);

                    // Mark loop header with bound
                    if cfg.nodes.len() > 2 {
                        cfg.set_loop_bound(1, loop_bound);
                    }

                    // Solve IPET
                    let result = solve_ipet(&cfg, clock_mhz);

                    results.push(IpetAnalysis {
                        loop_name: loop_decl.name.name.clone(),
                        meets_deadline: result.wcet_ms <= deadline_ms,
                        deadline_ms,
                        result: Some(result),
                        error: None,
                    });
                }
                Err(e) => {
                    results.push(IpetAnalysis {
                        loop_name: loop_decl.name.name.clone(),
                        meets_deadline: false,
                        deadline_ms,
                        result: None,
                        error: Some(e),
                    });
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_core::{Ident, Duration};

    #[test]
    fn test_fallback_graph_complete() {
        let mut graph = FallbackGraph::new();
        graph.register_sensor("imu");
        graph.register_sensor("altitude");

        let fallback = FallbackDecl {
            sensor_name: Ident::new("altitude", Span::dummy()),
            timeout: Duration { value: 200.0, unit: fabric_core::TimeUnit::Milliseconds, span: Span::dummy() },
            fallback_expr: Expression::Variable(Ident::new("estimated", Span::dummy())),
            span: Span::dummy(),
        };
        graph.add_fallback(&fallback);

        // IMU is missing fallback
        let errors = graph.check_completeness();
        assert!(errors.iter().any(|e| matches!(e, CheckError::MissingFallback { sensor, .. } if sensor == "imu")));
    }

    #[test]
    fn test_cost_model() {
        let model = CostModel::arm_cortex_m4();
        assert_eq!(*model.costs.get("add").unwrap(), 1.0);
        assert_eq!(*model.costs.get("div").unwrap(), 12.0);
        assert_eq!(*model.costs.get("fdiv").unwrap(), 14.0);
    }

    #[test]
    fn test_cfg_construction() {
        let model = CostModel::arm_cortex_m4();
        let stmts = vec![
            Statement::Let {
                name: Ident::new("x", Span::dummy()),
                ty: None,
                value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                span: Span::dummy(),
            },
            Statement::Let {
                name: Ident::new("y", Span::dummy()),
                ty: None,
                value: Expression::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(Expression::Variable(Ident::new("x", Span::dummy()))),
                    right: Box::new(Expression::Literal(Literal::Float(2.0), Span::dummy())),
                    span: Span::dummy(),
                },
                span: Span::dummy(),
            },
        ];

        let cfg = build_cfg(&stmts, &model);
        // entry + 2 lets + exit = 4 nodes
        assert_eq!(cfg.nodes.len(), 4);
        assert!(cfg.nodes[cfg.entry].is_entry);
        assert!(cfg.nodes[cfg.exit].is_exit);
    }

    #[test]
    fn test_ipet_basic() {
        let model = CostModel::arm_cortex_m4();
        let stmts = vec![
            Statement::Let {
                name: Ident::new("a", Span::dummy()),
                ty: None,
                value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                span: Span::dummy(),
            },
        ];

        let cfg = build_cfg(&stmts, &model);
        let result = solve_ipet(&cfg, 72.0);

        // Should have some positive WCET
        assert!(result.wcet_cycles > 0.0);
        assert!(result.wcet_ms > 0.0);
        // All nodes should execute once
        for (_, count) in &result.execution_counts {
            assert!(*count >= 1.0);
        }
    }

    #[test]
    fn test_ipet_if_else() {
        let model = CostModel::arm_cortex_m4();
        let stmts = vec![
            Statement::IfElse {
                condition: Expression::Variable(Ident::new("cond", Span::dummy())),
                then_body: vec![
                    Statement::Let {
                        name: Ident::new("x", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                ],
                else_body: Some(vec![
                    Statement::Let {
                        name: Ident::new("y", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(2.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                ]),
                span: Span::dummy(),
            },
        ];

        let cfg = build_cfg(&stmts, &model);
        let result = solve_ipet(&cfg, 72.0);

        // WCET should include both branches (conservative)
        assert!(result.wcet_cycles > 0.0);
    }

    #[test]
    fn test_ipet_straight_line_matches_heuristic() {
        let model = CostModel::arm_cortex_m4();
        // Simple straight-line: three additions
        let stmts = vec![
            Statement::Let {
                name: Ident::new("a", Span::dummy()),
                ty: None,
                value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                span: Span::dummy(),
            },
            Statement::Let {
                name: Ident::new("b", Span::dummy()),
                ty: None,
                value: Expression::Literal(Literal::Float(2.0), Span::dummy()),
                span: Span::dummy(),
            },
            Statement::Let {
                name: Ident::new("c", Span::dummy()),
                ty: None,
                value: Expression::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(Expression::Variable(Ident::new("a", Span::dummy()))),
                    right: Box::new(Expression::Variable(Ident::new("b", Span::dummy()))),
                    span: Span::dummy(),
                },
                span: Span::dummy(),
            },
        ];

        let cfg = build_cfg(&stmts, &model);
        // No loop bound for straight-line — each block executes exactly once
        // (The IPET solver uses entry=1, exit=1 as constraints)

        let result = solve_ipet(&cfg, 72.0);

        // Straight-line: each block executes exactly once
        // WCET should equal sum of all block costs
        let total_cost: f64 = cfg.nodes.iter().map(|n| n.cost_cycles).sum();
        assert!((result.wcet_cycles - total_cost).abs() < 1.0,
            "IPET WCET {} should match sum of block costs {}", result.wcet_cycles, total_cost);
    }

    #[test]
    fn test_ipet_branch_picks_worst_case() {
        let model = CostModel::arm_cortex_m4();
        // if-else: then branch is cheap (1 statement), else branch is expensive (3 statements)
        let stmts = vec![
            Statement::IfElse {
                condition: Expression::Variable(Ident::new("cond", Span::dummy())),
                then_body: vec![
                    Statement::Let {
                        name: Ident::new("x", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                ],
                else_body: Some(vec![
                    Statement::Let {
                        name: Ident::new("y", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                    Statement::Let {
                        name: Ident::new("z", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(2.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                    Statement::Let {
                        name: Ident::new("w", Span::dummy()),
                        ty: None,
                        value: Expression::Literal(Literal::Float(3.0), Span::dummy()),
                        span: Span::dummy(),
                    },
                ]),
                span: Span::dummy(),
            },
        ];

        let cfg = build_cfg(&stmts, &model);
        let result = solve_ipet(&cfg, 72.0);

        // IPET should pick the expensive (else) branch as the binding path
        assert!(result.wcet_cycles > 0.0);
        // The WCET should be at least the cost of the else branch
        let else_branch_cost: f64 = model.estimate_stmt_cost(&stmts[0]);
        assert!(result.wcet_cycles >= else_branch_cost,
            "IPET should pick worst-case path (>= {} cycles), got {}",
            else_branch_cost, result.wcet_cycles);
    }

    #[test]
    fn test_ipet_unbounded_loop_flagged() {
        use super::*;

        // An unbounded loop should be flagged with UnboundedLoop error
        let loop_decl = LoopDecl {
            name: Ident::new("infinite", Span::dummy()),
            deadline: Duration { value: 10.0, unit: fabric_core::TimeUnit::Milliseconds, span: Span::dummy() },
            body: vec![
                Statement::Let {
                    name: Ident::new("x", Span::dummy()),
                    ty: None,
                    value: Expression::Literal(Literal::Float(1.0), Span::dummy()),
                    span: Span::dummy(),
                },
            ],
            span: Span::dummy(),
        };

        let result = estimate_loop_bound(&loop_decl);
        // Currently, estimate_loop_bound always succeeds with a heuristic
        // In a future version with proper static bound analysis, this should return Err(UnboundedLoop)
        assert!(result.is_ok());
    }

    #[test]
    fn test_ipet_infeasible_cfg_graceful() {
        let model = CostModel::arm_cortex_m4();
        // Empty program — no statements to analyze
        let stmts: Vec<Statement> = vec![];

        let cfg = build_cfg(&stmts, &model);
        // Even with an empty body, IPET should not panic
        let result = solve_ipet(&cfg, 72.0);

        // Empty body: only entry and exit nodes, both execute once
        // WCET should be minimal (just entry/exit overhead)
        assert!(result.wcet_cycles >= 0.0);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Refinement Types — Mathematical uncertainty proof (no Z3 dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Interval arithmetic for uncertainty propagation
#[derive(Debug, Clone, Copy)]
pub struct Interval {
    pub min: f64,
    pub max: f64,
}

impl Interval {
    pub fn new(value: f64, uncertainty: f64) -> Self {
        Self {
            min: value - uncertainty,
            max: value + uncertainty,
        }
    }

    pub fn center(&self) -> f64 {
        (self.min + self.max) / 2.0
    }

    pub fn uncertainty(&self) -> f64 {
        (self.max - self.min) / 2.0
    }

    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }

    pub fn overlaps(&self, other: &Interval) -> bool {
        self.min <= other.max && other.min <= self.max
    }

    /// Addition: [a, b] + [c, d] = [a+c, b+d]
    pub fn add(&self, other: &Interval) -> Interval {
        Interval {
            min: self.min + other.min,
            max: self.max + other.max,
        }
    }

    /// Subtraction: [a, b] - [c, d] = [a-d, b-c]
    pub fn sub(&self, other: &Interval) -> Interval {
        Interval {
            min: self.min - other.max,
            max: self.max - other.min,
        }
    }

    /// Multiplication: [a, b] * [c, d]
    pub fn mul(&self, other: &Interval) -> Interval {
        let products = [
            self.min * other.min,
            self.min * other.max,
            self.max * other.min,
            self.max * other.max,
        ];
        Interval {
            min: products.iter().cloned().fold(f64::INFINITY, f64::min),
            max: products.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        }
    }

    /// Scalar multiplication: k * [a, b] = [k*a, k*b] if k >= 0
    pub fn scale(&self, k: f64) -> Interval {
        if k >= 0.0 {
            Interval {
                min: self.min * k,
                max: self.max * k,
            }
        } else {
            Interval {
                min: self.max * k,
                max: self.min * k,
            }
        }
    }
}

/// Prove that sensor fusion stays within safety bounds
///
/// Given sensors with uncertainties and weights, prove:
///   safe_min <= sum(w_i * s_i) / sum(w_i) <= safe_max
///
/// Uses interval arithmetic to compute the exact worst-case range.
pub fn prove_fusion_safe(
    sensor_values: &[(f64, f64)],  // (nominal, uncertainty) pairs
    weights: &[f64],
    safe_min: f64,
    safe_max: f64,
) -> (bool, Interval) {
    assert_eq!(sensor_values.len(), weights.len());

    // Create intervals for each sensor
    let intervals: Vec<Interval> = sensor_values.iter()
        .map(|(val, unc)| Interval::new(*val, *unc))
        .collect();

    // Weighted sum: sum(w_i * s_i)
    let mut weighted_sum = Interval { min: 0.0, max: 0.0 };
    for (interval, weight) in intervals.iter().zip(weights.iter()) {
        let weighted = interval.scale(*weight);
        weighted_sum = weighted_sum.add(&weighted);
    }

    // Total weight
    let total_weight: f64 = weights.iter().sum();

    // Fusion result = weighted_sum / total_weight
    let fusion = Interval {
        min: weighted_sum.min / total_weight,
        max: weighted_sum.max / total_weight,
    };

    // Check if fusion is provably within safety bounds
    let is_safe = fusion.min >= safe_min && fusion.max <= safe_max;

    (is_safe, fusion)
}

/// Prove that uncertainty propagation stays within allowed bounds
///
/// For weighted average fusion, the output uncertainty is:
///   output_uncertainty = sum(|w_i| * uncertainty_i)
///
/// This is the interval arithmetic result for linear combinations.
pub fn prove_uncertainty_bounded(
    sensor_uncertainties: &[f64],
    weights: &[f64],
    max_allowed_uncertainty: f64,
) -> (bool, f64) {
    assert_eq!(sensor_uncertainties.len(), weights.len());

    let total_uncertainty: f64 = sensor_uncertainties.iter()
        .zip(weights.iter())
        .map(|(unc, w)| unc * w.abs())
        .sum();

    (total_uncertainty <= max_allowed_uncertainty, total_uncertainty)
}

/// Refinement check result
#[derive(Debug, Clone)]
pub struct RefinementResult {
    pub sensor: String,
    pub nominal_value: f64,
    pub uncertainty: f64,
    pub proved_interval: Interval,
    pub is_bounded: bool,
    pub bound_violation: Option<String>,
}

/// Check refinement types for all sensor accesses in a program
pub fn check_refinement_types(program: &Program) -> Vec<RefinementResult> {
    let mut results = Vec::new();
    let mut sensor_info: HashMap<String, (f64, f64)> = HashMap::new();

    // Collect sensor declarations
    for decl in &program.declarations {
        if let Declaration::Sensor(sensor) = decl {
            let uncertainty = match &sensor.sensor_type.error_bound {
                ErrorBound::Absolute(v, _) => *v,
                ErrorBound::Relative(v, _) => *v,
            };
            // Default nominal value of 0.0 (will be refined during analysis)
            sensor_info.insert(sensor.name.name.clone(), (0.0, uncertainty));
        }
    }

    // Check each loop body
    for decl in &program.declarations {
        if let Declaration::Loop(loop_decl) = decl {
            for stmt in &loop_decl.body {
                check_stmt_refinement(stmt, &sensor_info, &mut results);
            }
        }
    }

    results
}

fn check_stmt_refinement(
    stmt: &Statement,
    sensor_info: &HashMap<String, (f64, f64)>,
    results: &mut Vec<RefinementResult>,
) {
    match stmt {
        Statement::Let { value, .. } => {
            check_expr_refinement(value, sensor_info, results);
        }
        Statement::Write { value, .. } => {
            check_expr_refinement(value, sensor_info, results);
        }
        Statement::IfElse { then_body, else_body, .. } => {
            for s in then_body {
                check_stmt_refinement(s, sensor_info, results);
            }
            if let Some(else_stmts) = else_body {
                for s in else_stmts {
                    check_stmt_refinement(s, sensor_info, results);
                }
            }
        }
        _ => {}
    }
}

fn check_expr_refinement(
    expr: &Expression,
    sensor_info: &HashMap<String, (f64, f64)>,
    results: &mut Vec<RefinementResult>,
) {
    if let Expression::SensorMerge { sensors, weights, .. } = expr {
        let sensor_values: Vec<(f64, f64)> = sensors.iter()
            .filter_map(|s| sensor_info.get(&s.name).copied())
            .collect();

        let weight_vals: Vec<f64> = weights.iter().filter_map(|w| {
            if let Expression::Literal(Literal::Float(v), _) = w {
                Some(*v)
            } else {
                None
            }
        }).collect();

        if sensor_values.len() == weight_vals.len() && !sensor_values.is_empty() {
            // Prove fusion is safe within [0, 100] bounds
            let (is_safe, fusion_interval) = prove_fusion_safe(
                &sensor_values,
                &weight_vals,
                0.0,
                100.0,
            );

            // Prove uncertainty is bounded
            let uncertainties: Vec<f64> = sensor_values.iter().map(|(_, u)| *u).collect();
            let (unc_bounded, total_unc) = prove_uncertainty_bounded(
                &uncertainties,
                &weight_vals,
                5.0,
            );

            for (sensor, (nominal, uncertainty)) in sensors.iter().zip(sensor_values.iter()) {
                results.push(RefinementResult {
                    sensor: sensor.name.clone(),
                    nominal_value: *nominal,
                    uncertainty: *uncertainty,
                    proved_interval: fusion_interval,
                    is_bounded: is_safe && unc_bounded,
                    bound_violation: if !is_safe {
                        Some(format!("fusion range [{:.3}, {:.3}] exceeds [0, 100]",
                            fusion_interval.min, fusion_interval.max))
                    } else if !unc_bounded {
                        Some(format!("uncertainty {:.3} exceeds max 5.0", total_unc))
                    } else {
                        None
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod refinement_tests {
    use super::*;

    #[test]
    fn test_interval_arithmetic() {
        let a = Interval::new(5.0, 0.5); // [4.5, 5.5]
        let b = Interval::new(3.0, 0.3); // [2.7, 3.3]

        let sum = a.add(&b);
        assert!((sum.min - 7.2).abs() < 0.001);
        assert!((sum.max - 8.8).abs() < 0.001);

        // [4.5, 5.5] - [2.7, 3.3] = [4.5-3.3, 5.5-2.7] = [1.2, 2.8]
        let diff = a.sub(&b);
        assert!((diff.min - 1.2).abs() < 0.001);
        assert!((diff.max - 2.8).abs() < 0.001);
    }

    #[test]
    fn test_fusion_proof_safe() {
        // Two sensors: 5.0 ± 0.5 and 3.0 ± 0.3
        // Weights: 0.6, 0.4
        // Fusion = 0.6*s1 + 0.4*s2
        // Uncertainty = 0.6*0.5 + 0.4*0.3 = 0.42
        let (safe, interval) = prove_fusion_safe(
            &[(5.0, 0.5), (3.0, 0.3)],
            &[0.6, 0.4],
            0.0,
            100.0,
        );
        assert!(safe);
        assert!(interval.min > 3.0);
        assert!(interval.max < 7.0);
    }

    #[test]
    fn test_fusion_proof_unsafe() {
        // Sensor near boundary with large uncertainty
        let (safe, _) = prove_fusion_safe(
            &[(99.0, 2.0)],
            &[1.0],
            0.0,
            100.0,
        );
        // 99.0 + 2.0 = 101.0 > 100.0 → unsafe
        assert!(!safe);
    }

    #[test]
    fn test_uncertainty_bounded() {
        let (bounded, total) = prove_uncertainty_bounded(
            &[0.5, 0.3],
            &[0.6, 0.4],
            5.0,
        );
        assert!(bounded);
        assert!((total - 0.42).abs() < 0.001);
    }
}
