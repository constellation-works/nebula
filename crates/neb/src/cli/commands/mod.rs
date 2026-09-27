//! One handler per verb, grouped by the section `neb --help` lists it under.
//!
//! Each handler is one arm of [`super::run`]: it takes that variant's
//! arguments and the [`super::Invocation`] `run` settled, makes one core
//! operation and says what it did.

// A handler's parameters are its variant's fields, by value and under their
// clap names, as the arm of `run` destructured them, so its body is that
// arm's unchanged.
#![allow(
    clippy::needless_pass_by_value,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "a handler's parameters are its clap variant's fields, as `run` destructured them"
)]

pub(super) mod corpus;
pub(super) mod inbox;
pub(super) mod maintenance;
pub(super) mod node;
pub(super) mod query;
pub(super) mod reference;
