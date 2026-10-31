//! Small spring animator used for the notch shape and scrolling.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub v: f32,
    pub target: f32,
    vel: f32,
}

impl Spring {
    pub fn new(v: f32) -> Self {
        Spring { v, target: v, vel: 0.0 }
    }

    pub fn set(&mut self, t: f32) {
        self.target = t;
    }

    pub fn snap(&mut self, t: f32) {
        self.v = t;
        self.target = t;
        self.vel = 0.0;
    }

    pub fn moving(&self) -> bool {
        (self.v - self.target).abs() > 0.05 || self.vel.abs() > 0.05
    }

    /// Advance by `dt` seconds. `k` is stiffness; damping is derived (slightly under-damped).
    /// Returns true while still moving.
    pub fn step(&mut self, dt: f32, k: f32, damping_ratio: f32) -> bool {
        if !self.moving() {
            self.v = self.target;
            self.vel = 0.0;
            return false;
        }
        let c = 2.0 * k.sqrt() * damping_ratio;
        // Sub-step for stability when frames are slow.
        let steps = ((dt / 0.004).ceil() as usize).clamp(1, 16);
        let h = dt / steps as f32;
        for _ in 0..steps {
            let a = -k * (self.v - self.target) - c * self.vel;
            self.vel += a * h;
            self.v += self.vel * h;
        }
        true
    }
}
