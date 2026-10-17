import { defineConfig, devices } from "@playwright/test";

// The site under test. run.sh starts the release binary on this port; the tests themselves run in
// the official Playwright container, so fonts, anti-aliasing and browsers are the same on every
// machine and the screenshot baselines compare pixel for pixel.
const baseURL = process.env.BASE_URL ?? "http://localhost:8090";

const sizes = {
  phone: { width: 375, height: 812 },
  tablet: { width: 820, height: 1180 },
  desktop: { width: 1440, height: 900 },
};

const engines = [
  { name: "chromium", use: devices["Desktop Chrome"] },
  { name: "firefox", use: devices["Desktop Firefox"] },
  { name: "webkit", use: devices["Desktop Safari"] },
] as const;

export default defineConfig({
  testDir: "tests",
  fullyParallel: true,
  // Three engines at once exhaust memory with more workers; 6 keeps every browser stable.
  workers: process.env.CI ? 2 : 6,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "report" }]],
  outputDir: "results",
  snapshotPathTemplate: "snapshots/{projectName}/{arg}{ext}",
  expect: {
    // Pixel perfect: only sub-pixel anti-aliasing noise is tolerated.
    toHaveScreenshot: { maxDiffPixelRatio: 0.0005, threshold: 0.15, animations: "disabled", scale: "css" },
  },
  use: { baseURL, trace: "retain-on-failure", screenshot: "only-on-failure" },
  projects: engines.flatMap((engine) =>
    Object.entries(sizes).map(([size, viewport]) => ({
      name: `${engine.name}-${size}`,
      use: { ...engine.use, viewport, deviceScaleFactor: 1, hasTouch: size === "phone" && engine.name !== "firefox", isMobile: false },
    })),
  ),
});
