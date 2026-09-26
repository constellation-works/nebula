import assert from "node:assert/strict";

/**
 * Record every CSP violation the page reports from now on, with the policy
 * that was violated. Idempotent: a second call only clears the record.
 */
async function watchViolations() {
  await browser.execute(() => {
    window.__nebulaCspViolations = [];
    if (window.__nebulaCspWatching) return;
    window.__nebulaCspWatching = true;
    window.addEventListener("securitypolicyviolation", (event) => {
      window.__nebulaCspViolations.push({
        directive: event.effectiveDirective,
        policy: event.originalPolicy,
      });
    });
  });
}

/** Wait for a reported violation of `directive`, and return it. */
async function violationOf(directive, timeoutMsg) {
  let found;
  await browser.waitUntil(
    async () => {
      const violations = await browser.execute(() => window.__nebulaCspViolations);
      found = violations.find((violation) => violation.directive.startsWith(directive));
      return found !== undefined;
    },
    { timeout: 5_000, timeoutMsg },
  );
  return found;
}

/** The sources a policy lists for one directive. */
function sources(policy, directive) {
  const entry = policy
    .split(";")
    .map((part) => part.trim().split(/\s+/))
    .find(([name]) => name === directive);
  return entry === undefined ? [] : entry.slice(1);
}

describe("production webview CSP", () => {
  it("blocks inline scripts while the UI and corpus IPC remain available", async () => {
    await browser.switchToWindow("main");

    const tablist = await browser.$('[role="tablist"]');
    await tablist.waitForDisplayed();

    const inboxTab = await browser.$('[role="tab"]:nth-child(1)');
    const graphTab = await browser.$('[role="tab"]:nth-child(2)');
    assert.equal(await inboxTab.getText(), "Inbox");
    assert.equal(await graphTab.getText(), "Graph");

    await watchViolations();
    await browser.execute(() => {
      window.__nebulaInlineScriptRan = false;
      const script = document.createElement("script");
      script.textContent = "window.__nebulaInlineScriptRan = true;";
      document.head.append(script);
    });

    const violation = await violationOf(
      "script-src",
      "the webview did not report blocking the inline script",
    );
    assert.equal(await browser.execute(() => window.__nebulaInlineScriptRan), false);
    // Tauri adds a hash of every bundled script to `script-src`, and a
    // webview ignores 'unsafe-inline' next to a hash, so the block above
    // holds even if the configured policy allowed inline script. The policy
    // itself must not: it is what takes effect the day those hashes go.
    const scriptSources = sources(violation.policy, "script-src");
    assert.ok(scriptSources.includes("'self'"), `no script-src in the reported policy: ${violation.policy}`);
    assert.ok(
      !scriptSources.includes("'unsafe-inline'") && !scriptSources.includes("'unsafe-eval'"),
      `the production script-src allows inline script or eval: ${scriptSources.join(" ")}`,
    );

    await graphTab.click();
    await browser.waitUntil(
      async () => {
        const selectedTab = await browser.$('[role="tab"][aria-selected="true"]');
        return (await selectedTab.getText()) === "Graph";
      },
      { timeout: 5_000, timeoutMsg: "the Graph tab did not become active" },
    );

    const graph = await browser.executeAsync((done) => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (typeof invoke !== "function") {
        done({ ok: false, error: "Tauri IPC bridge is unavailable" });
        return;
      }
      invoke("graph").then(
        (value) => done({ ok: true, value }),
        (error) => done({ ok: false, error: String(error) }),
      );
    });
    assert.equal(graph.ok, true, graph.error);
    assert.deepEqual(graph.value.nodes, []);
    assert.deepEqual(graph.value.edges, []);
  });

  // Opening a node must not contact a host the operator never chose
  // (STD-05 §R20). The panel renders remote images as links; this is the
  // policy behind it, for any `<img>` that still reaches the page.
  it("blocks a remote image in the production CSP", async () => {
    await browser.switchToWindow("main");
    await watchViolations();
    await browser.execute(() => {
      const image = document.createElement("img");
      image.src = "https://example.invalid/x.png";
      document.body.append(image);
    });

    const violation = await violationOf(
      "img-src",
      "the webview did not report blocking the remote image",
    );
    assert.match(violation.directive, /^img-src/);
    await browser.execute(() => document.querySelector('img[src="https://example.invalid/x.png"]')?.remove());
  });
});
