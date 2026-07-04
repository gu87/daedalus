"""CLI entry point for daedalus-orch — Pi × Daedalus V1 bridge."""

from __future__ import annotations

import argparse
import asyncio
import json
import math
import os
import sys
from pathlib import Path
from typing import Any

from daedalus.orch.client import DaedalusClient
from daedalus.orch.exceptions import (
    DaedalusClientError,
    DaedalusConnectionError,
    DaedalusError,
    DaedalusTimeoutError,
)
from daedalus.orch.task_card import build_task_card

DEFAULT_SOCK = os.environ.get("DAEDALUSD_SOCK", "/tmp/daedalusd.sock")
DEFAULT_AGENT = "default-worker"
DEFAULT_TIMEOUT = 300.0
DEFAULT_RUNS_DIR = os.path.join(os.environ.get("HOME", "/tmp"), ".daedalus", "runs")


def _terminal_permission_handler(perm: dict[str, Any]) -> str:
    """Interactive [y/N] permission handler.  Defaults to denied on non-TTY."""
    tool = perm.get("tool", "unknown")
    args = json.dumps(perm.get("args", {}), ensure_ascii=False)
    prompt = f"\n[Gate] {tool}\n  {args}\n批准？[y/N] "

    if not sys.stdin.isatty():
        print(f"[Gate] {tool} — non-TTY, auto-denied", file=sys.stderr)
        return "denied"

    try:
        ans = input(prompt).strip().lower()
    except EOFError:
        print("", file=sys.stderr)
        return "denied"

    return "approved" if ans in ("y", "yes") else "denied"


def main() -> None:
    parser = argparse.ArgumentParser(prog="daedalus")
    sub = parser.add_subparsers(dest="command", required=True)

    ping_p = sub.add_parser("ping", help="send system.ping to daedalusd")
    ping_p.add_argument("--socket", default=DEFAULT_SOCK, help="UDS path")

    run_p = sub.add_parser("run", help="dispatch a task to daedalusd")
    run_p.add_argument("goal", help="task goal")
    run_p.add_argument("--agent", default=DEFAULT_AGENT, help="agent ID (default: %(default)s)")
    run_p.add_argument("--socket", default=DEFAULT_SOCK, help="UDS path")
    run_p.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help="timeout seconds")
    run_p.add_argument("--json", action="store_true", help="output DaedRunResult JSON")

    read_p = sub.add_parser("read-run", help="read a specific run's result")
    read_p.add_argument("run_id", help="run identifier")
    read_p.add_argument("--json", action="store_true", default=True, help="output JSON (always on for read-run)")

    list_p = sub.add_parser("list-runs", help="list recent runs")
    list_p.add_argument("--json", action="store_true", default=True, help="output JSON (always on for list-runs)")
    list_p.add_argument("--limit", type=int, default=10, help="max runs to return")

    args = parser.parse_args()

    if args.command == "ping":
        asyncio.run(_ping(args.socket))
    elif args.command == "run":
        asyncio.run(_run(args))
    elif args.command == "read-run":
        _read_run(args.run_id)
    elif args.command == "list-runs":
        _list_runs(args.limit)


async def _ping(sock_path: str) -> None:
    try:
        async with DaedalusClient(sock_path) as client:
            resp = await client.ping()
            print(json.dumps(resp, ensure_ascii=False))
    except DaedalusClientError as exc:
        msg = f"daemon error: [{exc.error}] {exc.detail}"
        print(msg, file=sys.stderr)
        sys.exit(1)
    except DaedalusTimeoutError as exc:
        print(f"timeout: {exc}", file=sys.stderr)
        sys.exit(1)
    except DaedalusConnectionError as exc:
        print(f"connection failed: {exc}", file=sys.stderr)
        sys.exit(1)
    except DaedalusError as exc:
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)


async def _run(args: argparse.Namespace) -> None:
    goal = args.goal.strip()
    if not goal:
        print("error: goal must not be empty", file=sys.stderr)
        sys.exit(2)

    agent = args.agent.strip()
    if not agent:
        print("error: --agent must not be empty", file=sys.stderr)
        sys.exit(2)

    timeout = args.timeout
    if timeout <= 0 or not math.isfinite(timeout):
        print("error: --timeout must be positive and finite", file=sys.stderr)
        sys.exit(2)

    task_id, task_card = build_task_card(goal, agent)
    assert task_id == task_card["task_card_id"]
    assert agent == task_card["execution_plan"]["primary_agent"]

    try:
        async with DaedalusClient(args.socket) as client:
            result = await client.dispatch(
                agent, task_id, task_card,
                permission_handler=_terminal_permission_handler,
                timeout=timeout,
            )
            if getattr(args, "json", False):
                done = result["done"]
                outbox = done.get("outbox", {})
                _print_daed_run_result_json(
                    run_id=done.get("run_id", "unknown"),  # daemon returns run_id in task.done
                    status="done",
                    summary=outbox.get("summary", ""),
                )
            else:
                print(json.dumps(result["done"], ensure_ascii=False))
    except DaedalusClientError as exc:
        if getattr(args, "json", False):
            _print_daed_run_result_json(
                run_id="unknown",
                status="error",
                error=f"[{exc.error}] {exc.detail}",
            )
            sys.exit(1)
        print(f"[{exc.error}] {exc.detail}", file=sys.stderr)
        sys.exit(1)
    except DaedalusTimeoutError as exc:
        if getattr(args, "json", False):
            _print_daed_run_result_json(
                run_id="unknown",
                status="running",
                error=f"timeout: {exc}",
            )
            sys.exit(1)
        print(f"timeout: {exc}", file=sys.stderr)
        sys.exit(1)
    except DaedalusConnectionError as exc:
        if getattr(args, "json", False):
            _print_daed_run_result_json(
                run_id="unknown",
                status="error",
                error=f"connection failed: {exc}",
            )
            sys.exit(1)
        print(f"connection failed: {exc}", file=sys.stderr)
        sys.exit(1)
    except DaedalusError as exc:
        if getattr(args, "json", False):
            _print_daed_run_result_json(
                run_id="unknown",
                status="error",
                error=str(exc),
            )
            sys.exit(1)
        print(f"error: {exc}", file=sys.stderr)
        sys.exit(1)


def _print_daed_run_result_json(
    run_id: str,
    status: str,
    summary: str = "",
    error: str = "",
) -> None:
    """Write a DaedRunResult to stdout."""
    result: dict[str, Any] = {
        "run_id": run_id,
        "status": status,
    }
    if summary:
        result["summary"] = summary
    if error:
        result["error"] = error
    print(json.dumps(result, ensure_ascii=False))


def _read_run(run_id: str) -> None:
    """Read a run's transcript and produce DaedReadResult JSON."""
    run_dir = Path(DEFAULT_RUNS_DIR) / run_id
    if not run_dir.is_dir():
        print(json.dumps({"error": "run not found", "status": "error"}, ensure_ascii=False))
        sys.exit(1)

    tpath = run_dir / "transcript.jsonl"
    spath = run_dir / "summary.md"
    apath = run_dir / "artifacts"

    status = "unknown"
    summary = ""
    error = ""
    goal = ""

    if tpath.exists():
        lines = tpath.read_text().strip().splitlines()
        for line in lines:
            try:
                evt = json.loads(line)
            except json.JSONDecodeError:
                continue
            tp = evt.get("type", "")
            if tp == "task.started":
                goal = evt.get("goal", goal)
            elif tp == "task.done":
                status = "done"
                summary = (evt.get("outbox", {}) or {}).get("summary", "")
            elif tp == "task.error":
                status = "error"
                error = evt.get("detail", "")
        if status == "unknown" and lines:
            status = "running"

    if spath.exists():
        summary = spath.read_text().strip()

    result: dict[str, Any] = {
        "run_id": run_id,
        "status": status,
    }
    if summary:
        result["summary"] = summary
    if error:
        result["error"] = error
    result["artifacts_path"] = str(apath)
    result["transcript_path"] = str(tpath)
    if goal:
        result["goal"] = goal

    print(json.dumps(result, ensure_ascii=False))


def _list_runs(limit: int) -> None:
    """List recent runs as DaedListResult JSON."""
    runs_dir = Path(DEFAULT_RUNS_DIR)
    entries: list[dict[str, Any]] = []

    if runs_dir.is_dir():
        for child in sorted(runs_dir.iterdir(), reverse=True):
            if not child.is_dir():
                continue
            run_id = child.name
            tpath = child / "transcript.jsonl"

            status = "unknown"
            created_at = ""
            goal = ""

            if tpath.exists():
                lines = tpath.read_text().strip().splitlines()
                for line in lines:
                    try:
                        evt = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    tp = evt.get("type", "")
                    if tp == "task.started":
                        created_at = evt.get("timestamp", "")
                        goal = evt.get("goal", "")
                    elif tp == "task.done":
                        status = "done"
                        break
                    elif tp == "task.error":
                        status = "error"
                        break
                if status == "unknown" and lines:
                    status = "running"

            entries.append({
                "run_id": run_id,
                "status": status,
                "created_at": created_at,
                "goal": goal,
            })

            if len(entries) >= limit:
                break

    print(json.dumps({"runs": entries}, ensure_ascii=False))


if __name__ == "__main__":
    main()
