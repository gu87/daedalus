use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);
pub fn run_hook(script: &str, env: &[(&str, &str)], run_dir: &Path, hook_name: &str) {
    let log_path = run_dir.join(format!("hook-{hook_name}.log"));
    let script = script.to_string();
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut cmd = Command::new("sh");
        cmd.arg(&script);
        for (k, v) in &env {
            cmd.env(k, v);
        }
        let _ = tx.send(cmd.output());
    });
    match rx.recv_timeout(HOOK_TIMEOUT) {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = fs::write(
                &log_path,
                format!(
                    "exit={}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
                    output
                        .status
                        .code()
                        .map_or_else(|| "signal".into(), |c| c.to_string())
                ),
            );
        }
        Ok(Err(e)) => {
            let _ = fs::write(&log_path, format!("hook failed: {e}\n"));
        }
        Err(_) => {
            let _ = fs::write(
                &log_path,
                format!("hook timed out after {}s\n", HOOK_TIMEOUT.as_secs()),
            );
        }
    }
}
