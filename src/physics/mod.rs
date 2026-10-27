//! Модуль физики: полностью отделён от интерфейса.

pub mod sim;
pub mod vec;
pub mod worker;

pub use sim::{Algorithm, Body, Scenario, SimParams};
pub use vec::Vec2;
pub use worker::{Command, PhysicsWorker, Snapshot};
