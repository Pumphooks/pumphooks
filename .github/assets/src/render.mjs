// Rasterises the README SVGs at a few points in their animation, for review.
//   node .github/assets/src/render.mjs [chromium|webkit]
import { createRequire } from "node:module";
import { readFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../../package.json"));
const pw = require("playwright-core");
const engine = process.argv[2] ?? "chromium";
const out = join(here, "_render");
mkdirSync(out, { recursive: true });

const browser = await pw[engine].launch(engine === "chromium" ? { channel: "chrome" } : {});
const page = await browser.newPage({ viewport: { width: 1280, height: 1000 }, deviceScaleFactor: 1 });
for (const name of ["hero", "system", "route"]) {
  const svg = readFileSync(join(here, "..", `${name}.svg`), "utf8");
  const src = "data:image/svg+xml;base64," + Buffer.from(svg).toString("base64");
  await page.setContent(`<body style="margin:0;background:#fff"><img src="${src}" style="display:block"></body>`);
  await page.waitForTimeout(400);
  for (const t of [1, 4]) {
    await page.waitForTimeout(t === 1 ? 600 : 3000);
    const img = await page.$("img");
    await img.screenshot({ path: join(out, `${name}-${engine}-${t}s.png`) });
  }
}
await browser.close();
console.log("rendered to", out);
