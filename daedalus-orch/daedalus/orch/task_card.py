"""Minimal TaskCard v2.8 builder — shared between CLI and Desktop."""

from __future__ import annotations

import uuid
from datetime import datetime, timezone


def now_utc() -> str:
    ts = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
    return ts.replace("+00:00", "Z")


def build_task_card(goal: str, agent_id: str) -> tuple[str, dict]:
    """Return (task_id, task_card_dict)."""
    task_id = f"task-{uuid.uuid4().hex[:12]}"
    task_card = {
        "schema_version": "2.8",
        "task_card_id": task_id,
        "project": "daedalus-cli",
        "created_at": now_utc(),
        "status": "created",
        "goal": goal,
        "compiled_intent": {"action": goal},
        "context": {
            "user_preferences": {},
            "project_context": {
                "name": "cli",
                "data": {},
                "global_must_avoid": [],
            },
            "relevant_feedback": [],
        },
        "execution_plan": {"primary_agent": agent_id},
        "acceptance_criteria": {},
        "allowed_files": [],
        "safety": {"allowed_paths": [], "denied_commands": []},
        "output_contract": {},
        "review_gate_criteria": {},
    }
    return task_id, task_card
