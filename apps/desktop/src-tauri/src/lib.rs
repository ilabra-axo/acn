//! Tauri shell: every command is a one-line delegation to `nmesh_app::App`.
//! Keep logic out of here so the Android/ARCore client (via FFI) behaves identically.
//!
//! Configuration (env):
//! - `NMESH_RELAY`  comma-separated relay URLs (default `ws://127.0.0.1:7777`)
//! - `NMESH_DATA`   data directory (default: the OS app-data dir)
//! - `NMESH_NAME`   display name to publish
//! - `NMESH_MINT`   Cashu mint of the built-in wallet (default `http://127.0.0.1:3338`, the test
//!   mint `nmesh-acn-sim` hosts; empty or `none` disables the wallet)

use nmesh_app::{
    AcnHint, App, AppConfig, AppError, BoardView, ClockView, CommitmentView, ContactView, Identity,
    MessageView, WalletView,
};
use serde_json::Value;
use tauri::{Emitter, Manager, State};
use tracing_subscriber::EnvFilter;

const EVENT: &str = "nmesh://event";

#[tauri::command]
fn identity(app: State<'_, App>) -> Identity {
    app.identity()
}

#[tauri::command]
async fn contacts(app: State<'_, App>) -> Result<Vec<ContactView>, AppError> {
    Ok(app.contacts().await)
}

#[tauri::command]
async fn open_conversation(
    app: State<'_, App>,
    pubkey: String,
) -> Result<Vec<MessageView>, AppError> {
    app.open(&pubkey).await
}

#[tauri::command]
async fn add_contact(app: State<'_, App>, input: String) -> Result<ContactView, AppError> {
    app.add_contact(&input).await
}

#[tauri::command]
async fn accept_agent(app: State<'_, App>, pubkey: String) -> Result<ContactView, AppError> {
    app.accept_agent(&pubkey).await
}

#[tauri::command]
async fn send_text(
    app: State<'_, App>,
    pubkey: String,
    text: String,
) -> Result<MessageView, AppError> {
    app.send_text(&pubkey, &text).await
}

#[tauri::command]
async fn send_action(
    app: State<'_, App>,
    pubkey: String,
    target: String,
    action_id: Option<String>,
    values: Option<Value>,
    label: Option<String>,
) -> Result<MessageView, AppError> {
    app.send_action(&pubkey, &target, action_id, values, label)
        .await
}

// --- ACN commitments (docs/Core.md) ---

#[tauri::command]
async fn commitments(app: State<'_, App>, pubkey: String) -> Result<Vec<CommitmentView>, AppError> {
    app.commitments(&pubkey).await
}

#[tauri::command]
async fn acn_request(
    app: State<'_, App>,
    pubkey: String,
    text: String,
) -> Result<CommitmentView, AppError> {
    app.request(&pubkey, &text).await
}

#[tauri::command]
async fn acn_satisfy(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<CommitmentView, AppError> {
    app.satisfy(&pubkey, &commitment).await
}

#[tauri::command]
async fn acn_reject(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
    condition: Option<String>,
    note: String,
) -> Result<CommitmentView, AppError> {
    app.reject(&pubkey, &commitment, condition, &note).await
}

#[tauri::command]
async fn acn_accept_counter(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<CommitmentView, AppError> {
    app.accept_counter(&pubkey, &commitment).await
}

#[tauri::command]
async fn acn_dispute(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
    reason: String,
) -> Result<CommitmentView, AppError> {
    app.dispute(&pubkey, &commitment, &reason).await
}

#[tauri::command]
async fn acn_hint(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<Option<AcnHint>, AppError> {
    app.hint(&pubkey, &commitment).await
}

#[tauri::command]
async fn acn_run_hint(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<Option<CommitmentView>, AppError> {
    app.run_hint(&pubkey, &commitment).await
}

#[tauri::command]
async fn discover_demo(app: State<'_, App>) -> Result<Vec<ContactView>, AppError> {
    app.discover_demo().await
}

#[tauri::command]
fn clock(app: State<'_, App>) -> ClockView {
    app.clock()
}

#[tauri::command]
async fn set_clock_offset(app: State<'_, App>, offset: i64) -> Result<ClockView, AppError> {
    Ok(app.set_clock_offset(offset).await)
}

#[tauri::command]
async fn acn_withdraw(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<CommitmentView, AppError> {
    app.withdraw(&pubkey, &commitment).await
}

#[tauri::command]
async fn acn_fund(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<CommitmentView, AppError> {
    app.fund(&pubkey, &commitment).await
}

#[tauri::command]
async fn board(app: State<'_, App>, closed_limit: usize) -> Result<BoardView, AppError> {
    Ok(app.board(closed_limit).await)
}

#[tauri::command]
async fn reset_demo(app: State<'_, App>) -> Result<(), AppError> {
    app.reset_demo().await;
    Ok(())
}

// --- Wallet (Cashu, docs/Core.md §8) ---

#[tauri::command]
async fn wallet(app: State<'_, App>) -> Result<Option<WalletView>, AppError> {
    app.wallet().await
}

#[tauri::command]
async fn wallet_top_up(app: State<'_, App>, amount: u64) -> Result<WalletView, AppError> {
    app.top_up(amount).await
}

#[tauri::command]
async fn wallet_refund(
    app: State<'_, App>,
    pubkey: String,
    commitment: String,
) -> Result<WalletView, AppError> {
    app.refund(&pubkey, &commitment).await
}

// --- Version and dev updates ---

#[derive(serde::Serialize)]
struct BuildInfo {
    version: &'static str,
    commit: &'static str,
    built_at: u64,
    acn: &'static str,
    /// A debug build from a source checkout: updates mean "rebuild from the repo".
    dev: bool,
}

#[tauri::command]
fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("NMESH_GIT_COMMIT"),
        built_at: env!("NMESH_BUILT_AT").parse().unwrap_or(0),
        acn: nmesh_app::ACN_VERSION,
        dev: cfg!(debug_assertions),
    }
}

#[derive(serde::Serialize)]
struct UpdateStatus {
    /// The commit this binary was built from.
    current: &'static str,
    /// The repo's HEAD now.
    latest: String,
    available: bool,
    /// Commit subjects between the two, newest first (at most 10).
    changes: Vec<String>,
}

fn repo_git(args: &[&str]) -> Option<String> {
    let repo = option_env!("NMESH_REPO")?;
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Dev builds only: is the source checkout ahead of this binary? There is no release channel
/// yet, so release builds report nothing (a signed updater feed would go here).
#[tauri::command]
fn update_check() -> Option<UpdateStatus> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let current = env!("NMESH_GIT_COMMIT");
    let latest = repo_git(&["rev-parse", "--short=9", "HEAD"])?;
    let available = latest != current;
    let changes = if available {
        repo_git(&["log", "--format=%h %s", "-10", &format!("{current}..HEAD")])
            .map(|l| l.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    } else {
        vec![]
    };
    Some(UpdateStatus {
        current,
        latest,
        available,
        changes,
    })
}

/// Dev builds only: rewrite `build.rs` (same content) so `pnpm tauri dev` rebuilds the app from
/// the current sources and relaunches it. Release builds just relaunch.
#[tauri::command]
fn restart_to_update(handle: tauri::AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/build.rs");
        let src = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        std::fs::write(path, src).map_err(|e| format!("{path}: {e}"))?;
        tracing::info!("touched build.rs: tauri dev will rebuild and restart the app");
        Ok(())
    } else {
        handle.restart()
    }
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|tauri_app| {
            let relays: Vec<String> = std::env::var("NMESH_RELAY")
                .unwrap_or_else(|_| "ws://127.0.0.1:7777".into())
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let data_dir = match std::env::var_os("NMESH_DATA") {
                Some(d) => d.into(),
                None => tauri_app.path().app_data_dir()?,
            };
            let mut config = AppConfig::new(relays, data_dir);
            config.display_name = std::env::var("NMESH_NAME").ok();
            config.mint = match std::env::var("NMESH_MINT") {
                Ok(m) if m.trim().is_empty() || m.trim() == "none" => None,
                Ok(m) => Some(m.trim().to_string()),
                Err(_) => Some("http://127.0.0.1:3338".into()),
            };

            // Connecting waits at most a few seconds for relays; failures are non-fatal.
            let (app, mut events) = tauri::async_runtime::block_on(App::start(config))?;
            tauri_app.manage(app);

            let handle = tauri_app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = events.recv().await {
                    if let Err(e) = handle.emit(EVENT, ev) {
                        tracing::warn!(error = %e, "failed to emit event to webview");
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            identity,
            contacts,
            open_conversation,
            add_contact,
            accept_agent,
            send_text,
            send_action,
            commitments,
            acn_request,
            acn_satisfy,
            acn_reject,
            acn_withdraw,
            acn_accept_counter,
            acn_dispute,
            acn_hint,
            acn_run_hint,
            clock,
            set_clock_offset,
            discover_demo,
            wallet,
            wallet_top_up,
            wallet_refund,
            reset_demo,
            board,
            acn_fund,
            build_info,
            update_check,
            restart_to_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running nmesh");
}
