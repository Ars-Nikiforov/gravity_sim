//! Модуль физики: полностью отделён от интерфейса.

pub mod grid;
pub mod sim;
pub mod vec;
pub mod worker;

pub use sim::{Algorithm, Body, Integrator, Scenario, SimParams, predict_path};
pub use vec::{Vec2, Vec3};
pub use worker::{Command, PhysicsWorker};
