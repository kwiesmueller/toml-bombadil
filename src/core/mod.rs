//! Core types and utilities for Bombadil.
//!
//! This module provides foundational types used throughout the crate.

// thiserror v2 generates destructuring code that triggers unused_assignments
#[allow(unused_assignments)]
pub mod error;

pub use error::{BombadilError, Result};
