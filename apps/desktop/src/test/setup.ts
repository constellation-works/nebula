import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Without vitest globals, testing-library does not unmount between tests.
afterEach(cleanup);
