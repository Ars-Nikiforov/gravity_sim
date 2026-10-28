//! Модуль физики: полностью отделён от интерфейса.

pub mod grid;
pub mod multipole;
pub mod presets;
pub mod sim;
pub mod vec;
pub mod worker;

pub use multipole::Region;
pub use presets::{PresetCategory, Stability};
pub use sim::{
    Algorithm, Body, Integrator, OrbitPlan, Scenario, SimParams, orbit_plan, predict_path,
};
pub use vec::{Vec2, Vec3};
pub use worker::{Command, PhysicsWorker};
