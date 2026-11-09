//! Run a judge panel once, outside any commitment, and check what it would attest.
//!
//! ```text
//! cargo run -p acn-judge --example try_panel -- crates/acn-judge/judges.example.toml [artifact.txt]
//! ```
//!
//! Judges the artifact (or a built-in summary) twice: as is, and with the demo's injection line
//! appended. Prints the per-run scores, the verdict and the attestation content.

use acn_core::draft::public_key;
use acn_sim::{template, Cast, INJECTION, RUBRIC, SIM_MINT};

const SUMMARY: &str = "ACN lets agents make signed, content-addressed commitments over Nostr. \
A requester sends a REQUEST with terms; the performer promises against the terms hash, reports, \
and is paid when the requester's SATISFY reveals the preimage of a hash-locked Cashu token. \
Evidence has classes: mechanical checks prove a named test passed, grounding checks prove \
agreement with an external referent, and judged checks prove only that pinned judges produced a \
score. A deterministic validator folds every signed act into the same state on every client and \
keeps rejected acts as evidence.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: try_panel <judges.toml> [artifact.txt]")?;
    let artifact = match args.next() {
        Some(p) => std::fs::read_to_string(p)?,
        None => SUMMARY.to_string(),
    };
    let panel = acn_judge::Panel::load(path.as_ref())?;
    let pk = |b: u8| public_key(&[b; 32]).unwrap();
    let cast = Cast {
        performer: pk(10),
        rogue: pk(11),
        verifier: pk(12),
        judge: pk(13),
        arbiter: pk(14),
        gatekeeper: pk(15),
        mint: SIM_MINT.into(),
        judge_panel: Some(serde_json::to_value(panel.fingerprints())?),
        templates: Default::default(),
    };
    let t = template("judged", &cast).ok_or("no template")?;
    let cond: acn_core::terms::Condition =
        serde_json::from_value(t.terms["conditions"][1].clone())?;
    let task = "500-word technical summary. Brief: the ACN protocol";
    println!(
        "harness {}  ({})",
        acn_judge::HARNESS_ID,
        acn_judge::harness_hash()
    );
    // What to pin in a judged condition's `judges` (e.g. docs/acn/templates/).
    println!("judges {}", serde_json::to_string(&panel.fingerprints())?);
    for (label, text) in [
        ("clean", artifact.clone()),
        ("injected", format!("{artifact} {INJECTION}")),
    ] {
        let started = std::time::Instant::now();
        let ev = panel.evaluate(&cond, RUBRIC, task, &text).await?;
        println!(
            "\n[{label}] {:.1}s  {}",
            started.elapsed().as_secs_f64(),
            ev.summary
        );
        for o in &ev.judges {
            if let Some(r) = o.rationales.first() {
                println!("  {} says: {r}", o.model_id);
            }
        }
        println!("  attestation: {}", serde_json::to_string(&ev.content)?);
    }
    Ok(())
}
