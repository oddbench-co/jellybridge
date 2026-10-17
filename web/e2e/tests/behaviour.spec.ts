import { test, expect } from "@playwright/test";

const systems = [
  { os: "windows", label: "Windows", ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36" },
  { os: "macos", label: "macOS", ua: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15" },
  { os: "linux", label: "Linux", ua: "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0" },
];

test.describe("download button picks the visitor's system", () => {
  for (const s of systems) {
    test(s.label, async ({ browser, browserName }) => {
      // Chromium reports navigator.userAgentData from the real host, so only the user agent is faked where it decides.
      test.skip(browserName === "chromium" && s.os !== "linux", "userAgentData follows the host in Chromium");
      const context = await browser.newContext({ userAgent: s.ua });
      const page = await context.newPage();
      await page.goto("/");
      const button = page.locator("[data-os-download]");
      await expect(button).toHaveText(`Download for ${s.label}`);
      await expect(button).toHaveAttribute("href", `/download#${s.os}`);
      await button.click();
      await expect(page).toHaveURL(new RegExp(`/download#${s.os}$`));
      await expect(page.locator(`#${s.os} .platform__yours`)).toBeVisible();
      await expect(page.locator(".platform__yours:visible")).toHaveCount(1);
      await context.close();
    });
  }

  test("phones get the plain download link", async ({ browser }) => {
    const context = await browser.newContext({ userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1" });
    const page = await context.newPage();
    await page.goto("/");
    await expect(page.locator("[data-os-download]")).toHaveText("Download");
    await expect(page.locator("[data-os-download]")).toHaveAttribute("href", "/download");
    await context.close();
  });
});

test.describe("FAQ", () => {
  test("filters questions as you type, and says when nothing matches", async ({ page }) => {
    await page.goto("/faq");
    const search = page.getByRole("searchbox", { name: "Search questions" });
    await expect(search).toBeVisible();
    const items = page.locator(".faq details");
    const total = await items.count();
    expect(total).toBeGreaterThanOrEqual(10);

    await search.fill("telemetry");
    await expect(page.locator(".faq details:visible")).toHaveCount(1);
    await expect(page.locator(".faq details:visible summary")).toHaveText("Is there any telemetry?");

    await search.fill("direct play");
    await expect(page.locator(".faq details:visible").first()).toBeVisible();

    await search.fill("zzzz nothing like this");
    await expect(page.locator(".faq details:visible")).toHaveCount(0);
    await expect(page.locator("[data-faq-empty]")).toBeVisible();

    await search.fill("");
    await expect(page.locator(".faq details:visible")).toHaveCount(total);
    await expect(page.locator("[data-faq-empty]")).toBeHidden();
  });

  test("answers open and close from the keyboard", async ({ page }) => {
    await page.goto("/faq");
    const first = page.locator(".faq details").first();
    await first.locator("summary").focus();
    await page.keyboard.press("Enter");
    await expect(first).toHaveAttribute("open", "");
    await expect(first.locator("p")).toBeVisible();
    await page.keyboard.press("Enter");
    await expect(first).not.toHaveAttribute("open", "");
  });
});

test.describe("screenshot carousel", () => {
  test("arrows and thumbnails move between slides without scrolling the page", async ({ page }, info) => {
    await page.goto("/screenshots");
    const thumbs = page.locator("[data-carousel-thumb]");
    await expect(thumbs.first()).toHaveAttribute("aria-current", "true");
    // Bring the thumbnails on screen first: the click itself must not scroll the page.
    await thumbs.nth(2).scrollIntoViewIfNeeded();
    const pageY = await page.evaluate(() => scrollY);

    await thumbs.nth(2).click();
    await expect(thumbs.nth(2)).toHaveAttribute("aria-current", "true");
    await expect(thumbs.first()).toHaveAttribute("aria-current", "false");
    expect(await page.evaluate(() => scrollY)).toBe(pageY);

    if (!info.project.name.endsWith("phone")) {
      const next = page.getByRole("button", { name: "Next screenshot" });
      const prev = page.getByRole("button", { name: "Previous screenshot" });
      await next.click();
      await expect(thumbs.nth(3)).toHaveAttribute("aria-current", "true");
      await expect(next).toBeDisabled();
      await prev.click();
      await expect(thumbs.nth(2)).toHaveAttribute("aria-current", "true");
    }

    await page.locator(".carousel__track").focus();
    await page.keyboard.press("ArrowLeft");
    await expect(thumbs.nth(1)).toHaveAttribute("aria-current", "true");
  });
});

test.describe("keyboard and navigation", () => {
  test("the first Tab reaches the skip link, which jumps to the content", async ({ page }) => {
    await page.goto("/features");
    await page.keyboard.press("Tab");
    const skip = page.getByRole("link", { name: "Skip to content" });
    await expect(skip).toBeFocused();
    await expect(skip).toBeInViewport();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/#main$/);
  });

  test("the main navigation reaches every section and marks the current one", async ({ page }) => {
    await page.goto("/");
    for (const [name, path] of [["Features", "/features"], ["Screenshots", "/screenshots"], ["Download", "/download"], ["Docs", "/docs"]]) {
      await page.getByRole("navigation", { name: "Main" }).getByRole("link", { name, exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`${path}$`));
      await expect(page.getByRole("navigation", { name: "Main" }).getByRole("link", { name, exact: true })).toHaveAttribute("aria-current", "page");
    }
  });

  test("docs pages link to each other in order", async ({ page }) => {
    await page.goto("/docs");
    await page.getByRole("link", { name: /Next\s*Installation/ }).click();
    await expect(page).toHaveURL(/\/docs\/installation$/);
    await page.getByRole("link", { name: /Previous\s*Getting started/ }).click();
    await expect(page).toHaveURL(/\/docs$/);
  });

  test("a missing page says so and offers a way back", async ({ page }) => {
    const response = await page.goto("/no/such/page");
    expect(response?.status()).toBe(404);
    await page.getByRole("link", { name: "Go to the homepage" }).click();
    await expect(page).toHaveURL(/\/$/);
  });
});

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("everything still works, and nothing needs the script", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("[data-os-download]")).toHaveText("Download");
    await expect(page.locator("[data-os-download]")).toHaveAttribute("href", "/download");

    await page.goto("/faq");
    await expect(page.locator("[data-faq-filter]")).toBeHidden();
    await page.locator(".faq summary").first().click();
    await expect(page.locator(".faq details").first().locator("p")).toBeVisible();

    await page.goto("/screenshots");
    await expect(page.locator(".carousel__arrow")).toHaveCount(2);
    await expect(page.locator(".carousel__arrow").first()).toBeHidden();
    await page.locator("[data-carousel-thumb]").nth(3).click();
    await expect(page).toHaveURL(/#shot-player$/);
  });
});
