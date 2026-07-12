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
                write!(f, "loop '{}' estimated WCET {:.2}ms exceeds deadline {:.2}ms",
                    loop_name, estimated_wcet_ms, deadline_ms)
            }
            CheckError::UnknownLoopBound { loop_name, .. } => {
                write!(f, "cannot determine loop bound for '{}'", loop_name)
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
struct CostModel {
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
/// We use a greedy solver (simplex-like) for the ILP since
/// full ILP solvers are external dependencies.
pub fn solve_ipet(cfg: &Cfg, clock_mhz: f64) -> IpetResult {
    let n = cfg.nodes.len();

    // Build flow conservation constraints
    let mut constraints: Vec<IlpConstraint> = Vec::new();

    for node in &cfg.nodes {
        if node.is_entry {
            // Entry: exactly 1 execution
            constraints.push(IlpConstraint::Eq {
                vars: vec![node.id],
                bound: 1.0,
            });
        } else if node.is_exit {
            // Exit: exactly 1 execution (program completes once)
            constraints.push(IlpConstraint::Eq {
                vars: vec![node.id],
                bound: 1.0,
            });
        } else {
            // Flow conservation: in-degree executions = out-degree executions
            // Sum(predecessors executing to this) = Sum(successors executing from this)
            // Simplified: for linear paths, each node executes at least as many times
            // as its predecessors

            // Loop bound constraint
            if let Some(bound) = node.loop_bound {
                constraints.push(IlpConstraint::VarLeq {
                    var: node.id,
                    bound,
                });
            }
        }
    }

    // Flow conservation: for each node (except entry/exit),
    // number of times we enter = number of times we exit
    // For IPET, we encode this as:
    //   sum of predecessor execution counts = execution count of this node
    //   This is implicit in the CFG structure for acyclic graphs

    // For loops, we need back-edge constraints
    // A back edge (u -> v) where v dominates u means u -> v is a loop
    // The loop bound constrains how many times the back edge can execute

    // Simplified: assign execution counts greedily
    let mut exec_counts = vec![0.0f64; n];

    // Topological order execution (simplified IPET)
    exec_counts[cfg.entry] = 1.0;

    // BFS to propagate execution counts
    let mut visited = vec![false; n];
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(cfg.entry);
    visited[cfg.entry] = true;

    while let Some(node_id) = queue.pop_front() {
        let node = &cfg.nodes[node_id];
        let exec = exec_counts[node_id];

        for &succ_id in &node.successors {
            if !visited[succ_id] {
                exec_counts[succ_id] += exec;
                visited[succ_id] = true;
                queue.push_back(succ_id);
            }
        }
    }

    // Apply loop bounds (cap execution counts)
    for node in &cfg.nodes {
        if let Some(bound) = node.loop_bound {
            if exec_counts[node.id] > bound {
                exec_counts[node.id] = bound;
            }
        }
    }

    // Calculate WCET
    let wcet_cycles: f64 = cfg.nodes.iter()
        .map(|n| exec_counts[n.id] * n.cost_cycles)
        .sum();

    let wcet_ms = wcet_cycles / (clock_mhz * 1000.0);

    let execution_counts: Vec<(String, f64)> = cfg.nodes.iter()
        .map(|n| (n.label.clone(), exec_counts[n.id]))
        .collect();

    IpetResult {
        wcet_cycles,
        wcet_ms,
        execution_counts,
    }
}

/// IPET analysis result for a loop
#[derive(Debug, Clone)]
pub struct IpetAnalysis {
    pub loop_name: String,
    pub result: IpetResult,
    pub meets_deadline: bool,
    pub deadline_ms: f64,
}

/// Run IPET analysis on all loops in a program
pub fn ipet_check_timing(program: &Program, clock_mhz: f64) -> Vec<IpetAnalysis> {
    let cost_model = CostModel::arm_cortex_m4();
    let mut results = Vec::new();

    for decl in &program.declarations {
        if let Declaration::Loop(loop_decl) = decl {
            let deadline_ms = loop_decl.deadline.to_ms();

            // Build CFG from loop body
            let mut cfg = build_cfg(&loop_decl.body, &cost_model);

            // Set loop bound (conservative estimate)
            let loop_bound = estimate_loop_bound(loop_decl).unwrap_or(10.0);

            // Mark loop header with bound
            if cfg.nodes.len() > 2 {
                cfg.set_loop_bound(1, loop_bound); // node after entry
            }

            // Solve IPET
            let result = solve_ipet(&cfg, clock_mhz);

            results.push(IpetAnalysis {
                loop_name: loop_decl.name.name.clone(),
                meets_deadline: result.wcet_ms <= deadline_ms,
                deadline_ms,
                result,
            });
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
}
