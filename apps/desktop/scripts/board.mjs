// Board (kanban) smoke test + screenshots in presenter mode: commitments at different stages land
// in the right columns, tiles move only when acts arrive, and the drawer acts on a tile.
// Usage: pnpm build && pnpm preview --port 4173 &   then   node scripts/board.mjs [outDir]
import { chromium } from "playwright-core";

const out = process.argv[2] ?? "screenshots";
const url = process.env.URL ?? "http://localhost:4173";
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome", args: ["--no-sandbox"] });
const page = await browser.newPage({ viewport: { width: 1600, height: 940 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
const shot = async (name) => {
  await page.waitForTimeout(400);
  await page.screenshot({ path: `${out}/${name}.png` });
  console.log("saved", name);
};

await page.goto(url);
await page.waitForSelector("nav");

async function openAgent(name) {
  await page.locator("nav button", { hasText: name }).first().click();
  const accept = page.getByRole("button", { name: /I understand/ });
  if (await accept.isVisible().catch(() => false)) await accept.click();
  if (!(await page.locator(".director").isVisible().catch(() => false))) await page.getByRole("button", { name: /Director/ }).click();
}
const start = (title) => page.locator(".director .scn", { hasText: title }).click();
const auto = page.locator(".director .auto input");
const done = () => page.locator(".director .controls", { hasText: "Done" }).waitFor({ timeout: 30000 });

await openAgent("Atlas");
// Closed: quick (autopilot to the end).
await start("Quick tool call");
await auto.check();
await done();
await auto.uncheck();
// Left at different stages (no autopilot).
await start("Silent performer");
await page.locator(".director .controls", { hasText: "Fast-forward" }).waitFor({ timeout: 15000 });
await start("Negotiation");
await start("Paid call");
await start("Verified work");
await start("Governance gate");
await openAgent("Rogue");
await start("Rogue agent");
// Let Gate endorse before the clock jumps: a late endorsement (outside the governance window)
// would correctly not count.
await page.waitForTimeout(9000);

// Fast-forward 10 min: the silent promise is now overdue (derived, never announced).
await openAgent("Atlas");
await page.locator(".director .clock .btn", { hasText: "+10 min" }).click();

await page.locator(".views button", { hasText: "Board" }).click();
const lane = (id) => page.locator(`.lane[data-lane="${id}"]`);
const tile = (laneId, tool) => lane(laneId).locator(".tile", { hasText: `/${tool}` });
await tile("negotiating", "negotiate").waitFor({ timeout: 10000 });
await tile("review", "paid").waitFor({ timeout: 10000 });
await tile("review", "verified").waitFor({ timeout: 10000 });
await tile("review", "governed").waitFor({ timeout: 10000 });
await tile("in_progress", "silent").locator(".late").waitFor({ timeout: 10000 });
await tile("closed", "quick").waitFor({ timeout: 10000 });
await tile("closed", "rogue").locator(".flag", { hasText: "equivocation" }).waitFor({ timeout: 10000 });
if (!(await tile("negotiating", "negotiate").locator(".status", { hasText: "Your move" }).count())) throw new Error("counter should need me");
await tile("review", "paid").locator(".money", { hasText: "21 sat" }).waitFor();
await shot("b1-board");
console.log("ok   lanes: negotiating, review ×3, in progress (overdue), closed");

// Needs-you filter.
await page.locator(".board .filter").click();
if (await lane("in_progress").locator(".tile").count()) throw new Error("filter shows tiles that don't need me");
await shot("b2-board-needs-you");
await page.locator(".board .filter").click();

// Drawer: satisfy the paid call from the board; the tile moves to Closed and shows the payment.
await tile("review", "paid").click();
await page.locator(".drawer .commitment").waitFor();
await shot("b3-board-drawer");
await page.locator(".drawer").getByRole("button", { name: "Satisfy & pay" }).click();
await page.locator(".drawer .x").click();
await tile("closed", "paid").waitFor({ timeout: 10000 });
await tile("closed", "paid").locator(".money", { hasText: "paid" }).waitFor({ timeout: 10000 });
console.log("ok   satisfy from the drawer moves the tile to Closed (paid)");

// Accept the counter from the drawer: Negotiating → In progress → Review.
await tile("negotiating", "negotiate").click();
await page.locator(".drawer").getByRole("button", { name: "Accept counter-offer" }).click();
await page.locator(".drawer .x").click();
await tile("review", "negotiate").waitFor({ timeout: 15000 });
console.log("ok   accept counter: negotiating → review");

// Open in chat.
await tile("review", "negotiate").click();
await page.locator(".drawer").getByRole("button", { name: /Open in chat/ }).click();
await page.locator(".director").waitFor();
console.log("ok   open in chat");

// Reset empties the board.
await page.locator(".director .reset").click();
await page.locator(".director .reset", { hasText: "Click again" }).click();
await page.locator(".views button", { hasText: "Board" }).click();
await page.locator(".board .empty").waitFor({ timeout: 5000 });
console.log("ok   reset empties the board");

await browser.close();
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
console.log("board ok");
