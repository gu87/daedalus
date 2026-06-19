//! P5.3a: TaskStatus — 9-state pipeline task lifecycle.

/// Pipeline task status.
///
/// Separate from `AgentRunStatus` (daedalusd layer, 6 states).
/// The two are linked by `task_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Created,
    Dispatched,
    Running,
    WaitingForVerification,
    Completed,
    NeedsHumanReview,
    Failed,
    Blocked,
    Discarded,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Created => "created",
            TaskStatus::Dispatched => "dispatched",
            TaskStatus::Running => "running",
            TaskStatus::WaitingForVerification => "waiting_for_verification",
            TaskStatus::Completed => "completed",
            TaskStatus::NeedsHumanReview => "needs_human_review",
            TaskStatus::Failed => "failed",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Discarded => "discarded",
        }
    }

    pub fn parse_status(s: &str) -> Option<Self> {
        match s {
            "created" => Some(TaskStatus::Created),
            "dispatched" => Some(TaskStatus::Dispatched),
            "running" => Some(TaskStatus::Running),
            "waiting_for_verification" => Some(TaskStatus::WaitingForVerification),
            "completed" => Some(TaskStatus::Completed),
            "needs_human_review" => Some(TaskStatus::NeedsHumanReview),
            "failed" => Some(TaskStatus::Failed),
            "blocked" => Some(TaskStatus::Blocked),
            "discarded" => Some(TaskStatus::Discarded),
            _ => None,
        }
    }

    /// Validate a transition from `current` to `next`.
    ///
    /// Returns `Ok(next)` if the transition is allowed, or `Err` with a
    /// human-readable reason.
    pub fn transition(current: &TaskStatus, next: &TaskStatus) -> Result<TaskStatus, String> {
        use TaskStatus::*;
        match (current, next) {
            // Created → Dispatched
            (Created, Dispatched) => Ok(Dispatched),
            // Dispatched → Running
            (Dispatched, Running) => Ok(Running),
            // Running → WaitingForVerification / Failed / Blocked
            (Running, WaitingForVerification) => Ok(WaitingForVerification),
            (Running, Failed) => Ok(Failed),
            (Running, Blocked) => Ok(Blocked),
            // WaitingForVerification → Completed / NeedsHumanReview / Failed
            (WaitingForVerification, Completed) => Ok(Completed),
            (WaitingForVerification, NeedsHumanReview) => Ok(NeedsHumanReview),
            (WaitingForVerification, Failed) => Ok(Failed),
            // NeedsHumanReview → Completed / Failed
            (NeedsHumanReview, Completed) => Ok(Completed),
            (NeedsHumanReview, Failed) => Ok(Failed),
            // Blocked → Dispatched (retry) / Discarded
            (Blocked, Dispatched) => Ok(Dispatched),
            (Blocked, Discarded) => Ok(Discarded),
            // Terminal states cannot transition.
            (Completed, _) | (Failed, _) | (Discarded, _) => Err(format!(
                "cannot transition from terminal state '{}'",
                current.as_str()
            )),
            // Self-transition: no-op.
            (c, n) if c == n => Ok(next.clone()),
            // Anything else is illegal.
            (c, n) => Err(format!(
                "illegal transition: '{}' -> '{}'",
                c.as_str(),
                n.as_str()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_all_nine() {
        let all = [
            (TaskStatus::Created, "created"),
            (TaskStatus::Dispatched, "dispatched"),
            (TaskStatus::Running, "running"),
            (
                TaskStatus::WaitingForVerification,
                "waiting_for_verification",
            ),
            (TaskStatus::Completed, "completed"),
            (TaskStatus::NeedsHumanReview, "needs_human_review"),
            (TaskStatus::Failed, "failed"),
            (TaskStatus::Blocked, "blocked"),
            (TaskStatus::Discarded, "discarded"),
        ];
        for (v, s) in &all {
            assert_eq!(v.as_str(), *s);
        }
    }

    #[test]
    fn parse_status_all_nine() {
        for s in &[
            "created",
            "dispatched",
            "running",
            "waiting_for_verification",
            "completed",
            "needs_human_review",
            "failed",
            "blocked",
            "discarded",
        ] {
            let v = TaskStatus::parse_status(s).expect(s);
            assert_eq!(v.as_str(), *s);
        }
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(TaskStatus::parse_status("bogus").is_none());
    }

    #[test]
    fn transition_created_to_dispatched() {
        assert!(TaskStatus::transition(&TaskStatus::Created, &TaskStatus::Dispatched).is_ok());
    }

    #[test]
    fn transition_running_to_waiting() {
        assert!(
            TaskStatus::transition(&TaskStatus::Running, &TaskStatus::WaitingForVerification)
                .is_ok()
        );
    }

    #[test]
    fn transition_terminal_rejected() {
        assert!(TaskStatus::transition(&TaskStatus::Completed, &TaskStatus::Running).is_err());
    }

    #[test]
    fn transition_created_to_running_rejected() {
        assert!(TaskStatus::transition(&TaskStatus::Created, &TaskStatus::Running).is_err());
    }

    #[test]
    fn transition_dispatched_to_running() {
        assert!(TaskStatus::transition(&TaskStatus::Dispatched, &TaskStatus::Running).is_ok());
    }

    #[test]
    fn transition_waiting_to_completed() {
        assert!(TaskStatus::transition(
            &TaskStatus::WaitingForVerification,
            &TaskStatus::Completed
        )
        .is_ok());
    }

    #[test]
    fn transition_running_to_failed() {
        assert!(TaskStatus::transition(&TaskStatus::Running, &TaskStatus::Failed).is_ok());
    }
}
