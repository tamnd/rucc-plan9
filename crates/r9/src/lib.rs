//! The library behind `r9`: fetch pinned 9front trees, build them with rucc and with kencc in
//! 9front guests, boot them, test them, and record what happened.
//!
//! The commands arrive one milestone at a time, in the order of plan 16. Each command has its own
//! module, and the binary only parses the command line and calls it.

pub mod boot;
pub mod build;
pub mod census;
pub mod check;
pub mod cli;
pub mod console;
pub mod demands;
pub mod disk;
pub mod fetch;
pub mod image;
pub mod locate;
pub mod repo;
pub mod sets;
pub mod treediff;
