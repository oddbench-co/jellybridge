import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { PAGES, DOCS } from "./pages";

/** Loads a page and fails on anything a visitor would notice: errors, broken requests, missing images. */
async function load(page: Page, path: string) {
  const problems: string[] = [];
  // A 404 page reports its own status as a console error; that one is expected.
  page.on("console", (m) => { if (m.type() === "error" && !(path.includes("does-not-exist") && m.text().includes("404"))) problems.push(`console: ${m.text()}`); });
  page.on("pageerror", (e) => problems.push(`script: ${e.message}`));
  page.on("requestfailed", (r) => problems.push(`request failed: ${r.url()} ${r.failure()?.errorText}`));
  page.on("response", (r) => { if (r.status() >= 400 && !r.url().includes("does-not-exist")) problems.push(`HTTP ${r.status()}: ${r.url()}`); });
  const response = await page.goto(path, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);
  return { response, problems };
}

/** Lazy images load only near the viewport; scroll through the page so all of them are fetched. */
async function loadAllImages(page: Page) {
  await page.evaluate(async () => {
    for (const img of Array.from(document.images)) img.loading = "eager";
    await Promise.all(Array.from(document.images).map((img) => img.decode().catch(() => {})));
  });
}

for (const p of PAGES) {
  test.describe(p.name, () => {
    test("renders cleanly", async ({ page }) => {
      const { response, problems } = await load(page, p.path);
      expect(response?.status()).toBe(p.name === "not-found" ? 404 : 200);
      await expect(page.locator("h1")).toHaveCount(1);
      await expect(page.locator("h1")).toContainText(p.text);
      await loadAllImages(page);

      const broken = await page.evaluate(() => Array.from(document.images).filter((i) => !i.complete || i.naturalWidth === 0).map((i) => i.src));
      expect(broken, "images that did not load").toEqual([]);
      // Icons come from one sprite; a wrong id renders nothing, so check each symbol exists.
      const icons = await page.evaluate(() => Array.from(new Set(Array.from(document.querySelectorAll("use")).map((u) => u.getAttribute("href")!.split("#")[1]))));
      const sprite = await (await page.request.get(await page.evaluate(() => document.querySelector("use")?.getAttribute("href")?.split("#")[0] ?? "/static/img/icons.svg"))).text();
      for (const id of icons) expect(sprite, `icon #${id}`).toContain(`id="${id}"`);

      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      expect(overflow, "page scrolls sideways").toBeLessThanOrEqual(0);
      expect(problems).toEqual([]);
    });

    test("is accessible", async ({ page }) => {
      await load(page, p.path);
      const results = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa", "best-practice"]).analyze();
      const serious = results.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      expect(serious.map((v) => `${v.id}: ${v.help} (${v.nodes.map((n) => n.target.join(" ")).join(", ")})`)).toEqual([]);
    });

    test("matches the approved look", async ({ page }) => {
      await load(page, p.path);
      await loadAllImages(page);
      await expect(page).toHaveScreenshot(`${p.name}.png`, { fullPage: true });
    });
  });
}

test("every doc page renders with its sidebar entry current", async ({ page }) => {
  for (const slug of DOCS) {
    const { response, problems } = await load(page, `/docs/${slug}`);
    expect(response?.status(), slug).toBe(200);
    await expect(page.locator(`.docs__nav a[href="/docs/${slug}"]`)).toHaveAttribute("aria-current", "page");
    expect(problems, slug).toEqual([]);
  }
});

test("touch targets are at least 24 px", async ({ page }) => {
  for (const p of PAGES) {
    await load(page, p.path);
    const small = await page.evaluate(() =>
      Array.from(document.querySelectorAll<HTMLElement>("a, button, summary, input"))
        // Links inside running text are exempt from the target size rule (WCAG 2.5.8, "inline").
        .filter((el) => el.offsetParent !== null && !el.closest("p, td, dd, .prose, .packages"))
        .map((el) => ({ el, r: el.getBoundingClientRect() }))
        .filter(({ r }) => r.height < 24 || r.width < 24)
        .map(({ el, r }) => `${el.textContent?.trim() || el.getAttribute("aria-label")} ${Math.round(r.width)}x${Math.round(r.height)}`),
    );
    expect(small, p.path).toEqual([]);
  }
});
