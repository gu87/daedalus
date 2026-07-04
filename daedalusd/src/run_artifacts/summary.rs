use std::fs;
use std::path::Path;
#[allow(clippy::too_many_arguments)]
pub fn write_summary_for_done(
    dest: &Path,
    run_id: &str,
    task_id: &str,
    agent_id: &str,
    goal: &str,
    outbox: &crate::types::Outbox,
    started_at: &str,
    finished_at: &str,
) -> std::io::Result<()> {
    let md = format!("# Run Summary\n\n- Run ID: {run_id}\n- Task ID: {task_id}\n- Agent: {agent_id}\n- Goal: {goal}\n- Status: done\n- Started: {started_at}\n- Finished: {finished_at}\n\n## Result\n\n{}\n\n## Artifacts\n\n- [transcript.jsonl](./transcript.jsonl)\n", outbox.summary);
    let _ = fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
    fs::write(dest, md)
}
#[allow(clippy::too_many_arguments)]
pub fn write_summary_for_error(
    dest: &Path,
    run_id: &str,
    task_id: &str,
    agent_id: &str,
    goal: &str,
    error_taxonomy: &str,
    detail: &str,
    started_at: &str,
    finished_at: &str,
) -> std::io::Result<()> {
    let md = format!("# Run Summary\n\n- Run ID: {run_id}\n- Task ID: {task_id}\n- Agent: {agent_id}\n- Goal: {goal}\n- Status: error ({error_taxonomy})\n- Started: {started_at}\n- Finished: {finished_at}\n\n## Error\n\n{detail}\n\n## Artifacts\n\n- [transcript.jsonl](./transcript.jsonl)\n");
    let _ = fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")));
    fs::write(dest, md)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_done_has_required_fields() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("s.md");
        let ob = crate::types::Outbox {
            schema_version: "2.8".into(),
            task_id: "t1".into(),
            agent_id: "a1".into(),
            status: "done".into(),
            summary: "All good.".into(),
            changed_files: vec![],
            changed_files_source: "unknown".into(),
            verification: serde_json::json!({}),
            evidence: serde_json::json!({}),
            known_risks: vec![],
            errors: vec![],
            error_taxonomy: vec![],
            needs_human_review: false,
            notes: vec![],
        };
        write_summary_for_done(
            &p,
            "r1",
            "t1",
            "a1",
            "test goal",
            &ob,
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
        )
        .unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains("Run ID: r1"));
    }
    #[test]
    fn summary_error_has_required_fields() {
        let d = tempfile::TempDir::new().unwrap();
        let p = d.path().join("s.md");
        write_summary_for_error(
            &p,
            "r2",
            "t2",
            "a2",
            "bad goal",
            "tool_failure",
            "boom",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:01:00Z",
        )
        .unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains("r2"));
    }
}
