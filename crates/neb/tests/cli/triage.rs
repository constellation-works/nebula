//! `neb triage`: an interactive session over the inbox, oldest first, deciding
//! each entry as the single verbs would.

use crate::harness::{
    Corpus, assert_only_corpus_paths_in_log, corpus_repo, log, set_inbox_stamp_for, stamp_days_ago,
};
use crate::near::lexical_fixture;

/// The node id `triage` printed for an entry it promoted.
fn promoted_as(out: &str, entry: &str) -> String {
    let prefix = format!("promoted {entry} -> ");
    out.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("`{entry}` was not promoted:\n{out}"))
        .to_string()
}

/// Where each needle first appears in `out`, so an order can be asserted.
fn position(out: &str, needle: &str) -> usize {
    out.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not in:\n{out}"))
}

#[test]
fn triage_decides_each_entry_oldest_first_as_the_single_verbs_would() {
    let (c, _remote) = corpus_repo();
    lexical_fixture(&c);
    // Captured newest first, so file order and age order disagree.
    let later = c.run(&["capture", "-q", "later maybe"]).stdout_trim();
    let coffee = c.run(&["capture", "-q", "buy more coffee"]).stdout_trim();
    let tags = c
        .run(&["capture", "-q", "a taxonomy for tags"])
        .stdout_trim();
    let decay = c
        .run(&["capture", "-q", "ranking decay again"])
        .stdout_trim();
    set_inbox_stamp_for(&c.root, "ranking decay again", &stamp_days_ago(4));
    set_inbox_stamp_for(&c.root, "a taxonomy for tags", &stamp_days_ago(3));
    set_inbox_stamp_for(&c.root, "buy more coffee", &stamp_days_ago(2));
    set_inbox_stamp_for(&c.root, "later maybe", &stamp_days_ago(1));
    c.run(&["config", "commit", "on"]).assert_ok();

    // Root, the first candidate, drop, skip.
    let run = c.run_with_stdin(&["triage"], "p\n1\nd\ns\n").assert_ok();
    let out = run.stdout();
    assert!(
        position(&out, "ranking decay again") < position(&out, "a taxonomy for tags")
            && position(&out, "a taxonomy for tags") < position(&out, "buy more coffee")
            && position(&out, "buy more coffee") < position(&out, "later maybe"),
        "oldest first:\n{out}"
    );
    assert!(out.contains("[1/4]") && out.contains("[4/4]"), "{out}");
    assert!(out.contains("· 4 days"), "each entry shows its age:\n{out}");
    assert!(
        out.lines().any(|l| {
            let mut words = l.split_whitespace();
            words.next() == Some("1")
                && words
                    .next()
                    .is_some_and(|band| ["strong", "some", "weak"].contains(&band))
        }) && out.contains("a-single-global-taxonomy"),
        "the candidates are numbered, each with its band:\n{out}"
    );
    assert!(
        !out.lines().any(|l| l.starts_with("> ")) && !out.contains("q quit"),
        "no prompt or key legend when the keys are scripted:\n{out}"
    );

    let root = promoted_as(&out, &decay);
    let raw = std::fs::read_to_string(c.node_file(&root)).unwrap();
    assert!(!raw.contains("edges:"), "`p` promotes as a root:\n{raw}");
    let child = promoted_as(&out, &tags);
    let raw = std::fs::read_to_string(c.node_file(&child)).unwrap();
    assert!(
        raw.contains("- type: derives-from\n  to: a-single-global-taxonomy\n"),
        "`1` promotes under the first candidate, and only it:\n{raw}"
    );
    assert_eq!(raw.matches("- type:").count(), 1, "{raw}");
    assert!(out.contains(&format!("dropped {coffee}")), "{out}");
    assert!(out.contains(&format!("skipped {later}")), "{out}");
    assert!(
        out.contains("promoted 2, dropped 1, skipped 1; 1 still waiting"),
        "{out}"
    );

    // One commit per write, named as the single verb names its own, and
    // said on stderr: the session's screen is stdout.
    assert_eq!(run.stderr().matches("committed ").count(), 3, "{out}");
    assert!(!out.contains("committed "), "{out}");
    assert_eq!(
        log(&c.root)[..3],
        [
            format!("neb drop {coffee}"),
            format!("neb promote {tags} {child}"),
            format!("neb promote {decay} {root}"),
        ]
    );
    assert_only_corpus_paths_in_log(&c.root);
    let waiting = c.run(&["inbox"]).assert_ok().stdout();
    assert!(waiting.contains(&later), "{waiting}");
    for settled in [&decay, &tags, &coffee] {
        assert!(!waiting.contains(settled.as_str()), "{waiting}");
    }

    // `--no-commit` waives it for the whole session, as for one verb.
    let entry = c.run(&["capture", "-q", "one more"]).stdout_trim();
    let commits = log(&c.root).len();
    c.run_with_stdin(&["triage", "--no-commit"], "s\nd\n")
        .assert_ok();
    assert!(!c.run(&["inbox"]).stdout().contains(&entry));
    assert_eq!(log(&c.root).len(), commits);
}

#[test]
fn triage_skip_quit_and_end_of_input_leave_the_rest_waiting() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    let second = c.run(&["capture", "-q", "second thought"]).stdout_trim();
    let before = c.run(&["inbox"]).assert_ok().stdout();

    let out = c
        .run_with_stdin(&["triage"], "s\nq\nd\n")
        .assert_ok()
        .stdout();
    assert!(out.contains(&format!("skipped {first}")), "{out}");
    assert!(
        out.contains("promoted 0, dropped 0, skipped 1; 2 still waiting"),
        "`q` stops before the `d` after it:\n{out}"
    );
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A blank line decides nothing, and end of input stops as `q` does.
    let out = c.run_with_stdin(&["triage"], "\n").assert_ok().stdout();
    assert!(out.contains(&second) || out.contains(&first), "{out}");
    assert!(out.contains("skipped 0; 2 still waiting"), "{out}");
    assert_eq!(c.run(&["inbox"]).stdout(), before);
    let out = c.run_with_stdin(&["triage"], "").assert_ok().stdout();
    assert!(out.contains("skipped 0; 2 still waiting"), "{out}");
}

/// End of input stops as `q` does, except while a title waits to be used:
/// then the title would be lost without a word, so it is a refusal naming it
/// and the single verb that does the same without triage (STD-01 §R27).
#[test]
fn triage_eof_with_a_pending_title_is_an_error() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    c.run(&["capture", "-q", "second thought"]).assert_ok();
    let before = c.run(&["inbox"]).assert_ok().stdout();

    // `t` alone asks for the title on the next line, which never comes.
    let run = c.run_with_stdin(&["triage"], "t\n");
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.starts_with(&format!(
            "error: input ended before the title for `{first}` was given; nothing was promoted"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("neb promote {first} --title \"...\"")),
        "{stderr}"
    );
    assert!(!run.stdout().contains("still waiting"), "{}", run.stdout());
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A title set and never used is named, quoted for the shell.
    let run = c.run_with_stdin(&["triage"], "t A lost title\n");
    assert_eq!(run.out.status.code(), Some(1));
    let stderr = run.stderr();
    assert!(
        stderr.contains(&format!("the title `A lost title` was used for `{first}`"))
            && stderr.contains(&format!("neb promote {first} --title 'A lost title'")),
        "{stderr}"
    );
    assert_eq!(c.run(&["inbox"]).stdout(), before);

    // A title that was used is not pending, so end of input is `q` again.
    let run = c
        .run_with_stdin(&["triage"], "t A kept title\np\n")
        .assert_ok();
    assert!(run.stdout().contains("promoted 1"), "{}", run.stdout());
}

#[test]
fn triage_titles_a_promotion_and_attributes_it_with_by() {
    let c = Corpus::new();
    let a = c.run(&["capture", "-q", "the first words"]).stdout_trim();
    let b = c.run(&["capture", "-q", "the second words"]).stdout_trim();

    // `t` alone asks for the title on the next line; `t <title>` inlines it.
    let out = c
        .run_with_stdin(
            &["triage", "--by", "agent-x"],
            "t\nMy own title\np\nt Inline title\np\n",
        )
        .assert_ok()
        .stdout();
    assert_eq!(promoted_as(&out, &a), "my-own-title");
    assert_eq!(promoted_as(&out, &b), "inline-title");
    let raw = std::fs::read_to_string(c.node_file("my-own-title")).unwrap();
    assert!(raw.contains("title: My own title\n"), "{raw}");
    assert!(raw.contains("title_by: agent-x\n"), "{raw}");
    assert!(
        raw.contains("the first words"),
        "the capture is the body:\n{raw}"
    );
}

#[test]
fn triage_refuses_json_and_names_the_scriptable_verbs() {
    let c = Corpus::new();
    let entry = c.run(&["capture", "-q", "a thought"]).stdout_trim();
    for args in [["--json", "triage"], ["triage", "--json"]] {
        let run = c
            .run_with_stdin(&args, "d\n")
            .assert_fails()
            .says("interactive and has no JSON form")
            .says("neb inbox --json")
            .says("neb promote <entry>")
            .says("neb drop <entry>");
        assert!(run.stdout().is_empty(), "{}", run.stdout());
    }
    assert!(c.run(&["inbox"]).stdout().contains(&entry));
}

#[test]
fn a_piped_triage_refusal_ends_the_session_non_zero() {
    let c = Corpus::new();
    let first = c.run(&["capture", "-q", "first thought"]).stdout_trim();
    let second = c.run(&["capture", "-q", "second thought"]).stdout_trim();

    // An unknown key: the lines after it were written for an entry that did
    // not move, so none of them runs.
    let run = c
        .run_with_stdin(&["triage"], "x\nd\n")
        .assert_fails()
        .says("`x` is not a triage key");
    assert!(!run.stdout().contains("dropped"), "{}", run.stdout());

    // A candidate the entry was not shown is refused, not guessed at.
    c.run_with_stdin(&["triage"], "3\nd\n")
        .assert_fails()
        .says("there is no candidate 3; this entry has no candidates");

    // A refusal after a decision keeps the decision.
    c.run_with_stdin(&["triage"], "d\nt !!!\np\ns\n")
        .assert_fails()
        .says(&format!("dropped {first}"))
        .says("does not reduce to a usable id");
    let waiting = c.run(&["inbox"]).stdout();
    assert!(!waiting.contains(&first), "{waiting}");
    assert!(waiting.contains(&second), "{waiting}");
}

#[test]
fn triage_on_an_empty_inbox_says_so() {
    let c = Corpus::new();
    c.run_with_stdin(&["triage"], "p\n")
        .assert_ok()
        .says("inbox is empty");
}
