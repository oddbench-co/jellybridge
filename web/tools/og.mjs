// Renders static/img/og.jpg, the 1200x630 social preview card: the home hero, without the header.
// Needs the site running (cargo run --release) and the e2e dependencies (cd e2e && npm ci).
//
//   node tools/og.mjs [http://localhost:8080]
import { chromium } from "../e2e/node_modules/@playwright/test/index.mjs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const base = process.argv[2] ?? "http://localhost:8080";
const root = fileURLToPath(new URL("..", import.meta.url));
const browser = await chromium.launch();
// Without the script the button reads "Download" rather than naming one system.
const page = await browser.newPage({ viewport: { width: 1200, height: 630 }, deviceScaleFactor: 1, javaScriptEnabled: false });
await page.goto(base + "/", { waitUntil: "networkidle" });
await page.evaluate(() => document.fonts.ready);
// The card is for everyone: the button says "Download", as it does before the script picks a
// system. The header and badges are hidden and the hero is centred in the frame.
await page.evaluate(() => {
  document.querySelector("[data-os-label]").textContent = "Download";
  for (const el of document.querySelectorAll(".site-header, .badges")) el.style.display = "none";
  document.querySelector(".hero").style.paddingBlock = "78px 0";
});
await page.screenshot({ path: root + "target/og.png" });
await browser.close();
execFileSync("magick", [root + "target/og.png", "-quality", "86", "-sampling-factor", "4:2:0", "-strip", root + "static/img/og.jpg"]);
console.log("wrote static/img/og.jpg");
