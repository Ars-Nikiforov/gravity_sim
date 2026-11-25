//! Модуль физики: полностью отделён от интерфейса.

pub mod barnes_hut;
pub mod grid;
pub mod multipole;
pub mod presets;
pub mod profiler;
pub mod sim;
pub mod vec;
pub mod worker;

pub use multipole::Region;
pub use presets::{PresetCategory, StableSystemSpec, Stability};
pub use sim::{
    Algorithm, Body, Integrator, OrbitPlan, Scenario, SimParams, orbit_plan, predict_path,
    predict_paths,
};
pub use vec::{Vec2, Vec3};
pub use worker::{Command, PhysicsWorker};
