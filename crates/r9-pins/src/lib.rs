//! The sets, pins, rows and machines of rucc-plan9 (plan 04.1 and 04.4).
//!
//! `sets.toml` states rules: which branch, and how many releases. [`expand::expand`] reads what
//! 9front publishes today and turns the rules into the pins of `pins.toml`: a commit for each
//! tree, and the SHA-256 that 9front publishes for each release image.

pub mod expand;
pub mod fetch;
pub mod machines;
pub mod pins;
pub mod publish;
pub mod rows;
pub mod sets;
pub mod web;
