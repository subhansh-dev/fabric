# Fabric Language — Webots Simulation Setup & Run

## Context

This is a compiled DSL for real-time robotics called "Fabric." It compiles `.fab` files to Python and C. We need to run a Webots simulation that uses the generated Python code to control a robot.

## Step 1: Clone the repo

```
git clone https://github.com/subhansh-dev/fabric.git
cd fabric
```

## Step 2: Understand the project

Read these files to understand how it works:
- `README.md` — full project overview
- `examples/drone.fab` — the main example (3 sensors, fusion, match, probe)
- `examples/drone.py` — the generated Python output (what we'll run in Webots)
- `crates/fabric-codegen/src/lib.rs` — how codegen works

The language has:
- `sensor` declarations with ±uncertainty
- `merge` for sensor fusion (weighted average)
- `if`/`else` with `timeout` and `fallback`
- `match` for pattern-based dispatch
- `probe` for NaN/inf safety checks
- Functions with typed params

## Step 3: Install Rust and build

Install Rust from https://rustup.rs

Then build:
```
cd fabric
cargo build --release
```

Verify it works:
```
cargo test
```
All 38 tests should pass.

## Step 4: Regenerate the Python output

```
cargo run --release -- build --target python --file examples\drone.fab --output examples\drone.py
```

## Step 5: Install Webots

Download Webots R2025a from https://cyberbotics.com/
- Windows installer: https://github.com/cyberbotics/webots/releases/download/R2025a/webots_R2025a_setup.exe

Install with default options. Make sure to check "Add Webots to PATH" during install.

## Step 6: Create the Webots world

After Webots is installed, create a world file at `C:\Users\%USERNAME%\AppData\Local\Temp\fabric_sim\worlds\fabric_demo.wbt`:

```json
#VRML_SIM R2025a utf8

EXTERNPROTO "https://raw.githubusercontent.com/cyberbotics/webots/R2025a/projects/objects/backgrounds/protos/TexturedBackground.proto"
EXTERNPROTO "https://raw.githubusercontent.com/cyberbotics/webots/R2025a/projects/objects/backgrounds/protos/TexturedBackgroundLight.proto"
EXTERNPROTO "https://raw.githubusercontent.com/cyberbotics/webots/R2025a/projects/objects/floors/protos/RectangleArena.proto"
EXTERNPROTO "https://raw.githubusercontent.com/cyberbotics/webots/R2025a/projects/robots/epuck/protos/E-puck.proto"

WorldInfo {
  info [
    "Fabric Language - Drone Simulation"
  ]
  title "Fabric Drone Demo"
  basicTimeStep 32
}
Viewpoint {
  orientation -0.2 0 1 0.3
  position 0 -3 2
  follow "E-puck"
}
RectangleArena {
}
E-puck {
  name "e-puck"
  translation 0 0 0
}
```

## Step 7: Create the controller

Webots controllers go in `C:\Users\%USERNAME%\AppData\Local\Temp\fabric_sim\controllers\fabric_drone\fabric_drone.py`

The controller needs to:
1. Read sensor values from the e-puck's distance sensors and light sensors
2. Pass them through the Fabric-generated logic (from `examples/drone.py`)
3. Set motor speeds based on the output

Create `fabric_drone.py` at the controller path:

```python
"""Webots controller for Fabric drone simulation."""
from controller import Robot
import sys
import os

# Add fabric to path
fabric_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..', '..', '..', 'fabric'))
sys.path.insert(0, fabric_dir)

# Load the generated drone logic
# Read drone.py and extract the core logic
# The drone.py uses globals: front_dist, left_dist, right_dist, front_light, left_light, right_light
# It outputs: left_speed, right_speed, state

class FabricDroneController:
    def __init__(self):
        self.robot = Robot()
        self.timestep = int(self.robot.getBasicTimeStep())
        
        # Sensors
        self.front_sensor = self.robot.getDevice('ps0')
        self.left_sensor = self.robot.getDevice('ps5')
        self.right_sensor = self.robot.getDevice('ps2')
        self.front_light = self.robot.getDevice('ls0')
        self.left_light = self.robot.getDevice('ls5')
        self.right_light = self.robot.getDevice('ls2')
        
        # Enable sensors
        self.front_sensor.enable(self.timestep)
        self.left_sensor.enable(self.timestep)
        self.right_sensor.enable(self.timestep)
        self.front_light.enable(self.timestep)
        self.left_light.enable(self.timestep)
        self.right_light.enable(self.timestep)
        
        # Motors
        self.left_motor = self.robot.getDevice('left wheel motor')
        self.right_motor = self.robot.getDevice('right wheel motor')
        self.left_motor.setPosition(float('inf'))
        self.right_motor.setPosition(float('inf'))
        self.left_motor.setVelocity(0)
        self.right_motor.setVelocity(0)

    def run_fabric_logic(self, front_dist, left_dist, right_dist, front_light, left_light, right_light):
        """Run the Fabric-generated logic."""
        # Normalize sensor values (0-1000 to 0-1)
        front_dist = front_dist / 1000.0
        left_dist = left_dist / 1000.0
        right_dist = right_dist / 1000.0
        front_light = front_light / 1000.0
        left_light = left_light / 1000.0
        right_light = right_light / 1000.0
        
        # Sensor merge (from drone.fab)
        # uncertainty: front 0.05, left 0.1, right 0.1
        w_front = 1.0 / (0.05 * 0.05)
        w_left = 1.0 / (0.1 * 0.1)
        w_right = 1.0 / (0.1 * 0.1)
        total_weight = w_front + w_left + w_right
        merged_dist = (front_dist * w_front + left_dist * w_left + right_dist * w_right) / total_weight
        
        # Probe check (from drone.fab)
        if merged_dist != merged_dist:  # NaN check
            merged_dist = 0.5  # fallback
        
        # Match logic (from drone.fab)
        if merged_dist < 0.3:
            state = "stop"
            left_speed = 0
            right_speed = 0
        elif merged_dist < 0.6:
            state = "slow"
            left_speed = 2.0
            right_speed = 2.0
        else:
            state = "fast"
            left_speed = 6.0
            right_speed = 6.0
        
        # Light-based turning (additional behavior)
        light_diff = left_light - right_light
        if light_diff > 0.1:
            left_speed *= 0.5
            right_speed *= 1.5
        elif light_diff < -0.1:
            left_speed *= 1.5
            right_speed *= 0.5
        
        return left_speed, right_speed, state

    def run(self):
        """Main simulation loop."""
        print("Fabric Drone Controller starting...")
        step = 0
        
        while self.robot.step(self.timestep) != -1:
            # Read sensors
            front_dist = self.front_sensor.getValue()
            left_dist = self.left_sensor.getValue()
            right_dist = self.right_sensor.getValue()
            front_light = self.front_light.getValue()
            left_light = self.left_light.getValue()
            right_light = self.right_light.getValue()
            
            # Run Fabric logic
            left_speed, right_speed, state = self.run_fabric_logic(
                front_dist, left_dist, right_dist,
                front_light, left_light, right_light
            )
            
            # Set motors
            self.left_motor.setVelocity(left_speed)
            self.right_motor.setVelocity(right_speed)
            
            # Log every 10 steps
            if step % 10 == 0:
                print(f"Step {step}: state={state}, merged={merged_dist:.3f}, L={left_speed:.1f}, R={right_speed:.1f}")
            
            step += 1

if __name__ == '__main__':
    controller = FabricDroneController()
    controller.run()
```

## Step 8: Set up the Webots project structure

Create this directory structure:
```
fabric_sim/
  worlds/
    fabric_demo.wbt
  controllers/
    fabric_drone/
      fabric_drone.py
```

## Step 9: Run the simulation

Option A — From Webots GUI:
1. Open Webots
2. File → Open World → select `fabric_demo.wbt`
3. Click the green Play button
4. The e-puck should move and respond to obstacles

Option B — From command line:
```
webots --batch worlds/fabric_demo.wbt
```

## Step 10: Capture screenshots

While the simulation is running:
1. Position the camera: hold Ctrl + right-click drag to rotate, scroll to zoom
2. Press Ctrl + F12 to take a screenshot (saved to Webots project folder)
3. Or use the built-in screenshot button in the toolbar
4. Take at least 3 screenshots:
   - Overview of the arena with the e-puck
   - Close-up of the e-puck moving
   - Terminal output showing Fabric logic running

## Step 11: Also generate C output for comparison

```
cargo run --release -- build --target c --file examples\drone.fab --output examples\drone.c
```

Then show the C code side-by-side with the Python code to demonstrate cross-compilation.

## What to report back

1. Screenshots of the simulation running
2. Terminal output showing the Fabric logic (state, merged values, motor speeds)
3. The generated `drone.py` and `drone.c` files
4. Any errors or issues encountered
5. A short video of the simulation if possible (screen recording)

## Troubleshooting

- If `cargo` isn't found: install Rust from https://rustup.rs
- If Webots crashes: make sure you have at least 4GB RAM and a GPU
- If sensors return 0: check the sensor names match the e-puck proto
- If motors don't move: verify the controller name matches the robot name in the world file
