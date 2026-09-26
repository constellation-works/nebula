import assert from "node:assert/strict";

describe("production webview CSP", () => {
  it("blocks inline scripts while the UI and corpus IPC remain available", async () => {
    await browser.switchToWindow("main");

    const tablist = await browser.$('[role="tablist"]');
    await tablist.waitForDisplayed();

    const inboxTab = await browser.$('[role="tab"]:nth-child(1)');
    const graphTab = await browser.$('[role="tab"]:nth-child(2)');
    assert.equal(await inboxTab.getText(), "Inbox");
    assert.equal(await graphTab.getText(), "Graph");

    await browser.execute(() => {
      window.__nebulaCspViolations = [];
      window.__nebulaInlineScriptRan = false;
      window.addEventListener("securitypolicyviolation", (event) => {
        window.__nebulaCspViolations.push(event.effectiveDirective);
      });

      const script = document.createElement("script");
      script.textContent = "window.__nebulaInlineScriptRan = true;";
      document.head.append(script);
    });

    await browser.waitUntil(
      () =>
        browser.execute(() =>
          window.__nebulaCspViolations.some((directive) => directive.startsWith("script-src")),
        ),
      { timeout: 5_000, timeoutMsg: "the webview did not report blocking the inline script" },
    );
    assert.equal(await browser.execute(() => window.__nebulaInlineScriptRan), false);

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
});
