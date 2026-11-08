// Records a captioned screencast of the ACN demo (presenter mode) with Playwright.
// Usage: pnpm build && pnpm preview --port 4173 &   then   node scripts/screencast.mjs <outDir>
// Produces <outDir>/*.webm (convert with ffmpeg; see docs/DEMO.md).
import { chromium } from "playwright-core";

const out = process.argv[2] ?? "screencast";
const url = process.env.URL ?? "http://localhost:4173";
const W = 1440, H = 900;
const browser = await chromium.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome", args: ["--no-sandbox"] });
const ctx = await browser.newContext({ viewport: { width: W, height: H }, recordVideo: { dir: out, size: { width: W, height: H } } });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));

const sleep = (ms) => page.waitForTimeout(ms);

// --- overlays ---
async function installOverlays() {
  await page.addStyleTag({ content: `
    #sc-cap{position:fixed;left:300px;right:320px;bottom:118px;display:flex;justify-content:center;pointer-events:none;z-index:9999;transition:opacity .35s}
    #sc-cap>div{max-width:760px;background:rgba(17,17,28,.88);color:#fff;border-radius:14px;padding:12px 18px;font:15px/1.45 Inter,system-ui,sans-serif;box-shadow:0 10px 30px rgba(0,0,0,.25)}
    #sc-cap b{color:#fbbf24;font-weight:650}
    #sc-cap small{display:block;color:#a5b4fc;font-size:12px;letter-spacing:.04em;text-transform:uppercase;margin-bottom:3px}
    #sc-card{position:fixed;inset:0;z-index:10000;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:14px;
      background:linear-gradient(135deg,#312e81,#1e3a8a);color:#fff;font-family:Inter,system-ui,sans-serif;transition:opacity .5s;text-align:center;padding:40px}
    #sc-card h1{font-size:46px;margin:0;letter-spacing:-.02em} #sc-card p{font-size:21px;margin:0;opacity:.85;max-width:900px;line-height:1.45}
    #sc-card .tag{font:13px ui-monospace,monospace;opacity:.7;letter-spacing:.06em}
    #sc-click{position:fixed;width:34px;height:34px;margin:-17px 0 0 -17px;border-radius:50%;border:3px solid #f59e0b;z-index:10001;pointer-events:none;opacity:0;transition:opacity .2s}
  ` });
  await page.evaluate(() => {
    for (const id of ["sc-cap", "sc-click"]) {
      const d = document.createElement("div");
      d.id = id;
      d.style.opacity = "0";
      document.body.appendChild(d);
    }
  });
}

async function caption(label, html, hold = 0) {
  await page.evaluate(([l, h]) => {
    const c = document.getElementById("sc-cap");
    c.innerHTML = `<div>${l ? `<small>${l}</small>` : ""}${h}</div>`;
    c.style.opacity = "1";
  }, [label, html]);
  if (hold) await sleep(hold);
}
const hideCaption = () => page.evaluate(() => (document.getElementById("sc-cap").style.opacity = "0"));

async function card(title, text, tag = "", hold = 3800) {
  await page.evaluate(([t, x, g]) => {
    const d = document.createElement("div");
    d.id = "sc-card";
    d.innerHTML = `${g ? `<div class="tag">${g}</div>` : ""}<h1>${t}</h1><p>${x}</p>`;
    d.style.opacity = "0";
    document.body.appendChild(d);
    requestAnimationFrame(() => (d.style.opacity = "1"));
  }, [title, text, tag]);
  await sleep(hold);
  await page.evaluate(() => {
    const d = document.getElementById("sc-card");
    d.style.opacity = "0";
    setTimeout(() => d.remove(), 500);
  });
  await sleep(600);
}

/** Click with a visible ring so viewers see what was pressed. */
async function click(locator, pause = 900) {
  await locator.waitFor({ state: "visible", timeout: 30000 });
  await locator.scrollIntoViewIfNeeded();
  const b = await locator.boundingBox();
  if (b) {
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 12 });
    await page.evaluate(([x, y]) => {
      const r = document.getElementById("sc-click");
      r.style.left = `${x}px`;
      r.style.top = `${y}px`;
      r.style.opacity = "1";
      setTimeout(() => (r.style.opacity = "0"), 450);
    }, [b.x + b.width / 2, b.y + b.height / 2]);
  }
  await sleep(250);
  await locator.click();
  await sleep(pause);
}

const director = () => page.locator(".director");
const nextBtn = (label) => page.locator(".director .controls .btn.primary:not([disabled])", { hasText: label });
const lastCard = () => page.locator(".commitment").last();

async function scenario(title) {
  await click(page.locator(".director .scn", { hasText: title }), 400);
}

async function waitStatus(text, timeout = 30000) {
  await lastCard().locator(".status", { hasText: text }).waitFor({ timeout });
}

// --- script ---
await page.goto(url);
await page.waitForSelector("nav");
await installOverlays();
await card("nmesh × ACN", "Agent Commitments over Nostr: people and agents make, fulfil, verify and close commitments as signed events.", "PROTOTYPE DEMO");

await caption("The app", "A Nostr messenger for humans and agents. Agents are peers with their own keys; structured results render in context.", 4200);
await click(page.locator(".demo-btn"), 1200);
await caption("Before first contact", "Every agent shows <b>who runs it</b>, what it does with your data, and whether its key is <b>bound to an accountable operator</b>.", 5200);
await click(page.getByRole("button", { name: /I understand/ }), 800);
await director().waitFor();
await caption("The Director", "A presenter panel. You play the <b>requester</b>; a simulated cast (performer, verifier, judge, arbiter, gate) responds with real signed acts, judged by the real validator.", 5200);

// 1. Negotiation
await card("1 · Negotiation", "Counter-offers bind to content-addressed terms. A promise is made against a terms hash.", "PROFILE N", 3000);
await scenario("Negotiation");
await caption("Request", "Your <b>REQUEST</b> carries the terms inline, identified by their hash.", 0);
await lastCard().locator(".counter").waitFor({ timeout: 20000 });
await sleep(600);
await caption("Counter-offer", "Atlas <b>counters</b>: one more hour and a revision round. New terms → new hash. The card shows the diff.", 5000);
await click(nextBtn("Accept counter"), 800);
await caption("Accepted", "Accepting turns the counter into the promise. Atlas sends <b>PROGRESS</b> (liveness only) and then a signed <b>REPORT</b>.", 0);
await waitStatus("Your review");
await caption("Only you can close it", "A REPORT asserts completion; it never closes the commitment. Closure is the requester's act (INV-5).", 4200);
await click(nextBtn("Accept result"), 800);
await waitStatus("Satisfied");
await click(lastCard().getByRole("button", { name: /signed events/ }), 600);
await caption("Every act is a signed Nostr event", "The thread is just a projection. State is a deterministic fold over these events — same result in any order, on any relay.", 5200);
await click(lastCard().getByRole("button", { name: /signed events/ }), 300);

// 2. Verified work
await card("2 · Evidence has classes", "Mechanical attestations can satisfy a condition. LLM opinions never can.", "INV-8 · V-11 · V-16", 3200);
await scenario("Verified work");
await nextBtn("Reject the tests").waitFor({ timeout: 30000 });
await lastCard().locator(".conditions").scrollIntoViewIfNeeded();
await caption("Conditions of satisfaction", "<b>Verity</b> re-ran the tests: <b>pass, counted</b>. <b>Muse</b>, an LLM judge, says the tests look flaky — an opinion, <b>not counted</b>.", 6500);
await click(nextBtn("Reject the tests"), 900);
await caption("Contradicted rejection", "You may still reject (only you judge), but the validator flags it <b>contradicted_rejection</b>: visible, attributable.", 5500);
await waitStatus("Your review");
await caption("Rework", "Below the rework limit, the REJECT sends it back. Atlas delivers <b>v2</b>.", 3500);
await click(nextBtn("Accept result"), 700);
await waitStatus("Satisfied");

// 3. Paid call
await card("3 · Closing is paying", "21 sat hash-locked to the performer (simulated Cashu). The preimage only you hold is the payment.", "PROFILE T1 · INV-6", 3200);
await scenario("Paid call");
await waitStatus("Your review");
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Locked", "Funds are <b>🔒 locked</b> to Atlas until you are satisfied; refundable to you after the locktime.", 4500);
await click(nextBtn("Satisfy"), 900);
await waitStatus("Satisfied");
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Paid", "Your SATISFY reveals the preimage — the validator checks it against the hashlock. <b>One signed act closes and pays.</b>", 5000);

// 4. Dispute
await card("4 · Dispute & ruling", "50 sat, with an arbiter named in the terms. The preimage is escrowed to the arbiter.", "§8.4", 3200);
await scenario("Dispute");
await click(nextBtn("Reject the concepts"), 600);
await caption("Dispute", "You reject. Atlas <b>disputes</b>. The arbiter, <b>Solon</b>, holds an escrowed copy of the preimage.", 0);
await waitStatus("Satisfied", 30000);
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Ruling", "Solon rules for the performer and releases the preimage: <b>Atlas is paid without you</b>. Arbiters are scored keys like anyone else.", 5500);

// 5. Governance
await card("5 · Governance gates", "Terms can demand that an outside gate endorses the performer's acts.", "K_ENDORSE · V-14", 3000);
await scenario("Governance");
await lastCard().locator(".chain .pend", { hasText: "pending" }).waitFor({ timeout: 20000 });
await caption("Pending endorsement", "Atlas has reported, but the REPORT is <b>pending</b> until <b>Gate</b> (a human sign-off) endorses it.", 0);
await waitStatus("Your review", 30000);
await caption("Endorsed", "Gate endorsed it — only now does the REPORT take effect.", 3500);
await click(nextBtn("Accept result"), 700);
await waitStatus("Satisfied");

// 6. Silent performer
await card("6 · Time without trust in clocks", "Lapses are derived by every client, never announced.", "status_at", 3000);
await scenario("Silent");
await nextBtn("Fast-forward").waitFor({ timeout: 20000 });
await caption("Promised… then silence", "Atlas promised a 2-minute deadline and went quiet. Let's move the presentation clock.", 3800);
await click(nextBtn("Fast-forward"), 900);
await waitStatus("Lapsed");
await caption("Lapsed", "Past the deadline with no REPORT: <b>lapsed</b>, recorded against the performer for scoring.", 4200);
await click(nextBtn("Withdraw"), 700);
await waitStatus("Withdrawn");

// 7. Rogue
await card("7 · Bad actors are visible", "An unbound agent sends a PROMISE and a DECLINE at the same time.", "V-13 equivocation", 3000);
await click(page.locator("nav button", { hasText: "Rogue" }), 900);
await caption("Unaccountable", "<b>Rogue</b> has no operator binding — the app says so before you talk to it.", 3800);
await click(page.getByRole("button", { name: /I understand/ }), 700);
if (!(await director().isVisible())) await click(page.getByRole("button", { name: /Director/ }), 500);
await scenario("Rogue agent");
await waitStatus("Declined");
await caption("Deterministic resolution", "Both acts are kept as evidence. The closing act wins on every client; the other is <b>stale</b>, and Rogue is flagged for <b>equivocation</b>.", 6500);

await hideCaption();
await card("Built on Nostr, MCP and Cashu", "Rust core · NIP-17 private transport · MCP agents · JSON-Schema data contracts · Marmot groups next · ARCore later", "nmesh · ACN v0.2", 4500);

await ctx.close();
await browser.close();
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
console.log("recorded");
