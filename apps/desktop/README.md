# Nebula desktop

A menu-bar app over the same corpus `neb` uses. Tauri 2, React, TypeScript;
the Rust shell links `nebula-core` directly, so what it shows is what the CLI
shows.

## Run

```sh
make desktop-dev     # pnpm tauri dev: live reload for the frontend
make desktop-check   # tsc + vitest
make desktop         # pnpm tauri build: target/release/bundle/macos/Nebula.app
```

Needs `pnpm` and a Rust toolchain. The `.app` is not signed or notarised;
macOS will ask you to allow it the first time.

## The corpus

Found the way the CLI finds it: `NEBULA_ROOT`, else `~/.nebula`. The app
never creates one. If the path has no corpus, the window says which path it
tried and offers a reload once you have set `NEBULA_ROOT` or run `neb init`.

Nothing is stored in this repository; the corpus is private and lives outside.

## Capture

`Alt+Space` from anywhere opens a floating box. Enter appends one line to
`inbox/YYYY-MM.md`, exactly as `neb capture` does; Escape closes it. The
shortcut is read from `settings.json` under the app's config directory
(`~/Library/Application Support/works.constellation.nebula/` on macOS), which
is written with the default on first launch.

The menu-bar item shows the unsettled inbox count and updates when the corpus
changes on disk, whoever changed it.

## Layout

- `src-tauri/` — the shell: commands, file watcher, tray, shortcut.
- `src/` — the frontend. `src/types/` is generated from `nebula-core`
  (`pnpm gen-types`, or `make types` at the root) and never edited by hand;
  CI fails on drift.
