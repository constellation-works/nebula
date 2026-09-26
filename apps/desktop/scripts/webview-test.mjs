import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(desktopRoot, "../..");
const temporaryRoot = mkdtempSync(path.join(os.tmpdir(), "nebula-csp-webview-"));
const corpusRoot = path.join(temporaryRoot, "corpus");
const env = { ...process.env, NEBULA_ROOT: corpusRoot };

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const error = new Error(`${command} exited with status ${result.status ?? "unknown"}`);
    error.exitCode = result.status ?? 1;
    throw error;
  }
}

let exitCode = 0;
try {
  run("cargo", ["run", "--quiet", "--package", "neb", "--", "--root", corpusRoot, "init"], repoRoot);
  run(
    process.execPath,
    [
      path.join(desktopRoot, "node_modules/@tauri-apps/cli/tauri.js"),
      "build",
      "--debug",
      "--no-bundle",
      "--config",
      "src-tauri/tauri.conf.webdriver.json",
      "--features",
      "webdriver-test",
    ],
    desktopRoot,
  );
  run(
    process.execPath,
    [path.join(desktopRoot, "node_modules/@wdio/cli/bin/wdio.js"), "run", "wdio.conf.mjs"],
    desktopRoot,
  );
} catch (error) {
  console.error(error);
  exitCode = error.exitCode ?? 1;
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

process.exitCode = exitCode;
