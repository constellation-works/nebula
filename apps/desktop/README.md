# Nebula desktop

A menu-bar app over the same corpus `neb` uses. Tauri 2, React, TypeScript;
the Rust shell links `nebula-core` directly, so what it shows is what the CLI
shows.

## Run

```sh
make desktop-dev     # pnpm tauri dev with the development-only CSP overlay
make desktop-check   # tsc + vitest
make desktop         # pnpm tauri build: unsigned .app and .dmg under target/release/bundle/macos/
pnpm test:webview    # build and exercise the production CSP in the native webview (CI: `webview`)
```

Needs pnpm 11.23.0, the version `packageManager` in `package.json` pins with
its sha512 (`corepack enable` installs it and checks the hash), and the Rust
toolchain `rust-toolchain.toml` names. On macOS, `make desktop` creates both
`Nebula.app` and a versioned `Nebula_*.dmg` under
`target/release/bundle/macos/`; see Tauri's [DMG distribution
guide](https://v2.tauri.app/distribute/dmg/) for that installer format. The
app and disk image are unsigned and not notarised.

### Install on macOS

1. On a Mac, run `make desktop` from the repository root.
2. In Finder, open `target/release/bundle/macos/` and double-click the
   generated `Nebula_*.dmg`.
3. In the disk image window, drag `Nebula.app` onto the Applications folder.
   Eject the disk image when the copy finishes.
4. Open Nebula from Applications. Because this build is not signed or
   notarised, Gatekeeper may block the first launch. If it does, try opening
   Nebula once, then open **System Settings → Privacy & Security**, scroll to
   **Security**, and choose **Open Anyway** for Nebula. Confirm the prompt,
   then launch the app again. Apple makes this option available for about an
   hour after the blocked launch; managed Macs may prevent the override.

Only use **Open Anyway** when you trust where the app came from. An unsigned,
unnotarised app has not been checked by Apple, so macOS cannot verify that it
is free of known malware or has not been modified. See Apple's [safe app
opening guidance](https://support.apple.com/102445).

`make desktop-dev` layers `src-tauri/tauri.conf.dev.json` over the production
configuration. `pnpm test:webview` builds with the production CSP plus a
debug-only WebDriver capability, initializes an empty corpus in a temporary
directory, and verifies that inline script injection and a remote image are
both blocked while the Inbox/Graph UI and `graph` IPC command work. The app
and `neb init` run with `HOME` and the XDG directories inside that temporary
directory, so the check never touches your own settings; it fails unless the
app wrote its `settings.json` there. CI's `webview` job runs it on Linux
under `xvfb-run` on every change. Where it cannot run (no display, no
webkit2gtk on Linux, no `cargo`, no `pnpm install`) it prints
`skipped: <what is missing>` and exits 0 before building anything;
`REQUIRE_WEBVIEW=1`, which CI sets, makes that skip a failure.

## The corpus

Found from `$NEBULA_ROOT`, else the nearest corpus at or above the working
directory the app was launched from, else `~/.config/nebula/root`, else
`~/.nebula`. An app opened from a launcher rather than a terminal starts in `/`
or the home directory, where no corpus is found.
Unlike the CLI, the desktop has no `--root` argument. It resolves this path
once at startup and never creates a corpus. If that path has no corpus, the
window says which path it tried; create the corpus at that same path and press
Reload. Changing `$NEBULA_ROOT` or the configured-root file requires restarting
the app before Reload can use the new path. If no path can be resolved at all
(for example `~/.config/nebula/root` is empty or unreadable), the window, the
tray's startup warnings and every command report that error instead of a path.

Nothing is stored in this repository; the corpus is private and lives outside.

## Capture

`Alt+Space` from anywhere opens a floating box. Enter appends one line to
`inbox/YYYY-MM.md`, exactly as `neb capture` does; Escape closes it. The
shortcut is read from `settings.json` under the app's config directory
(`~/Library/Application Support/works.constellation.nebula/` on macOS), which
is written with the default on first launch. Open **Settings** from the tray
menu or the window tab to change the shortcut or launch-at-login setting.

With commits on (`neb config commit on`), each capture, drop and promote is
committed as `neb` commits it. When git refuses that commit (for example the
repository around the corpus ignores it), the write has still happened: the
capture box clears and says `captured (not committed: <why>)`, and the
floating window stays open with that warning until Escape; a drop or promote
removes the entry from the list and notes above it that the change is not
committed. Nothing offers a retry, since retrying would write it twice.
Once the cause is fixed, the next commit records it: a `neb` commit takes the
whole corpus, not just the last write.

The menu-bar item shows the unsettled inbox count and updates when the corpus
changes on disk, whoever changed it.

## Graph

The Graph tab draws the whole corpus from one `graph()` call: a layered DAG
(`elkjs`, top to bottom, parents above children) laid out in a web worker so
a few hundred nodes do not freeze the window, and rendered as plain SVG.

- **Cards.** Status is the colour stripe: seed grey, hypothesis blue, refuted
  red, abandoned muted. Titles are cut at forty characters; hover for the
  whole thing. Up to three tag chips, then `+n`. A `↺` marks a node that
  reopens a refuted one.
- **Edges.** Genealogy (`derives-from`, `refines`, `generalizes`, `reopens`)
  is solid with the arrowhead at the parent. `contradicts` is dashed, has no
  arrowhead, and does not shape the layout: it is drawn afterwards between
  the two cards.
- **Pan and zoom.** Drag or scroll the canvas to pan; Ctrl+scroll zooms about the cursor.
  **Fit** brings the whole drawing back into view. The viewport is yours
  until you press Fit: a change on disk redraws in place.
- **Select.** Click a card to open it in the panel and light its lineage:
  ancestors in amber, descendants in teal, everything else dimmed. Click
  empty canvas or press Escape to clear. Double-click a card, or press
  **Open file** in the panel, to open the node's file in whatever the OS
  opens `.md` with. Tab focuses cards; arrow keys follow connected cards in
  their direction, and Enter or Space selects one. In the window tabs, Left
  and Right switch views, while Home and End go to the first and last view.
- **Filter.** The search box matches id, title, body or status after typing
  pauses; the tag chips require every selected tag, as `neb list --tag` does.
  Nonmatches dim in place, preserving the full layout and its lineage edges.
  The arrows step between matches; **Clear** resets both filters. With a node
  selected, **Lineage only** shows just it, its ancestors and descendants.
- **Panel.** Title, status, dates, tags, kill condition, `closed.why` when
  the node is closed, the body as rendered markdown (GFM; raw HTML is shown
  as text, never rendered), the node's edges as two lists you can click
  through, and its references as a table. `http(s)` and `mailto` URIs open
  in the OS default handler; repo paths and wikilinks are shown as text.
  Drag the panel's left edge to resize it. Read-only: editing is a session
  with the agent, or the editor.
- The toolbar's `N nodes · M ms` is the time from the layout request to the
  frame after the drawing committed, so the performance target is checkable
  in the app itself.

## Remote images

Opening a node makes no network request. A Markdown image hosted on the web
(`http:` or `https:`) renders as a link, labelled with its alt text (or its
URL when there is none), that opens in your default browser when you click
it; it is never fetched into the panel. Bodies are often written by agents or
synced from elsewhere, and fetching an image would tell its host, which you
never chose, that and when you opened the node. The production and
development Content Security Policies back this up with
`img-src 'self' data:`, so an image from anywhere else is blocked, and
`pnpm test:webview` checks that the production policy blocks one.

## Layout

- `src-tauri/` — the shell: commands, file watcher, tray, shortcut.
- `src/` — the frontend. `src/types/` is generated from `nebula-core`
  (`pnpm gen-types`, or `make types` at the root) and never edited by hand;
  CI fails on drift, including a stale or missing file (`make types-check`).
- `src/layout.ts` is the pure adapter (`GraphExport` → elk graph → drawn
  layout); `src/layout.worker.ts` runs elk off the main thread and
  `src/layoutClient.ts` talks to it, falling back to the same elk in-thread
  where there is no `Worker` (vitest), so the tests run the real layout.
