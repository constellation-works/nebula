//! Online half of invariant 13: resolve cited Orbit task ids.
//!
//! Off by default. `check --online` spawns the Orbit CLI as a subprocess
//! (never a shell) so a corpus cannot cite a task that never existed, and so
//! a `tasks` entry still marked open is flagged once Orbit has closed it.

use super::{Level, Report};
use crate::corpus::Doc;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// Environment override for the Orbit program. Tests point this at a stub.
const ORBIT_BIN_ENV: &str = "NEBULA_ORBIT_BIN";

enum Resolve {
    Status(String),
    Failed(String),
}

/// Resolve every well-formed `origin.task` and `tasks[].id` once, then apply
/// the cached results to each citing node.
pub(super) fn check_tasks(docs: &[Doc], r: &mut Report) {
    let ids = well_formed_ids(docs);
    if ids.is_empty() {
        return;
    }

    let bin = orbit_bin();
    if !program_exists(&bin) {
        r.push(
            Level::Error,
            13,
            None,
            "orbit binary not found (set NEBULA_ORBIT_BIN or install orbit on PATH); skipping online task resolution",
        );
        return;
    }

    let mut cache = HashMap::new();
    for id in &ids {
        cache.insert(*id, resolve(&bin, id));
    }

    for doc in docs {
        apply_node(doc, &cache, r);
    }
}

fn apply_node(doc: &Doc, cache: &HashMap<&str, Resolve>, r: &mut Report) {
    let node = doc.node.id.as_str();
    let mut reported = HashSet::new();

    if let Some(id) = doc.node.origin.as_ref().and_then(|o| o.task.as_deref()) {
        if let Some(Resolve::Failed(detail)) = cache.get(id)
            && reported.insert(id)
        {
            r.push(
                Level::Error,
                13,
                Some(node),
                format!("Orbit task `{id}` did not resolve: {detail}"),
            );
        }
    }

    for task in &doc.node.tasks {
        let id = task.id.as_str();
        match cache.get(id) {
            Some(Resolve::Failed(detail)) if reported.insert(id) => {
                r.push(
                    Level::Error,
                    13,
                    Some(node),
                    format!("Orbit task `{id}` did not resolve: {detail}"),
                );
            }
            Some(Resolve::Status(status)) if task.is_open() && is_terminal(status) => {
                r.push(
                    Level::Warn,
                    13,
                    Some(node),
                    format!("task {id} is {status} in Orbit but marked open on this node"),
                );
            }
            _ => {}
        }
    }
}

fn well_formed_ids(docs: &[Doc]) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for doc in docs {
        let tasks = doc.node.tasks.iter().map(|t| t.id.as_str());
        let origin = doc.node.origin.as_ref().and_then(|o| o.task.as_deref());
        for id in tasks.chain(origin) {
            if super::rules::is_task_id(id) && seen.insert(id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn orbit_bin() -> PathBuf {
    std::env::var_os(ORBIT_BIN_ENV).map_or_else(|| PathBuf::from("orbit"), PathBuf::from)
}

fn program_exists(bin: &Path) -> bool {
    if looks_like_path(bin) {
        bin.is_file()
    } else {
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file())
    }
}

fn looks_like_path(bin: &Path) -> bool {
    bin.components().any(|c| {
        matches!(
            c,
            Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir
        )
    }) || bin.components().count() > 1
}

fn resolve(bin: &Path, id: &str) -> Resolve {
    let input = json!({ "id": id, "fields": ["status"] }).to_string();
    match Command::new(bin)
        .args(["tool", "run", "orbit.task.show", "--input", &input])
        .stdin(Stdio::null())
        .output()
    {
        Err(err) => Resolve::Failed(err.to_string()),
        Ok(out) if out.status.success() => match parse_status(&out.stdout) {
            Ok(status) => Resolve::Status(status),
            Err(detail) => Resolve::Failed(detail),
        },
        Ok(out) => {
            let detail = first_line(&out.stderr)
                .or_else(|| first_line(&out.stdout))
                .unwrap_or_else(|| "non-zero exit".into());
            Resolve::Failed(detail)
        }
    }
}

fn parse_status(stdout: &[u8]) -> Result<String, String> {
    let text = String::from_utf8_lossy(stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("resolver produced empty stdout".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| format!("resolver stdout is not JSON: {e}"))?;
    match value {
        serde_json::Value::String(status) if !status.is_empty() => Ok(status),
        serde_json::Value::Object(map) => map
            .get("status")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "resolver JSON has no status".into()),
        _ => Err("resolver JSON has no status".into()),
    }
}

fn first_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn is_terminal(status: &str) -> bool {
    matches!(status, "done" | "rejected" | "archived")
}
