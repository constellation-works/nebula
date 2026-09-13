//! The data layer: what a corpus is, where it lives, and how it is read
//! and written. Nothing here knows about the terminal or about clap.

pub mod config;
pub mod model;
pub mod store;

pub use model::{Closed, Doc, Edge, EdgeType, Node, Origin, Reference, Status};
pub use store::Store;
