//! The invariant checker as a command.

use super::{out_json, store_at};
use crate::check::{self, Level};
use crate::render::{self, bold, dim};
use anyhow::Result;
use std::path::PathBuf;
use std::process::ExitCode;

/// Run the invariants.
pub fn check(root: Option<PathBuf>, json: bool) -> Result<ExitCode> {
    let store = store_at(root)?;
    let report = check::run(&store)?;
    if json {
        out_json(&report)?;
    } else {
        for f in &report.findings {
            let (tag, code) = match f.level {
                Level::Error => ("ERROR", "31;1"),
                Level::Warn => ("warn ", "33"),
            };
            let where_ = f.node.as_deref().unwrap_or("corpus");
            println!(
                "{} {} {} {}",
                render::paint(code, tag),
                dim(&format!("[{}]", f.rule)),
                bold(where_),
                f.message
            );
        }
        println!(
            "\n{} nodes, {} errors, {} warnings",
            report.nodes,
            report.errors(),
            report.warnings()
        );
    }
    Ok(if report.errors() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
