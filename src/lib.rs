//! `wabhoa` — Western Alliance Bank community association (HOA) assessment
//! payment portal, from the command line.
//!
//! The binary in `main.rs` is a thin clap shell over these modules. They are
//! public so the integration tests can exercise the parsers directly against
//! captured portal markup, which is where the interesting failure modes live —
//! the portal has no API contract, only rendered pages that can drift.
//!
//! Read-only: nothing here mutates portal state. See [`writes`] for the
//! catalog of what is deliberately left unimplemented.

pub mod client;
pub mod commands;
pub mod config;
pub mod dates;
pub mod html;
pub mod parse;
pub mod writes;
