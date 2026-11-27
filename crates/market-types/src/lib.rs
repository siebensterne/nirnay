#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Exact domain types: no I/O, no async, no SBE DTOs.

pub mod fixed;
pub mod model;
mod side;
pub mod units;
pub use side::{InvalidSide, Side};
