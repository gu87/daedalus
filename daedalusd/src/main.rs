use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use daedalusd::daemon::{DaemonContext, DefaultAgentLoopFactory};
use daedalusd::gate::{CriteriaRegistry, GateRouter};

/// Orphan scan runs every 60 s.
const ORPHAN_SCAN_INTERVAL_SECS: u64 = 60;
/// Runs whose heartbeat is older than this threshold (90 s) are orphaned.
const ORPHAN_CUTOFF_SECS: i64 = 90;
/// P3.4: maximum task retries enforced by GateRouter global cap.
const MAX_TASK_RETRIES: u32 = 5;

#[tokio::main]
async fn main() {
    // ── socket path ────────────────────────────────────────────────
    let socket_path = std::env::var("DAEDALUSD_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/daedalusd.sock"));

    // ── state directory & database ─────────────────────────────────
    let state_dir = std::env::var("DAEDALUSD_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".daedalus").join("state")
        });

    if let Err(e) = init_db(&state_dir) {
        eprintln!("daedalusd fatal: database init failed: {}", e);
        std::process::exit(1);
    }

    let db_path = state_dir.join("daedalusd.sqlite");

    // ── daemon config (P2.7 + P3.4 gate) ───────────────────────────
    let mut config = daedalusd::config::DaedalusConfig::load();
    config.db_path = Some(db_path.clone());

    // P3.4: construct GateRouter from gate-criteria.yaml.
    // File not found → defaults (silent).  YAML parse error / invalid
    // action → fatal (exit 1).
    let registry = match CriteriaRegistry::with_overrides(&config.gate_criteria_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("daedalusd fatal: invalid gate criteria config: {e}");
            std::process::exit(1);
        }
    };
    let gate_router = Arc::new(GateRouter::new(registry, MAX_TASK_RETRIES));

    let ctx = Arc::new(DaemonContext {
        config: config.clone(),
        db_path: db_path.clone(),
        factory: Arc::new(DefaultAgentLoopFactory { config }),
        gate_router,
    });

    // ── shared shutdown token ──────────────────────────────────────
    let shutdown_token = CancellationToken::new();

    // ── orphan scanner ─────────────────────────────────────────────
    let orphan_token = shutdown_token.clone();
    let orphan_db = db_path.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(ORPHAN_SCAN_INTERVAL_SECS));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let cutoff = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64
                        - ORPHAN_CUTOFF_SECS;
                    match tokio::task::spawn_blocking({
                        let db_path = orphan_db.clone();
                        move || {
                            let conn = daedalusd::db::pool::open(&db_path)?;
                            daedalusd::db::orphan::scan_orphans(&conn, cutoff)
                        }
                    })
                    .await
                    {
                        Ok(Ok(ids)) if !ids.is_empty() => {
                            eprintln!(
                                "daedalusd orphaned {} run(s): {:?}",
                                ids.len(),
                                ids
                            );
                        }
                        Ok(Err(e)) => {
                            eprintln!("daedalusd orphan scan error: {e}");
                        }
                        Err(_) => {}
                        _ => {}
                    }
                }
                _ = orphan_token.cancelled() => break,
            }
        }
    });

    // ── IPC server ─────────────────────────────────────────────────
    eprintln!("daedalusd v0.2.0 starting on {}", socket_path.display());

    let signal_token = shutdown_token.clone();
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("register SIGTERM handler");

        tokio::select! {
            _ = &mut ctrl_c => {}
            _ = sigterm.recv() => {}
        }

        signal_token.cancel();
    });

    let shutdown = shutdown_token.cancelled();

    match daedalusd::ipc::server::run_with_context(&socket_path, shutdown, ctx).await {
        Ok(()) => eprintln!("daedalusd shut down cleanly"),
        Err(e) => {
            eprintln!("daedalusd fatal: {}", e);
            std::process::exit(1);
        }
    }
}

fn init_db(state_dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(state_dir)?;
    let db_path = state_dir.join("daedalusd.sqlite");
    let mut conn = daedalusd::db::pool::open(&db_path)?;
    daedalusd::db::migrations::run_all(&mut conn)?;
    eprintln!("daedalusd database ready at {}", db_path.display());
    Ok(())
}
