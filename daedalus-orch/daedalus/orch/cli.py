"""Minimal CLI entry point for daedalus-orch."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
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
DEFAULT_AGENT = "daedalus-desktop"
DEFAULT_TIMEOUT = 300.0


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
    except (EOFError, KeyboardInterrupt):
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
    run_p.add_argument("--socket", default=DEFAULT_SOCK, help="UDS path (default: %(default)s)")
    run_p.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT, help="timeout seconds (default: %(default)s)")

    args = parser.parse_args()

    if args.command == "ping":
        asyncio.run(_ping(args.socket))
    elif args.command == "run":
        asyncio.run(_run(args))


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
    if timeout <= 0:
        print("error: --timeout must be positive", file=sys.stderr)
        sys.exit(2)

    task_id, task_card = build_task_card(goal, agent)
    # Contract: task_id == task_card["task_card_id"] AND agent == task_card["execution_plan"]["primary_agent"]
    assert task_id == task_card["task_card_id"]
    assert agent == task_card["execution_plan"]["primary_agent"]

    try:
        async with DaedalusClient(
            args.socket, on_permission=_terminal_permission_handler
        ) as client:
            result = await client.dispatch(
                agent, task_id, task_card, timeout=timeout,
            )
            # Output task.done JSON directly.
            print(json.dumps(result["done"], ensure_ascii=False))
    except DaedalusClientError as exc:
        print(f"[{exc.error}] {exc.detail}", file=sys.stderr)
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


if __name__ == "__main__":
    main()
