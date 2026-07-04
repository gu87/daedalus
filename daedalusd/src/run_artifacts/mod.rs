pub mod hooks;
pub mod summary;
pub mod transcript;

use std::path::PathBuf;

fn ensure_run_dir(runs_dir: &str, run_id: &str) -> PathBuf {
    let d = PathBuf::from(runs_dir).join(run_id);
    let _ = std::fs::create_dir_all(&d);
    d
}

#[allow(clippy::too_many_arguments)]
pub fn write_task_done_artifacts(
    runs_dir: &str,
    run_id: &str,
    task_id: &str,
    agent_id: &str,
    goal: &str,
    outbox: &crate::types::Outbox,
    started_at: &str,
    finished_at: &str,
    hooks: &crate::config::HooksConfig,
) {
    let run_dir = ensure_run_dir(runs_dir, run_id);
    let tpath = run_dir.join("transcript.jsonl");
    let spath = run_dir.join("summary.md");
    if let Ok(mut tw) = transcript::TranscriptWriter::new(&tpath) {
        tw.write_event(&serde_json::json!({
            "type":"task.done","run_id":run_id,"task_id":task_id,
            "agent_id":agent_id,"outbox":outbox,"timestamp":finished_at,
        }));
    }
    let _ = summary::write_summary_for_done(
        &spath,
        run_id,
        task_id,
        agent_id,
        goal,
        outbox,
        started_at,
        finished_at,
    );
    let rd = run_dir.to_string_lossy().to_string();
    let tp = tpath.to_string_lossy().to_string();
    let sp = spath.to_string_lossy().to_string();
    for script in &hooks.task_done {
        let env: Vec<(&str, &str)> = vec![
            ("DAEDALUS_RUN_ID", run_id),
            ("DAEDALUS_TASK_ID", task_id),
            ("DAEDALUS_RUN_DIR", &rd),
            ("DAEDALUS_TRANSCRIPT_PATH", &tp),
            ("DAEDALUS_SUMMARY_PATH", &sp),
            ("DAEDALUS_STATUS", "done"),
        ];
        hooks::run_hook(script, &env, &run_dir, "task_done");
    }
}

#[allow(clippy::too_many_arguments)]
pub fn write_task_error_artifacts(
    runs_dir: &str,
    run_id: &str,
    task_id: &str,
    agent_id: &str,
    goal: &str,
    error_taxonomy: &str,
    detail: &str,
    started_at: &str,
    finished_at: &str,
    hooks: &crate::config::HooksConfig,
) {
    let run_dir = ensure_run_dir(runs_dir, run_id);
    let tpath = run_dir.join("transcript.jsonl");
    let spath = run_dir.join("summary.md");
    if let Ok(mut tw) = transcript::TranscriptWriter::new(&tpath) {
        tw.write_event(&serde_json::json!({
            "type":"task.error","run_id":run_id,"task_id":task_id,
            "agent_id":agent_id,"error_taxonomy":error_taxonomy,
            "detail":detail,"timestamp":finished_at,
        }));
    }
    let _ = summary::write_summary_for_error(
        &spath,
        run_id,
        task_id,
        agent_id,
        goal,
        error_taxonomy,
        detail,
        started_at,
        finished_at,
    );
    let rd = run_dir.to_string_lossy().to_string();
    let tp = tpath.to_string_lossy().to_string();
    let sp = spath.to_string_lossy().to_string();
    for script in &hooks.task_error {
        let env: Vec<(&str, &str)> = vec![
            ("DAEDALUS_RUN_ID", run_id),
            ("DAEDALUS_TASK_ID", task_id),
            ("DAEDALUS_RUN_DIR", &rd),
            ("DAEDALUS_TRANSCRIPT_PATH", &tp),
            ("DAEDALUS_SUMMARY_PATH", &sp),
            ("DAEDALUS_STATUS", "error"),
        ];
        hooks::run_hook(script, &env, &run_dir, "task_error");
    }
}

pub fn write_task_started(
    runs_dir: &str,
    run_id: &str,
    task_id: &str,
    agent_id: &str,
    goal: &str,
    timestamp: &str,
) {
    let run_dir = ensure_run_dir(runs_dir, run_id);
    let tpath = run_dir.join("transcript.jsonl");
    if let Ok(mut tw) = transcript::TranscriptWriter::new(&tpath) {
        tw.write_event(&serde_json::json!({
            "type":"task.started","run_id":run_id,"task_id":task_id,
            "agent_id":agent_id,"goal":goal,"timestamp":timestamp,
        }));
    }
}
