<p align="center">
  <img src="https://img.shields.io/badge/Rust-2021-orange?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/tests-38%20passing-brightgreen?style=for-the-badge" alt="Tests">
  <img src="https://img.shields.io/badge/crates-8-blueviolet?style=for-the-badge" alt="Crates">
  <img src="https://img.shields.io/badge/targets-Python%20%7C%20C-informational?style=for-the-badge" alt="Targets">
  <img src="https://img.shields.io/badge/license-MIT-yellow?style=for-the-badge" alt="License">
</p>

<h1 align="center">Fabric</h1>

<p align="center">
  <b>A compiled language for robots and drones that catches your mistakes before they crash your hardware.</b>
</p>

<p align="center">
  Real-time deadlines, sensor uncertainty, and failure handling as compiler-checked language features — not runtime hope.
</p>

---

## Why does this exist

I got tired of reading robotics code where the stabilization loop *probably* runs fast enough, the sensor readings *probably* aren't too noisy, and the fallback logic *probably* covers every failure case. "Probably" doesn't cut it when there's a drone in the air.

Most robotics software is written in C++ or Python on top of ROS. The language doesn't know or care that your IMU drops out sometimes, that your loop needs to finish in 2ms or the quadcopter flips, or that you forgot to handle the case where both GPS and barometer fail at the same time. You find out about these things at 3am in the field when something hits the ground.

Fabric is a small compiled language where these aren't runtime bugs. They're compile-time errors.

---

## What it actually does

```
sensor imu: Sensor<f32, ±0.1>
sensor altitude: Sensor<f32, ±0.5>

actuator motors: Motor[4]

when imu unavailable for 100ms {
    fallback to 0.0
}

loop control within 2ms {
    let fused: f32 = merge imu altitude [0.6, 0.4]
    write motors[0] = fused
}
```

That code declares two noisy sensors with explicit error bounds, a fallback path for when the IMU dies, a control loop with a 2ms deadline, and sensor fusion with weights. The compiler checks all of it.

**If you write this:**

```
loop stabilize within 2ms {
    let val: f32 = add(1.0)    // wrong: add expects 2 args
}
```

The compiler tells you: `WrongArity: expected 2 arguments, found 1`. At compile time. Not when the drone is flying.

**If you forget a fallback:**

```
sensor gps: Sensor<f32, ±1.0>
// no "when gps unavailable" handler
loop control within 2ms {
    let pos: f32 = read gps    // uses gps with no fallback
}
```

The compiler refuses to build: `MissingFallback: sensor 'gps' has no fallback path`. Same way Rust makes you handle every enum variant.

**If your loop is too slow:**

```
loop control within 0.5ms {
    let a: f32 = read imu
    let b: f32 = merge imu altitude [0.7, 0.3]
    let c: f32 = complex_math(b)
    // ... lots more code ...
}
```

The timing analyzer estimates worst-case execution time and tells you: `DeadlineExceeded: loop 'control' estimated 1.2ms, deadline is 0.5ms`.

---

## Language features

### Sensor types with uncertainty bounds

```
sensor imu: Sensor<f32, ±0.1>
sensor lidar: Sensor<f32, ±0.02>
```

The error bound travels with the type. When you combine two sensors, the compiler tracks the combined uncertainty automatically.

### Sensor fusion

```
let fused: f32 = merge imu altitude [0.6, 0.4]
```

Weighted average with combined uncertainty. `merge` takes at least two sensors and optional weights. The compiler computes: `±(0.1 * 0.6 + 0.5 * 0.4) = ±0.26`.

### Deadline-checked loops

```
loop stabilize within 2ms {
    // body must finish within 2ms
}
```

The timing analyzer estimates worst-case execution time against ARM Cortex-M4 instruction costs. If the loop can't finish in time, it's a compile error.

### Mandatory fallback paths

```
when imu unavailable for 100ms {
    fallback to dead_reckoning(0.0)
}
```

Every sensor dependency must have an explicit fallback. The compiler builds a dependency graph and checks coverage. Missing a path = compile error. Circular fallbacks = compile error.

### State matching

```
let mode: f32 = match gps {
    ok => 1.0,
    timeout => 0.5,
    error => 0.0
}
```

Branch on sensor health explicitly. Forces you to handle every state.

### Sensor probing

```
let alive: bool = probe imu
if alive {
    let val: f32 = read imu
}
```

Boolean check on sensor availability for runtime branching.

### Functions

```
fn dead_reckoning(imu_val: f32) -> f32 {
    return imu_val * 0.95
}
```

Parameter types, return types, arity checking, recursive calls. Works.

---

## Compiler pipeline

```
.fab source
  -> Lexer          (logos, 50+ tokens, handles UTF-8 symbols)
  -> Parser         (hand-rolled Pratt parser, 772 lines)
  -> Type Checker   (uncertainty propagation, arity checks)
  -> Fallback Check (BFS graph analysis, cycle detection)
  -> Timing Check   (WCET estimation, ARM Cortex-M4 costs)
  -> Code Generator
       -> Python    (Webots robotics controller)
       -> C         (ARM Cortex-M / Raspberry Pi)
```

8 Rust crates. 3,507 lines of Rust. 38 tests.

---

## Project structure

```
fabric/
  crates/
    fabric-core/      146 lines   Shared types, spans, error handling
    fabric-ast/       327 lines   AST definitions
    fabric-lexer/     284 lines   Tokenizer (logos)
    fabric-parser/    772 lines   Pratt parser
    fabric-types/     447 lines   Type system, uncertainty tracking
    fabric-checker/   375 lines   Fallback graph, timing analysis
    fabric-codegen/   606 lines   Python + C code generation
    fabric-cli/       550 lines   CLI binary (check, build, ast, timing)
  examples/
    drone.fab         3 sensors, merge, match, probe, functions, fallbacks
    stabilize.fab     PID controller with sensor fusion
  tools/
    fabric-run.py     Compiles .fab and sets up Webots project
  webots/
    worlds/           Webots world files
```

---

## Generated output

The same `.fab` file compiles to both Python (for Webots simulation) and C (for ARM hardware).

**Python output (drone.py):**

```python
class FabricController(Robot):
    def run(self):
        while self.step(self.timeStep) != -1:
            imu_raw = self.imu.getValue()
            altitude_raw = self.altitude.getValue()

            # Fallback checks
            if imu_raw is not None:
                imu_last_known = imu_raw
            elif self.getTime() - imu_timeout_start > imu_TIMEOUT:
                imu_fallback_active = True

            # Sensor fusion
            fused = ((imu * 0.60) + (altitude * 0.40))

            # State-dependent control
            mode = (1.00 if gps is not None else (0.50 if timeout_gps else 0.00))

            correction = (err * 0.50) * mode
            self.motors[0].setVelocity(correction)
```

**C output (drone.c):**

```c
int main(void) {
    init_sensors();
    init_actuators();
    while (1) {
        uint32_t loop_start = hal_get_time_ms();
        float imu_raw = sensor_read(imu_handle);

        float fused = ((imu * 0.60) + (altitude * 0.40));
        float mode = (!isnan(gps) ? 1.00 : (timeout_gps ? 0.50 : 0.00));
        float correction = (err * 0.50) * mode;
        motor_set_position(motors_handles[0], correction);

        // Enforce timing deadline
        uint32_t elapsed = hal_get_time_ms() - loop_start;
        if (elapsed < (uint32_t)2) {
            hal_sleep_ms((uint32_t)2 - elapsed);
        }
    }
}
```

---

## Getting started

### Prerequisites

- Rust toolchain (1.75+)
- Python 3.10+ (for Webots runner, optional)

### Quick test

```powershell
.\try.ps1
```

This builds the compiler, compiles `drone.fab` to Python and C, runs timing analysis, and runs all 38 tests. Takes about 20 seconds on first run.

### Build

```bash
git clone https://github.com/yourusername/fabric.git
cd fabric
cargo build --release
```

### Run tests

```bash
cargo test
# 38 tests, all passing
```

### Compile a .fab file

```bash
# Check for errors
./target/release/fabric check --file examples/drone.fab

# Generate Python
./target/release/fabric build --target python --file examples/drone.fab --output drone.py

# Generate C
./target/release/fabric build --target c --file examples/drone.fab --output drone.c

# Dump AST
./target/release/fabric ast --file examples/drone.fab

# Check timing
./target/release/fabric timing --file examples/drone.fab --clock-mhz 72.0
```

### Using the Webots runner (optional)

```bash
python tools/fabric-run.py examples/drone.fab --launch
```

Requires Webots R2023b+ installed separately.

---

## Proving it works

The test suite covers every stage of the compiler:

| What's tested | How many |
|---|---|
| Lexer tokenization | 8 tests |
| Parser correctness | 5 tests |
| Type system | 3 tests |
| Checker logic | 2 tests |
| Codegen output | 2 tests |
| End-to-end integration | 18 tests |
| **Total** | **38 tests** |

Integration tests parse real `.fab` code, type-check it, run the fallback and timing analyzers, generate Python and C, and assert the output contains correct code.

Example from the test suite:

```rust
#[test]
fn test_merge_expression() {
    let source = r#"
        sensor imu: Sensor<f32, ±0.1>
        sensor altitude: Sensor<f32, ±0.5>
        // ...
        let fused: f32 = merge imu altitude [0.6, 0.4]
    "#;
    let prog = parse_source(source);
    let errors = check_program(&prog, 72.0);
    assert!(errors.is_empty());

    let py = PythonEmitter.emit_program(&prog);
    assert!(py.contains("imu * 0.6"));
    assert!(py.contains("altitude * 0.4"));
}
```

---

## What this doesn't do (yet)

- **Z3 refinement types** -- would prove uncertainty bounds mathematically instead of tracking them heuristically
- **IPET timing solver** -- would give proven worst-case bounds instead of estimated
- **Multi-drone coordination** -- single robot only for now
- **Real hardware validation** -- tested against generated code, not physical motors

---

## Architecture decisions

**Why hand-rolled parser instead of chumsky/pest?**

Chumsky's API changed significantly between versions and the documentation was sparse for the patterns I needed. A Pratt parser for expressions + recursive descent for statements was about 700 lines and handles everything cleanly. No dependencies to fight.

**Why not LLVM?**

Overkill for a DSL. The C backend generates plain C that any GCC/Clang can compile. The Python backend generates valid Webots controllers. No IR needed.

**Why logos for lexing?**

Logos generates a DFA-based tokenizer from attribute macros. Zero runtime overhead, handles Unicode correctly, and the callback system works well for duration literals and error bounds.

**Why Rust?**

The type system in Rust mirrors what I'm building in Fabric. Pattern matching for the AST, enums for error types, traits for the codegen interface. Plus `cargo test` is excellent.

---

## License

MIT

---

## Built by

A 17-year-old who got tired of drone code that works until it doesn't.
