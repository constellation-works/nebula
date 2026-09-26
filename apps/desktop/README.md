# Nebula desktop

A menu-bar app over the same corpus `neb` uses. Tauri 2, React, TypeScript;
the Rust shell links `nebula-core` directly, so what it shows is what the CLI
shows.

## Run

```sh
make desktop-dev     # pnpm tauri dev with the development-only CSP overlay
make desktop-check   # tsc + vitest
make desktop         # pnpm tauri build: unsigned .app and .dmg under target/release/bundle/macos/
pnpm test:webview    # build and exercise the production CSP in the native webview
```

Needs `pnpm` and a Rust toolchain. On macOS, `make desktop` creates both
`Nebula.app` and a versioned `Nebula_*.dmg` under
`target/release/bundle/macos/`. The app and disk image are unsigned and not
notarised.

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
configuration. The WebDriver check builds with the production CSP plus a
debug-only WebDriver capability, initializes an empty corpus in a temporary
directory, and verifies that inline script injection is blocked while the
Inbox/Graph UI and `graph` IPC command work.

## The corpus

Found from `$NEBULA_ROOT`, else `~/.config/nebula/root`, else `~/.nebula`.
Unlike the CLI, the desktop has no `--root` argument. It resolves this path
once at startup and never creates a corpus. If that path has no corpus, the
window says which path it tried; create the corpus at that same path and press
Reload. Changing `$NEBULA_ROOT` or the configured-root file requires restarting
the app before Reload can use the new path.

Nothing is stored in this repository; the corpus is private and lives outside.

## Capture

`Alt+Space` from anywhere opens a floating box. Enter appends one line to
`inbox/YYYY-MM.md`, exactly as `neb capture` does; Escape closes it. The
shortcut is read from `settings.json` under the app's config directory
(`~/Library/Application Support/works.constellation.nebula/` on macOS), which
is written with the default on first launch.

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
- **Pan and zoom.** Drag the canvas to pan, wheel to zoom about the cursor.
  **Fit** brings the whole drawing back into view. The viewport is yours
  until you press Fit: a change on disk redraws in place.
- **Select.** Click a card to open it in the panel and light its lineage:
  ancestors in amber, descendants in teal, everything else dimmed. Click
  empty canvas or press Escape to clear. Double-click a card, or press
  **Open file** in the panel, to open the node's file in whatever the OS
  opens `.md` with.
- **Filter.** The search box matches title substrings; the tag chips narrow
  to nodes carrying every selected tag, as `neb list --tag` does. Filters
  apply before layout, so what is left is redrawn compactly; edges to hidden
  nodes disappear with them. **Clear** resets both.
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

Node bodies can include Markdown images hosted on the web. The production and
development Content Security Policies allow images from HTTPS URLs
(`img-src 'self' https:`), while scripts, connections, and other content stay
restricted to their existing sources. Loading a remote image contacts its
host directly from your Mac; that host can observe the request and its timing,
so image URLs can also be used for tracking. Do not open node bodies with
remote images unless that trade-off is acceptable. See Tauri's [DMG
distribution guide](https://v2.tauri.app/distribute/dmg/) for the installer
format used by `make desktop`.

## Layout

- `src-tauri/` — the shell: commands, file watcher, tray, shortcut.
- `src/` — the frontend. `src/types/` is generated from `nebula-core`
  (`pnpm gen-types`, or `make types` at the root) and never edited by hand;
  CI fails on drift.
- `src/layout.ts` is the pure adapter (`GraphExport` → elk graph → drawn
  layout); `src/layout.worker.ts` runs elk off the main thread and
  `src/layoutClient.ts` talks to it, falling back to the same elk in-thread
  where there is no `Worker` (vitest), so the tests run the real layout.
