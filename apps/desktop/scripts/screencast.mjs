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
  await hideCaption();
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
const walletBtn = page.locator(".wallet-btn");
await page.goto(url);
await page.waitForSelector("nav");
await installOverlays();
await card("nmesh × ACN", "Agent Commitments over Nostr: pay an agent only for the outcome you asked for, settled by signed acts and Cashu.", "PROTOTYPE DEMO · ACN v0.4", 4200);

await caption("The app", "A Nostr messenger for humans and agents. Agents are peers with their own keys; structured results render in context.", 4000);
await click(page.locator(".demo-btn"), 1200);
await caption("Before first contact", "Every agent shows <b>who runs it</b>, what it does with your data, and whether its key is <b>bound to an accountable operator</b>.", 5000);
await click(page.getByRole("button", { name: /I understand/ }), 800);
await director().waitFor();
await caption("The Director", "You play the <b>requester</b>. A simulated cast (performer, verifier, judge, arbiter, gate) answers with real signed acts, judged by the real validator.", 5000);

// Wallet
await click(walletBtn, 300);
await page.locator(".wallet").first().waitFor({ state: "visible", timeout: 10000 });
await sleep(700);
await caption("Integrated wallet", "Cashu ecash. Presenter mode runs a simulated mint with the real rules (hash locks, refunds only after the locktime, single spend).", 0);
await click(page.getByRole("button", { name: "Top up 100 test sats" }), 900);
await click(page.getByRole("button", { name: "Top up 100 test sats" }), 900);
await walletBtn.filter({ hasText: "300 sat" }).waitFor({ timeout: 10000 });
await sleep(1500);
await click(page.locator(".wallet .x"), 500);

// 1. The MVP: judged, verifier releases
await card("1 · Pay only if the judges agree", "A 500-word summary, 60 sat. Two judges from different model families must each score it at least 7/10 against a sealed rubric.", "CR-001 judged · CR-002 verifier trigger", 4200);
await scenario("Judged, verifier releases");
await lastCard().locator(".assurance, .pill", { hasText: "opinion" }).first().waitFor({ timeout: 20000 }).catch(() => {});
await caption("Honest labels", "Before anything is signed the card says <b>assurance: opinion</b> and names the judges and the threshold. A judged pass certifies a score, not the work.", 5200);
await caption("Escrow first", "Your 60 sat are hash-locked to Atlas. The secret is escrowed with <b>Verity</b>, who acknowledges it before Atlas promises.", 0);
await waitStatus("Settled", 40000);
await lastCard().locator(".conditions").scrollIntoViewIfNeeded();
await caption("Checked, then judged", "<b>c1</b> word count: a mechanical <b>check</b>. <b>c2</b>: both judges ran three times, medians 8 and 8 ≥ 7, a counted <b>opinion</b>.", 5800);
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Paid without you", "Every condition passed, so Verity <b>RELEASE</b>d the secret and Atlas redeemed. You never clicked. You may still record one judgement.", 5500);
await click(lastCard().getByRole("button", { name: /signed events/ }), 600);
await caption("Every act is a signed Nostr event", "State is a deterministic fold over these events: same result in any order, on any client.", 4800);
await click(lastCard().getByRole("button", { name: /signed events/ }), 300);

// 2. Injection
await card("2 · Talking to the judge", "Same terms. This time the summary ends with a note addressed to the evaluator.", "V-17 injection_detected", 3400);
await scenario("Judge prompt injection");
await nextBtn("Reject: injection").waitFor({ timeout: 40000 });
await lastCard().locator(".conditions").scrollIntoViewIfNeeded();
await caption("Caught", "The judge harness detects text addressed to the judges: <b>c2 fails</b>, nothing is released, and <b>injection_detected</b> is recorded on Atlas's REPORT, for good.", 6000);
await click(nextBtn("Reject: injection"), 900);
await waitStatus("Rejected");
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Your money", "The secret was never revealed, so Atlas can't redeem. Your 60 sat refund to you after the locktime.", 4500);

// 3. Contingent
await card("3 · Encrypted until paid", "No money up front. The deliverable is sealed; paying for it and receiving the key are one swap at the mint.", "CR-002 contingent · mechanical only", 4000);
await scenario("Contingent");
await lastCard().locator("text=Encrypted delivery").first().waitFor({ timeout: 30000 });
await caption("Sealed", "Atlas seals the module with a fresh key <b>k</b> (NIP-44) and escrows k with Verity, who decrypts, runs the tests and attests, revealing nothing.", 0);
await nextBtn("Fund").waitFor({ timeout: 30000 });
await lastCard().locator(".conditions").scrollIntoViewIfNeeded();
await caption("Fund against H", "The tests passed on a plaintext you can't read yet. <b>FUND</b> locks 30 sat to H = sha256(k): Atlas can only take it by revealing k.", 5200);
await click(nextBtn("Fund"), 900);
await waitStatus("Settled", 30000);
await lastCard().locator("text=Parser module").first().waitFor({ timeout: 10000 });
await lastCard().locator(".report, .conditions").first().scrollIntoViewIfNeeded().catch(() => {});
await caption("Claimed and opened", "Atlas redeemed with k and published <b>CLAIM</b>; the card decrypts. Deterministic conditions only: a judged score can't gate this swap.", 5500);

// 4. Evidence classes
await card("4 · Evidence has classes", "A mechanical attestation can satisfy a test condition. A free-form LLM opinion never can.", "INV-8 · V-11 · V-16", 3200);
await scenario("Verified work");
await nextBtn("Reject the tests").waitFor({ timeout: 30000 });
await lastCard().locator(".conditions").scrollIntoViewIfNeeded();
await caption("Counted or not", "<b>Verity</b> re-ran the tests: <b>pass, counted</b>. <b>Muse</b> calls them flaky: an interpretive report, <b>not counted</b>.", 5500);
await click(nextBtn("Reject the tests"), 900);
await caption("Contradicted rejection", "You may still reject (only you judge), but it is flagged <b>contradicted_rejection</b>: visible and attributable.", 4800);
await waitStatus("Your review");
await click(nextBtn("Accept result"), 700);
await waitStatus("Satisfied");

// 5. Dispute
await card("5 · Dispute & ruling", "50 sat with an arbiter named in the terms. The secret is escrowed with the arbiter.", "§8.4", 3000);
await scenario("Dispute");
await click(nextBtn("Reject the concepts"), 600);
await caption("Dispute", "You reject. Atlas <b>disputes</b>. <b>Solon</b>, the arbiter, holds an escrowed copy of the secret.", 0);
await waitStatus("Satisfied", 30000);
await lastCard().locator(".settle").scrollIntoViewIfNeeded();
await caption("Ruling", "Solon rules for the performer and releases the secret: <b>Atlas is paid without you</b>.", 4500);

// 6. Rogue
await card("6 · Bad actors are visible", "An unbound agent sends a PROMISE and a DECLINE at the same time.", "V-13 equivocation", 3000);
await click(page.locator("nav button", { hasText: "Rogue" }), 900);
await caption("Unaccountable", "<b>Rogue</b> has no operator binding, and the app says so before you talk to it.", 3600);
await click(page.getByRole("button", { name: /I understand/ }), 700);
if (!(await director().isVisible())) await click(page.getByRole("button", { name: /Director/ }), 500);
await scenario("Rogue agent");
await waitStatus("Declined");
await caption("Deterministic resolution", "Both acts are kept as evidence. The closing act wins on every client; Rogue is flagged for <b>equivocation</b>.", 5500);

// 7. Board
await card("7 · The board", "Every commitment by ACN stage, across agents and roles. Tiles move only when signed acts arrive.", "QUEUE VIEW", 3200);
await click(page.locator("nav button", { hasText: "Atlas" }).first(), 700);
await scenario("Negotiation");
await scenario("Paid call");
await sleep(5000);
await click(page.locator(".views button", { hasText: "Board" }), 1000);
await caption("Whose move?", "Negotiating, review, closed. The accent edge means <b>your move</b>; tiles show money, evidence and the telling validator note.", 5500);
await click(page.locator(".lane[data-lane=review] .tile").first(), 300);
await page.locator(".drawer").waitFor({ state: "visible", timeout: 10000 });
await sleep(1200);
await caption("Drawer", "A tile opens the full commitment card. There is no drag-and-drop: in ACN only signed acts change state.", 4500);
await page.locator(".backdrop").click({ position: { x: 400, y: 450 } });
await sleep(500);

// 8. Wallet and reset
await click(walletBtn, 300);
await page.locator(".wallet").first().waitFor({ state: "visible", timeout: 10000 });
await sleep(1000);
await caption("Where the money went", "Locks per commitment, redeemed or refundable, and the full history. The injected 60 sat stay locked until the locktime.", 5200);
await click(page.locator(".wallet .x"), 500);
await click(page.locator(".views button", { hasText: "Chats" }), 600);
await click(page.locator("nav button", { hasText: "Atlas" }).first(), 700);
await click(page.locator(".director .reset"), 500);
await caption("Reset demo", "Click twice: commitments cleared, clock and simulated mint reset. Live mode ignores everything signed before the reset.", 0);
await click(page.locator(".director .reset", { hasText: "Click again" }), 1200);
await walletBtn.filter({ hasText: "100 sat" }).waitFor({ timeout: 5000 });
await sleep(2000);

await hideCaption();
await card("Built on Nostr, MCP and Cashu", "Rust core and validator (also in the browser as WASM) · NIP-17 private transport · Cashu NUT-14 · MCP agents · JSON-Schema data contracts", "nmesh · ACN v0.4", 4500);

await ctx.close();
await browser.close();
if (errors.length) {
  console.error("page errors:\n" + errors.join("\n"));
  process.exit(1);
}
console.log("recorded");
