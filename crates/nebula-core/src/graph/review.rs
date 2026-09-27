//! What needs attention: `open`, the short list, and `review`, the weekly
//! report, with the thresholds both apply.

use crate::error::Result;
use crate::model::{self, Status};
use crate::stamp;
use crate::store::Inbox;
use serde::{Deserialize, Serialize};

use super::Graph;

/// Days a seed may sit untouched before [`open`] and [`review`] raise it.
pub const SEED_DAYS: i64 = 90;
/// Days a hypothesis may sit untouched before `review` calls it stale.
pub const HYPOTHESIS_DAYS: i64 = 30;
/// Days an inbox capture may wait before [`open`] and [`review`] raise it.
pub const INBOX_DAYS: i64 = 14;
/// Days a new node may remain without references before [`open`] and [`review`] raise it.
pub const NO_REFERENCES_DAYS: i64 = 14;

/// Nodes that need attention: what `neb review --short` reports. Serializes
/// as the bare list.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpenReport(pub Vec<OpenItem>);

/// One thing waiting on you.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct OpenItem {
    /// The node, or `inbox` for the inbox as a whole.
    pub id: String,
    /// What it is waiting for.
    pub why: String,
}

/// Hypotheses at least fourteen days old with no references, seeds untouched
/// for ninety days, and inbox captures waiting fourteen days or more.
///
/// The short form of [`review`]: `neb review --short`, and the deprecated
/// `neb open` that forwards to it.
pub fn open(graph: &Graph<'_>, inbox: &Inbox, tags: &[String]) -> Result<OpenReport> {
    let tags = model::normalize_tags(tags);
    let mut items: Vec<OpenItem> = Vec::new();
    let waiting = stale_inbox(inbox);
    if waiting > 0 {
        items.push(OpenItem {
            id: "inbox".into(),
            why: format!(
                "{} waiting over fourteen days; promote or drop them",
                count(waiting, "capture")
            ),
        });
    }
    for d in graph.docs().iter().filter(|d| d.node.has_all_tags(&tags)) {
        let n = &d.node;
        // The genuinely actionable gap: a hypothesis that names what would
        // kill it and has nothing attached that bears on the question.
        if n.status == Status::Hypothesis
            && n.references.is_empty()
            && older_than(&n.created, NO_REFERENCES_DAYS)
        {
            items.push(OpenItem {
                id: n.id.clone(),
                why: "hypothesis with no references".into(),
            });
        }
        if n.status == Status::Seed && older_than(&n.updated, SEED_DAYS) {
            items.push(OpenItem {
                id: n.id.clone(),
                why: "seed untouched for ninety days; abandon it?".into(),
            });
        }
    }
    Ok(OpenReport(items))
}

/// The weekly maintenance report. Serializes as the bare list of findings.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReviewReport(pub Vec<ReviewItem>);

/// Which review rule raised a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "kebab-case")]
pub enum ReviewRule {
    /// A hypothesis nobody has touched lately.
    StaleHypothesis,
    /// A seed gone cold.
    UntouchedSeed,
    /// A live node with nothing attached to it.
    NoReferences,
    /// A hypothesis standing on a kill condition the human never wrote.
    UnconfirmedKill,
    /// Captures rotting in the inbox.
    StaleInbox,
}

impl ReviewReport {
    /// Keep the first `limit` findings under each rule, in order, and drop
    /// the rest. Returns how many each rule lost, leaving out any that lost
    /// none, so a reader can say the list is cut and by how much.
    ///
    /// Per rule rather than overall, so four thousand cold seeds cannot
    /// crowd a stale inbox out of the report.
    pub fn truncate_per_rule(&mut self, limit: usize) -> Vec<(ReviewRule, usize)> {
        /// Count one more under `rule`, and return the count so far.
        fn bump(tally: &mut Vec<(ReviewRule, usize)>, rule: ReviewRule) -> usize {
            if let Some((_, n)) = tally.iter_mut().find(|(r, _)| *r == rule) {
                *n += 1;
                return *n;
            }
            tally.push((rule, 1));
            1
        }

        let mut seen = Vec::new();
        let mut omitted = Vec::new();
        self.0.retain(|item| {
            if bump(&mut seen, item.rule) <= limit {
                return true;
            }
            bump(&mut omitted, item.rule);
            false
        });
        omitted
    }
}

/// One finding from `review`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReviewItem {
    /// Which rule raised it.
    pub rule: ReviewRule,
    /// The node, or `inbox`.
    pub id: String,
    /// Its title, or `inbox`.
    pub title: String,
    /// What to look at, and what is proposed.
    pub reason: String,
}

/// Hypotheses gone quiet, seeds gone cold, nodes nobody has situated, and
/// inbox captures rotting unprocessed.
///
/// Read-only by design. This walks a graph and returns findings; it never
/// edits a node, an inbox entry, or the config. The judgement calls (kill
/// conditions that look satisfied, likely `contradicts` pairs) stay out of
/// scope for the same reason: they belong to the agent that reads this
/// report, not to this query.
///
/// `since` overrides the hypothesis and seed thresholds together. The inbox's
/// fourteen-day rule is unaffected; it is [`open`]'s rule, reused rather than
/// duplicated.
pub fn review(graph: &Graph<'_>, inbox: &Inbox, since: Option<i64>) -> Result<ReviewReport> {
    let hypothesis_days = since.unwrap_or(HYPOTHESIS_DAYS);
    let seed_days = since.unwrap_or(SEED_DAYS);

    let mut stale_hypotheses = Vec::new();
    let mut untouched_seeds = Vec::new();
    let mut no_references = Vec::new();
    let mut unconfirmed_kills = Vec::new();

    for d in graph.docs() {
        let n = &d.node;
        if n.status == Status::Hypothesis && older_than(&n.updated, hypothesis_days) {
            stale_hypotheses.push(ReviewItem {
                rule: ReviewRule::StaleHypothesis,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("hypothesis untouched for {}", count(hypothesis_days, "day")),
            });
        }
        if n.status == Status::Seed && older_than(&n.updated, seed_days) {
            untouched_seeds.push(ReviewItem {
                rule: ReviewRule::UntouchedSeed,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!(
                    "seed untouched for {}; propose: status abandoned",
                    count(seed_days, "day")
                ),
            });
        }
        if n.status == Status::Hypothesis
            && n.references.is_empty()
            && older_than(&n.created, NO_REFERENCES_DAYS)
        {
            no_references.push(ReviewItem {
                rule: ReviewRule::NoReferences,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: "no references attached".into(),
            });
        }
        // A hypothesis is only as honest as the falsifier under it, and a
        // falsifier somebody else proposed is not yet the human's claim.
        if n.status == Status::Hypothesis && !model::is_human(n.kill_by.as_deref()) {
            let by = n.kill_by.as_deref().unwrap_or(model::HUMAN);
            unconfirmed_kills.push(ReviewItem {
                rule: ReviewRule::UnconfirmedKill,
                id: n.id.clone(),
                title: n.title.clone(),
                reason: format!("kill condition written by `{by}`, not confirmed by a human"),
            });
        }
    }

    let waiting = stale_inbox(inbox);
    let mut items = stale_hypotheses;
    items.append(&mut untouched_seeds);
    items.append(&mut no_references);
    items.append(&mut unconfirmed_kills);
    if waiting > 0 {
        items.push(ReviewItem {
            rule: ReviewRule::StaleInbox,
            id: "inbox".into(),
            title: "inbox".into(),
            reason: format!(
                "{} waiting over fourteen days; promote or drop them",
                count(waiting, "capture")
            ),
        });
    }
    Ok(ReviewReport(items))
}

/// `1 capture`, `2 captures`: a count with its noun agreeing. Every noun
/// these reports count takes a plain `s`.
fn count<N>(n: N, noun: &str) -> String
where
    N: std::fmt::Display + PartialEq + From<u8> + Copy,
{
    if n == N::from(1) {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Whether a `YYYY-MM-DD` date is at least `days` old.
fn older_than(date: &str, days: i64) -> bool {
    stamp::days_since(date).is_some_and(|d| d >= days)
}

/// Captures waiting at least fourteen days.
fn stale_inbox(inbox: &Inbox) -> usize {
    inbox
        .0
        .iter()
        .filter_map(|entry| stamp::days_since_stamp(&entry.at))
        .filter(|days| *days >= INBOX_DAYS)
        .count()
}
