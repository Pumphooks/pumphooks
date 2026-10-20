#!/usr/bin/env node
// Visual check for the web app: node scripts/shot.mjs <url> <out-prefix> [w=1440] [h=900] [scrollYs=0] [mouse=x,y]
// Uses the locally installed Google Chrome (no download).
import { chromium } from "playwright-core";

const [url, out, w = "1440", h = "900", ys = "0", mouse] = process.argv.slice(2);
const browser = await chromium.launch({
  executablePath: process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  args: ["--use-angle=metal", "--enable-gpu", "--ignore-gpu-blocklist"],
});
const page = await browser.newPage({ viewport: { width: Number(w), height: Number(h) }, deviceScaleFactor: 1 });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
await page.goto(url, { waitUntil: "networkidle" });
await page.waitForTimeout(1600);
for (const [i, y] of ys.split(",").entries()) {
  await page.evaluate((y) => window.scrollTo(0, Number(y)), y);
  if (mouse) {
    const [mx, my] = mouse.split(",").map(Number);
    await page.mouse.move(mx - 40, my - 30);
    await page.mouse.move(mx, my, { steps: 8 });
  }
  await page.waitForTimeout(1400);
  const file = `${out}-${i}.jpg`;
  await page.screenshot({ path: file, type: "jpeg", quality: 82 });
  console.log(file);
}
if (errors.length) console.log("PAGE ERRORS:\n" + errors.join("\n"));
await browser.close();
