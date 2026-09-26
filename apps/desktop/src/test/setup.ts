import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// Without vitest globals, testing-library does not unmount between tests.
afterEach(cleanup);

// `waitFor` and `findBy*` give up after 1 s by default, which a loaded host
// can spend before the ELK layout resolves. The wait stays bounded, so a
// node that never appears still fails; `testTimeout` in vite.config.ts sits
// above it so the wait's own error, not a bare timeout, is what gets reported.
configure({ asyncUtilTimeout: 10_000 });
