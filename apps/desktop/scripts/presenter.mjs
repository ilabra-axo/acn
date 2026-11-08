// Presenter-mode smoke test + screenshots: runs every ACN scenario in the browser with the
// Director's autopilot and checks each one reaches its intended end state.
// Usage: pnpm build && pnpm preview --port 4173 &   then   node scripts/presenter.mjs [outDir]
import { chromium } from "playwright-core";

const out = process.argv[2] ?? "screenshots";
const url = process.env.URL ?? "http://localhost:4173";
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome", args: ["--no-sandbox"] });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
const shot = async (name) => {
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${out}/${name}.png` });
  console.log("saved", name);
};

const EXPECT = {
  quick: "Satisfied", paid: "Satisfied", negotiate: "Satisfied", rework: "Satisfied", verified: "Satisfied",
  dispute: "Satisfied", governed: "Satisfied", silent: "Withdrawn", rogue: "Declined",
  judged: "Settled", injected: "Rejected", "judged-auto": "Satisfied", contingent: "Settled",
};
const TITLES = {
  quick: "Quick tool call", paid: "Paid call (Cashu)", negotiate: "Negotiation", rework: "Rework loop",
  verified: "Verified work", dispute: "Dispute & ruling", governed: "Governance gate", silent: "Silent performer", rogue: "Rogue agent",
  judged: "Judged, verifier releases", injected: "Judge prompt injection", "judged-auto": "Judged, you settle",
  contingent: "Contingent (encrypted until paid)",
};

await page.goto(url);
await page.waitForSelector("nav");

async function openAgent(name) {
  await page.locator("nav button", { hasText: name }).first().click();
  const accept = page.getByRole("button", { name: /I understand/ });
  if (await accept.isVisible().catch(() => false)) await accept.click();
  if (!(await page.locator(".director").isVisible().catch(() => false))) {
    await page.getByRole("button", { name: /Director/ }).click();
  }
}

async function run(id, { manual = [] } = {}) {
  const agent = id === "rogue" ? "Rogue" : "Atlas";
  await openAgent(agent);
  await page.locator(".director .scn", { hasText: TITLES[id] }).click();
  const card = page.locator(".commitment").last();
  for (const step of manual) {
    await page.waitForSelector(step.waitFor, { timeout: 15000 });
    if (step.shot) await shot(step.shot);
    if (step.click) await page.locator(".director .controls .btn.primary").click();
  }
  const auto = page.locator(".director .auto input");
  if (!(await auto.isChecked())) await auto.check();
  await card.locator(`.status`, { hasText: EXPECT[id] }).waitFor({ timeout: 30000 });
  await page.locator(".director .controls", { hasText: "Done" }).waitFor({ timeout: 10000 });
  await auto.uncheck();
  console.log(`ok   ${id} → ${EXPECT[id]}`);
  return card;
}

// Narrated captures for the key scenarios.
await run("negotiate", {
  manual: [{ waitFor: ".commitment .counter", shot: "p1-negotiate-counter", click: true }],
});
await run("verified", {
  manual: [
    { waitFor: ".commitment .conditions .nc", shot: null },
    { waitFor: ".director >> text=Reject the tests", shot: "p2-verified-evidence", click: true },
    { waitFor: ".commitment .notes >> text=contradicted_rejection", shot: "p3-verified-contradicted" },
  ],
});
await run("dispute", {
  manual: [{ waitFor: ".director >> text=Reject the concepts", click: true }, { waitFor: "text=With the arbiter", shot: "p4-dispute-arbiter" }],
});
await page.locator(".commitment").last().getByRole("button", { name: /signed events/ }).click();
await shot("p5-dispute-ruled-timeline");
await run("paid");
await shot("p6-paid-settled");

// Money moved in the simulated mint: 100 - 50 (dispute, ruled for Atlas) - 21 (paid).
const walletBtn = page.locator(".wallet-btn");
await walletBtn.filter({ hasText: "29 sat" }).waitFor({ timeout: 10000 });
await page.locator(".commitment").last().locator(".token", { hasText: "redeemed by Atlas" }).waitFor({ timeout: 10000 });
await page.locator(".director .bal", { hasText: "Atlas" }).filter({ hasText: "71 sat" }).waitFor({ timeout: 10000 });
console.log("ok   wallet: you 29 sat, Atlas 71 sat, token redeemed");

// Refund path: reject a paid result (no arbiter, no rework → closed unsatisfied), fast-forward
// past the locktime, reclaim the 21 sat.
await page.locator(".director .scn", { hasText: TITLES.paid }).click();
const paidCard = page.locator(".commitment").last();
await paidCard.getByRole("button", { name: "Reject…" }).click({ timeout: 15000 });
await paidCard.locator("form .field").last().fill("Not what I asked for");
await paidCard.getByRole("button", { name: "Reject", exact: true }).click();
await paidCard.locator(".token", { hasText: "locked at the mint" }).waitFor({ timeout: 10000 });
await walletBtn.filter({ hasText: "8 sat" }).waitFor({ timeout: 10000 });
const refundBtn = paidCard.locator(".settle .refund");
if (!(await refundBtn.isDisabled())) throw new Error("refund must wait for the locktime");
await shot("p6b-paid-rejected-refund-waits");
// The locktime is deadline + review window + 24 h margin ≈ 25 h out.
for (let i = 0; i < 26; i++) await page.locator(".director .clock .btn", { hasText: "+1 h" }).click();
await refundBtn.click({ timeout: 10000 }); // waits until enabled
await paidCard.locator(".token", { hasText: "refunded to you" }).waitFor({ timeout: 10000 });
await walletBtn.filter({ hasText: "29 sat" }).waitFor({ timeout: 10000 });
await page.locator(".director .clock .btn", { hasText: "Reset" }).click();
await walletBtn.click();
await shot("p6c-wallet");
await page.locator(".wallet .x").click();
console.log("ok   refund after the locktime");
await run("governed", { manual: [{ waitFor: "text=pending", shot: "p7-governed-pending" }] });
await run("silent", { manual: [{ waitFor: "text=Fast-forward", shot: null, click: true }, { waitFor: "text=Lapsed", shot: "p8-silent-lapsed" }] });
await run("rework");
await run("quick");
await run("rogue");
await shot("p9-rogue-equivocation");

// Reset demo (two-step confirm): no commitments left, the simulated mint back to 100 sat.
await openAgent("Atlas");
await page.locator(".director .reset").click();
await page.locator(".director .reset", { hasText: "Click again" }).click();
await page.locator(".wallet-btn").filter({ hasText: "100 sat" }).waitFor({ timeout: 5000 });
if ((await page.locator(".commitment").count()) !== 0) throw new Error("reset left commitments");
console.log("ok   reset demo");

// The MVP: pay only if two LLM judges score the rubric ≥ θ (CR-001), settled without you (CR-002).
await walletBtn.click();
await page.getByRole("button", { name: "Top up 100 test sats" }).click();
await walletBtn.filter({ hasText: "200 sat" }).waitFor({ timeout: 10000 });
await page.locator(".wallet .x").click();
await run("judged", { manual: [{ waitFor: ".commitment >> text=opinion", shot: "p10-judged-terms" }] });
await shot("p11-judged-released");
await run("injected");
await page.locator(".commitment").last().locator("text=injection_detected").first().waitFor({ timeout: 10000 });
await shot("p12-injected");
await run("judged-auto");
await run("contingent", { manual: [{ waitFor: ".commitment >> text=Encrypted delivery", shot: "p13-contingent-sealed" }] });
await page.locator(".commitment").last().locator("text=Parser module").first().waitFor({ timeout: 10000 });
await shot("p14-contingent-opened");
// 200 - 60 (judged) - 60 (injected, still locked) - 40 (judged-auto) - 30 (contingent).
await walletBtn.filter({ hasText: "10 sat" }).waitFor({ timeout: 10000 });
await page.locator(".director .bal", { hasText: "Atlas" }).filter({ hasText: "130 sat" }).waitFor({ timeout: 10000 });
console.log("ok   judged MVP: you 10 sat (60 locked), Atlas 130 sat");

await browser.close();
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
console.log("all scenarios ok");
