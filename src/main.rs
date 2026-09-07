//! neb — the nebula CLI.
//!
//! Arguments are parsed by hand rather than with a framework. The surface is
//! five verbs, and `capture` has a five-second budget it must never miss.

mod check;
mod model;

use anyhow::{Result, bail};
use model::*;
use std::path::{Path, PathBuf};

/// The corpus lives outside this repository. See README, "Repository boundary".
fn root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("NEBULA_ROOT") {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME")?;
    let default = PathBuf::from(&home).join(".nebula");
    if default.exists() {
        return Ok(default);
    }
    bail!("no corpus found. Set NEBULA_ROOT, or run `neb init <path>`.")
}

fn today() -> String {
    use time::{OffsetDateTime, macros::format_description};
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(format_description!("[year]-[month]-[day]"))
        .unwrap_or_default()
}

fn stamp() -> String {
    use time::{OffsetDateTime, macros::format_description};
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(format_description!("[year]-[month]-[day]T[hour]:[minute]"))
        .unwrap_or_default()
}

/// Short, stable, pronounceable enough to retype from a glance.
fn short_id(seed: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{:04x}", (h & 0xffff) as u16)
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').chars().take(60).collect()
}

fn node_path(root: &Path, id: &str) -> PathBuf {
    root.join("nodes").join(format!("{id}.md"))
}

/// Load every node in the corpus. Small corpus, so a full scan is cheaper
/// than maintaining an index that could drift.
fn load_all(root: &Path) -> Result<Vec<Doc>> {
    let dir = root.join("nodes");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "md") {
            out.push(model::read(&p)?);
        }
    }
    out.sort_by(|a, b| a.node.id.cmp(&b.node.id));
    Ok(out)
}

// ---------------------------------------------------------------- capture --

/// The five-second path. One line, no fields, no decisions. If this ever asks
/// you to pick a parent you will stop using it, and the corpus dies with it.
fn cmd_capture(args: &[String]) -> Result<()> {
    let text = args.join(" ");
    if text.trim().is_empty() {
        bail!("neb capture \"the thought\"");
    }
    let root = root()?;
    let dir = root.join("inbox");
    std::fs::create_dir_all(&dir)?;
    let now = stamp();
    let month = &now[..7];
    let id = short_id(&format!("{now}{text}"));
    let line = format!("- [{id}] {now} {}\n", text.trim());

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{month}.md")))?;
    f.write_all(line.as_bytes())?;
    println!("{id}");
    Ok(())
}

fn cmd_inbox() -> Result<()> {
    let root = root()?;
    let dir = root.join("inbox");
    if !dir.exists() {
        println!("inbox is empty");
        return Ok(());
    }
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    for f in files {
        for line in std::fs::read_to_string(&f)?.lines() {
            if !line.trim().is_empty() && !line.trim_start().starts_with("- ~~") {
                println!("{line}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- promote --

/// An inbox entry becomes a seed node. Deliberately a separate, explicit act:
/// most captures should never be promoted, and dropping one is a normal
/// outcome rather than a failure.
fn cmd_promote(args: &[String]) -> Result<()> {
    let mut entry = None;
    let mut title = None;
    let mut parents: Vec<String> = vec![];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--title" => title = it.next().cloned(),
            "--parent" => parents.extend(it.next().cloned()),
            _ if entry.is_none() => entry = Some(a.clone()),
            _ => bail!("unexpected argument: {a}"),
        }
    }
    let entry = entry.ok_or_else(|| anyhow::anyhow!("neb promote <inbox-id> [--title T] [--parent ID]"))?;
    let root = root()?;

    // Find the inbox line and lift its text as the node's opening prose.
    let mut found = None;
    let dir = root.join("inbox");
    let mut files: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    for f in &files {
        let content = std::fs::read_to_string(f)?;
        for line in content.lines() {
            if line.contains(&format!("[{entry}]")) {
                let text = line.splitn(3, ' ').nth(2).unwrap_or("").trim().to_string();
                found = Some((f.clone(), line.to_string(), text));
            }
        }
    }
    let Some((file, line, text)) = found else {
        bail!("no inbox entry [{entry}]");
    };

    let title = title.unwrap_or_else(|| text.clone());
    let id = slugify(&title);
    let path = node_path(&root, &id);
    if path.exists() {
        bail!("node {id} already exists");
    }
    std::fs::create_dir_all(root.join("nodes"))?;

    let now = today();
    let doc = Doc {
        node: Node {
            id: id.clone(),
            title,
            status: Status::Seed,
            created: now.clone(),
            updated: now,
            kill: None,
            tags: vec![],
            edges: parents
                .iter()
                .map(|p| Edge { kind: EdgeType::DerivesFrom, to: p.clone() })
                .collect(),
            evidence: vec![],
            references: vec![],
            tasks: vec![],
            origin: None,
            graduated_to: None,
        },
        body: format!("{text}\n"),
    };
    model::write(&path, &doc)?;

    // Strike the inbox entry rather than deleting it. Nothing in this system
    // is ever removed, including the record of what a node started as.
    let content = std::fs::read_to_string(&file)?;
    let struck = content.replace(&line, &format!("- ~~{}~~ -> {id}", line.trim_start_matches("- ")));
    std::fs::write(&file, struck)?;

    println!("{}", path.display());
    Ok(())
}

// ------------------------------------------------------------------- cite --

fn cmd_cite(args: &[String]) -> Result<()> {
    let (mut id, mut kind, mut uri, mut note, mut title) = (None, None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--kind" => kind = it.next().cloned(),
            "--uri" => uri = it.next().cloned(),
            "--note" => note = it.next().cloned(),
            "--title" => title = it.next().cloned(),
            _ if id.is_none() => id = Some(a.clone()),
            _ => bail!("unexpected argument: {a}"),
        }
    }
    let id = id.ok_or_else(|| anyhow::anyhow!("neb cite <node> --uri U [--kind K] [--note N]"))?;
    let uri = uri.ok_or_else(|| anyhow::anyhow!("--uri is required"))?;
    let root = root()?;
    let path = node_path(&root, &id);
    let mut doc = model::read(&path)?;
    let rid = format!("r{}", doc.node.references.len() + 1);
    doc.node.references.push(Reference {
        id: rid.clone(),
        kind: kind.unwrap_or_else(|| "other".into()),
        uri,
        title,
        note,
        added: today(),
        promoted_to: None,
        origin: None,
    });
    doc.node.updated = today();
    model::write(&path, &doc)?;
    println!("{id} {rid}");
    Ok(())
}

// ------------------------------------------------------------------ trace --

/// Walk genealogy upward. This is the feature the whole system exists for;
/// everything else is bookkeeping that keeps this walk honest.
fn cmd_trace(args: &[String]) -> Result<()> {
    let id = args.first().ok_or_else(|| anyhow::anyhow!("neb trace <node>"))?;
    let root = root()?;
    let docs = load_all(&root)?;
    let by_id: std::collections::HashMap<&str, &Doc> =
        docs.iter().map(|d| (d.node.id.as_str(), d)).collect();
    if !by_id.contains_key(id.as_str()) {
        bail!("no node {id}");
    }
    let mut seen = std::collections::HashSet::new();
    walk(id, &by_id, "", 0, true, &mut seen);
    Ok(())
}

fn walk(
    id: &str,
    by_id: &std::collections::HashMap<&str, &Doc>,
    prefix: &str,
    depth: usize,
    last: bool,
    seen: &mut std::collections::HashSet<String>,
) {
    let branch = if depth == 0 {
        String::new()
    } else if last {
        format!("{prefix}`- ")
    } else {
        format!("{prefix}|- ")
    };
    let Some(doc) = by_id.get(id) else {
        println!("{branch}{id}  [missing]");
        return;
    };
    let n = &doc.node;
    // A node reachable by two paths is a diamond, which is legal and expected.
    // Print it in both places, but only expand it once.
    let repeat = !seen.insert(id.to_string());
    println!(
        "{branch}{} [{}] {}{}",
        n.id,
        format!("{:?}", n.status).to_lowercase(),
        n.title,
        if repeat { "  (shown above)" } else { "" }
    );
    if repeat {
        return;
    }
    let parents: Vec<&str> = n.parents().collect();
    let child_prefix = if depth == 0 {
        String::new()
    } else {
        format!("{prefix}{}", if last { "   " } else { "|  " })
    };
    for (i, p) in parents.iter().enumerate() {
        walk(p, by_id, &child_prefix, depth + 1, i + 1 == parents.len(), seen);
    }
}

// -------------------------------------------------------------------------

fn usage() -> ! {
    eprintln!(
        "neb — idea lineage graph\n\
         \n\
           capture <text>            append a thought to the inbox\n\
           inbox                     list unprocessed captures\n\
           promote <id> [--title T] [--parent ID]\n\
           cite <node> --uri U [--kind K] [--note N]\n\
           trace <node>              walk ancestry\n\
           check                     run the invariants\n\
         \n\
         corpus location: $NEBULA_ROOT, else ~/.nebula"
    );
    std::process::exit(2)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else { usage() };
    let rest = &args[1..];
    match cmd.as_str() {
        "capture" => cmd_capture(rest),
        "inbox" => cmd_inbox(),
        "promote" => cmd_promote(rest),
        "cite" => cmd_cite(rest),
        "trace" => cmd_trace(rest),
        "check" => check::run(&root()?),
        "-h" | "--help" | "help" => usage(),
        other => bail!("unknown command: {other}"),
    }
}
