//! The 9P2000 server of the 9h monitor (plan 10.5). It serves a host directory, which the 9h
//! kernel mounts on `/mnt/term` through `#Z/hostfs`, as `drawterm` serves the host's files.
//!
//! [`msg`] reads and writes the messages. [`Server`] answers them with the host file system. The
//! server does not know the transport: [`Server::serve`] reads messages from a byte stream and
//! writes the replies, and the monitor gives it the stream of the two rings of `#Z/hostfs`.

pub mod msg;
mod server;

pub use server::{Options, Server};
