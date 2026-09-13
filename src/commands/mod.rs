//! One function per verb, grouped by what they act on. Every function takes
//! the resolved `--root` and returns an error the CLI prints; none of them
//! know about clap.

mod attach;
mod check;
mod inbox;
mod migrate;
mod node;
mod read;

pub use attach::cite;
pub use check::check;
pub use inbox::{capture, drop_entry, inbox, init, promote};
pub use migrate::migrate;
pub use node::{link, new_node, set_status, sharpen, tag, tag_list};
pub use read::{impact, list, open, review, show, trace};

use crate::corpus::Store;
use anyhow::Result;
use std::path::PathBuf;

fn store_at(root: Option<PathBuf>) -> Result<Store> {
    Store::open(root)
}

fn out_json<T: serde::Serialize>(v: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}
