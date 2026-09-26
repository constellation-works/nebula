// `pnpm test:webview`: build the app with the production CSP and a debug-only
// WebDriver, then drive it in the native webview (`test/webview.e2e.mjs`).
//
// Where the test cannot run it skips visibly, naming what is missing, and
// exits 0; `REQUIRE_WEBVIEW=1` turns that skip into a failure, which is how
// CI's `webview` job runs it (STD-04 §R8). The capabilities are checked
// before anything is built.
//
// The app and `neb init` run with HOME and the XDG directories inside a
// temporary root, so the app's settings, webview data and caches land there
// and never in the developer's own config (STD-03 §R20). Only the build sees
// the real environment, since cargo and rustup live under the real HOME.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(desktopRoot, "../..");
const tauriCli = path.join(desktopRoot, "node_modules/@tauri-apps/cli/tauri.js");
const wdioCli = path.join(desktopRoot, "node_modules/@wdio/cli/bin/wdio.js");
const required = process.env.REQUIRE_WEBVIEW === "1";

/** Whether `command args` runs and exits 0, without printing anything. */
function succeeds(command, args) {
  const result = spawnSync(command, args, { stdio: "ignore" });
  return result.error === undefined && result.status === 0;
}

/** What this machine lacks to run the test, one description per capability. */
function missingCapabilities() {
  const missing = [];
  if (process.platform !== "linux" && process.platform !== "darwin") {
    // Elsewhere the app's config directory ignores HOME, so the run could
    // not be kept off the host.
    missing.push(`a supported platform (Linux or macOS; this is ${process.platform})`);
    return missing;
  }
  if (process.platform === "linux") {
    if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) {
      missing.push("a display (neither DISPLAY nor WAYLAND_DISPLAY is set; try xvfb-run)");
    }
    if (!succeeds("pkg-config", ["--exists", "webkit2gtk-4.1"])) {
      missing.push("the native webview (pkg-config finds no webkit2gtk-4.1)");
    }
  }
  if (!succeeds("cargo", ["--version"])) {
    missing.push("cargo, to build the app");
  }
  if (!existsSync(tauriCli) || !existsSync(wdioCli)) {
    missing.push("the desktop's node modules (run `pnpm install --frozen-lockfile` in apps/desktop)");
  }
  return missing;
}

class HarnessError extends Error {}

/** Run one step; `step` names it when it fails. */
function run(step, command, args, { cwd, env }) {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new HarnessError(`${step} exited with ${result.status ?? `signal ${result.signal}`}`);
  }
}

/** Cargo's target directory, wherever `CARGO_TARGET_DIR` or config puts it. */
function targetDirectory() {
  const result = spawnSync("cargo", ["metadata", "--locked", "--format-version", "1", "--no-deps"], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new HarnessError(`cargo metadata exited with ${result.status}`);
  return JSON.parse(result.stdout).target_directory;
}

/** Where the app writes `settings.json` when HOME and XDG_CONFIG_HOME are `dirs`'. */
function settingsPath(dirs) {
  const { identifier } = JSON.parse(readFileSync(path.join(desktopRoot, "src-tauri/tauri.conf.json"), "utf8"));
  const configDir =
    process.platform === "darwin"
      ? path.join(dirs.HOME, "Library", "Application Support", identifier)
      : path.join(dirs.XDG_CONFIG_HOME, identifier);
  return path.join(configDir, "settings.json");
}

/**
 * The environment `neb init` and the app run in: the caller's, with every
 * per-user directory moved inside `root`, the corpus pinned, and nothing
 * inherited that points either at another corpus or repository.
 */
function isolatedEnvironment(root, corpusRoot) {
  const dirs = {
    HOME: path.join(root, "home"),
    XDG_CONFIG_HOME: path.join(root, "config"),
    XDG_DATA_HOME: path.join(root, "data"),
    XDG_CACHE_HOME: path.join(root, "cache"),
    XDG_STATE_HOME: path.join(root, "state"),
  };
  for (const dir of Object.values(dirs)) mkdirSync(dir, { recursive: true, mode: 0o700 });
  const env = { ...process.env, ...dirs, NEBULA_ROOT: corpusRoot };
  delete env.OBSERVATORY_ROOT;
  for (const name of Object.keys(env)) {
    if (name.startsWith("GIT_")) delete env[name];
  }
  return { env, dirs };
}

const missing = missingCapabilities();
if (missing.length > 0) {
  for (const capability of missing) {
    if (required) console.error(`error: the webview test needs ${capability}, and REQUIRE_WEBVIEW=1 forbids skipping`);
    else console.log(`skipped: ${capability}`);
  }
  process.exit(required ? 1 : 0);
}

const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "nebula-csp-webview-"));
const corpusRoot = path.join(temporaryRoot, "corpus");
let exitCode = 0;
try {
  const { env: isolated, dirs } = isolatedEnvironment(temporaryRoot, corpusRoot);

  // Build with the real environment. `webdriver-test` is enabled here and
  // nowhere else: the embedded WebDriver listens on localhost with no Host or
  // Origin check, so it is compiled only into this debug build.
  run("cargo build (neb)", "cargo", ["build", "--locked", "--quiet", "--package", "neb"], {
    cwd: repoRoot,
    env: process.env,
  });
  run(
    "tauri build",
    process.execPath,
    [
      tauriCli,
      "build",
      "--debug",
      "--no-bundle",
      "--config",
      "src-tauri/tauri.conf.webdriver.json",
      "--features",
      "webdriver-test",
      "--",
      "--locked",
    ],
    { cwd: desktopRoot, env: process.env },
  );
  const debugDir = path.join(targetDirectory(), "debug");

  run("neb init", path.join(debugDir, "neb"), ["--root", corpusRoot, "init"], {
    cwd: temporaryRoot,
    env: isolated,
  });
  run("wdio", process.execPath, [wdioCli, "run", "wdio.conf.mjs"], {
    cwd: desktopRoot,
    env: { ...isolated, NEBULA_WEBVIEW_APP: path.join(debugDir, "nebula-desktop") },
  });

  // Positive evidence that the app ran inside the temporary root: its first
  // launch writes `settings.json`, and it must have written it there.
  const settings = settingsPath(dirs);
  if (!existsSync(settings)) {
    throw new HarnessError(`the app wrote no ${settings}: it did not use the temporary HOME/XDG directories`);
  }
} catch (error) {
  console.error(error instanceof HarnessError ? `error: ${error.message}` : error);
  exitCode = 1;
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

process.exitCode = exitCode;
