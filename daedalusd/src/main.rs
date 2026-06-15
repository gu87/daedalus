use std::path::PathBuf;

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

    // ── IPC server ─────────────────────────────────────────────────
    eprintln!("daedalusd v0.1.0 starting on {}", socket_path.display());

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("register SIGTERM handler");

        tokio::select! {
            _ = &mut ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    };

    match daedalusd::ipc::server::run(&socket_path, shutdown).await {
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
