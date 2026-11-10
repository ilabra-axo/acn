// Drives the browser preview (mock backend) through the main flows and saves screenshots.
// Usage: pnpm build && pnpm preview --port 4173 &  then
//        NODE_PATH=<dir with playwright-core> node scripts/screenshots.mjs [outDir]
import { chromium } from "playwright-core";

const out = process.argv[2] ?? "screenshots";
const url = process.env.URL ?? "http://localhost:4173";
const browser = await chromium.launch({
  executablePath: process.env.CHROME ?? "/usr/bin/google-chrome",
  args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1180, height: 760 }, deviceScaleFactor: 1 });
page.on("pageerror", (e) => console.error("pageerror:", e.message));
page.on("console", (m) => m.type() === "error" && console.error("console:", m.text()));

const shot = async (name) => {
  await page.waitForTimeout(250);
  await page.screenshot({ path: `${out}/${name}.png` });
  console.log("saved", name);
};

await page.goto(url);
await page.waitForSelector("nav");
await shot("01-welcome");

await page.getByRole("button", { name: /Echo agent/ }).click();
await shot("02-agent-consent");

await page.getByRole("button", { name: /I understand/ }).click();
await page.locator(".composer textarea").fill("/");
await shot("03-slash-autocomplete");

await page.locator(".composer textarea").fill("/status");
await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.waitForSelector("table");
await page.locator(".composer textarea").fill("/form");
await page.keyboard.press("Enter");
await page.keyboard.press("Enter");
await page.waitForSelector(".messages form input.field");
await shot("04-table-and-form");

await page.locator(".messages form input.field").first().fill("Launch checklist");
await page.locator(".messages form textarea").fill("Relays up, bridge healthy, ship it.");
await page.getByRole("button", { name: "Create card" }).click();
// The host shows what the form will actually invoke, and asks (A17).
await page.locator(".confirm").waitFor({ timeout: 5000 });
await shot("05a-action-confirm");
await page.locator(".confirm").getByRole("button", { name: "Send", exact: true }).click();
await page.waitForSelector("text=Launch checklist >> nth=0");
await page.waitForTimeout(700);
await page.getByRole("button", { name: "Press me" }).last().click();
await page.locator(".confirm").getByRole("button", { name: "Send", exact: true }).click();
await page.waitForTimeout(700);
await shot("05-form-submitted-card");

// ACN commitment (presenter cast): request → signed REPORT → requester SATISFY.
await page.locator("nav button", { hasText: "Atlas" }).click();
await shot("02b-acn-agent-consent");
await page.getByRole("button", { name: /I understand/ }).click();
await page.locator(".composer textarea").fill("/quick a status report");
await page.keyboard.press("Escape");
await page.locator(".composer textarea").fill("/quick a status report");
await page.locator(".composer .send").click();
await page.waitForSelector("text=Your review");
await shot("07-commitment-reported");
await page.getByRole("button", { name: "Accept result" }).click();
await page.waitForSelector(".status >> text=Satisfied");
await shot("08-commitment-satisfied");

await page.getByRole("button", { name: /Alice/ }).click();
await shot("06-human-chat-location");

await browser.close();
