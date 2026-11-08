use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(args: &[&str]) -> Option<String> {
    let o = Command::new("git").args(args).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Build metadata for the version shown in the UI and the dev update check.
fn main() {
    let commit = git(&["rev-parse", "--short=9", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=NMESH_GIT_COMMIT={commit}");
    if let Some(root) = git(&["rev-parse", "--show-toplevel"]) {
        println!("cargo:rustc-env=NMESH_REPO={root}");
        // Re-run (new commit id) when HEAD moves.
        println!("cargo:rerun-if-changed={root}/.git/HEAD");
        if let Some(r) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={root}/.git/{r}");
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=NMESH_BUILT_AT={now}");
    println!("cargo:rerun-if-changed=build.rs");
    tauri_build::build()
}
