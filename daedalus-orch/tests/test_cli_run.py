"""P5+.6: daedalus run CLI tests — mock UDS daemon via asyncio unix server."""

import asyncio
import json
import os
import subprocess
import sys
import tempfile
from unittest import mock

from daedalus.orch.task_card import build_task_card


# ── task_card unit tests ──────────────────────────────────────────────

def test_task_card_ids_consistent():
    task_id, tc = build_task_card("test", "daedalus-desktop")
    assert task_id == tc["task_card_id"]
    assert tc["execution_plan"]["primary_agent"] == "daedalus-desktop"
    assert tc["schema_version"] == "2.8"
    assert tc["allowed_files"] == []
    assert tc["safety"]["allowed_paths"] == []


def test_task_card_project():
    _, tc = build_task_card("hi", "ag")
    assert tc["project"] == "daedalus-cli"


# ── CLI integration tests (mock daemon) ───────────────────────────────

def _run_cli(*args, stdin="", env=None):
    full_env = {**os.environ, "PYTHONPATH": "daedalus-orch", **(env or {})}
    return subprocess.run(
        [sys.executable, "-m", "daedalus.orch.cli", *args],
        capture_output=True, text=True, input=stdin, env=full_env, timeout=30,
    )


def test_run_task_done():
    """Mock daemon returns task.done."""
    sock_dir = tempfile.mkdtemp()
    sock_path = os.path.join(sock_dir, "test.sock")

    async def handler(reader, writer):
        buf = b""
        while True:
            chunk = await reader.read(4096)
            if not chunk: break
            buf += chunk
            if b"\n" in buf:
                nl = buf.index(b"\n")
                line = buf[:nl].decode()
                msg = json.loads(line)
                if msg.get("type") == "task.dispatch":
                    writer.write(json.dumps({
                        "type": "task.done", "ts": "2026-01-01T00:00:00.000Z",
                        "req_id": msg["req_id"], "agent_id": msg["agent_id"],
                        "task_id": msg["task_id"],
                        "outbox": {"summary": "ok", "task_id": msg["task_id"], "agent_id": msg["agent_id"]},
                    }).encode() + b"\n")
                    await writer.drain()
                    return

    async def _run():
        server = await asyncio.start_unix_server(handler, sock_path)
        await asyncio.sleep(0.1)  # ensure server is listening
        import concurrent.futures
        loop = asyncio.get_running_loop()
        with concurrent.futures.ThreadPoolExecutor() as pool:
            result = await loop.run_in_executor(
                pool, lambda: _run_cli("run", "test goal", "--socket", sock_path))
        server.close()
        await server.wait_closed()
        assert result.returncode == 0
        assert "outbox" in result.stdout
    asyncio.run(_run())


def test_run_task_error():
    """Mock daemon returns task.error."""
    sock_dir = tempfile.mkdtemp()
    sock_path = os.path.join(sock_dir, "test.sock")

    async def handler(reader, writer):
        buf = b""
        while True:
            chunk = await reader.read(4096)
            if not chunk: break
            buf += chunk
            if b"\n" in buf:
                nl = buf.index(b"\n")
                line = buf[:nl].decode()
                msg = json.loads(line)
                if msg.get("type") == "task.dispatch":
                    writer.write(json.dumps({
                        "type": "task.error", "ts": "2026-01-01T00:00:00.000Z",
                        "req_id": msg["req_id"], "agent_id": msg["agent_id"],
                        "task_id": msg["task_id"],
                        "error_taxonomy": "tool_failure", "detail": "boom",
                    }).encode() + b"\n")
                    await writer.drain()
                    return

    async def _run():
        server = await asyncio.start_unix_server(handler, sock_path)
        await asyncio.sleep(0.1)
        import concurrent.futures
        loop = asyncio.get_running_loop()
        with concurrent.futures.ThreadPoolExecutor() as pool:
            result = await loop.run_in_executor(
                pool, lambda: _run_cli("run", "test", "--socket", sock_path))
        server.close()
        await server.wait_closed()
        assert result.returncode == 1
        assert "tool_failure" in result.stderr
    asyncio.run(_run())


def test_run_connection_failed():
    result = _run_cli("run", "test", "--socket", "/tmp/no-such-sock-xyz")
    assert result.returncode == 1
    assert "connection" in result.stderr.lower() or "cannot connect" in result.stderr.lower()


def test_permission_tty_approved():
    from daedalus.orch.cli import _terminal_permission_handler
    with mock.patch("sys.stdin.isatty", return_value=True), \
         mock.patch("builtins.input", return_value="y"):
        assert _terminal_permission_handler({"tool": "test", "args": {"x": 1}}) == "approved"


def test_permission_tty_default_denied():
    from daedalus.orch.cli import _terminal_permission_handler
    with mock.patch("sys.stdin.isatty", return_value=True), \
         mock.patch("builtins.input", return_value=""):
        assert _terminal_permission_handler({"tool": "test", "args": {"x": 1}}) == "denied"


def test_permission_non_tty_auto_denied():
    from daedalus.orch.cli import _terminal_permission_handler
    with mock.patch("sys.stdin.isatty", return_value=False):
        assert _terminal_permission_handler({"tool": "test", "args": {"x": 1}}) == "denied"


def test_arg_defaults():
    result = _run_cli("run", "--help")
    assert result.returncode == 0
    assert "daedalus-desktop" in result.stdout


def test_arg_empty_goal_rejected():
    result = _run_cli("run", "   ", "--socket", "/tmp/test.sock")
    assert result.returncode == 2


def test_arg_negative_timeout_rejected():
    result = _run_cli("run", "test", "--timeout", "-1", "--socket", "/tmp/test.sock")
    assert result.returncode == 2


def test_arg_timeout_nan_rejected():
    result = _run_cli("run", "test", "--timeout", "nan", "--socket", "/tmp/test.sock")
    assert result.returncode == 2


def test_arg_empty_agent_rejected():
    result = _run_cli("run", "test", "--agent", "", "--socket", "/tmp/test.sock")
    assert result.returncode == 2


def test_run_permission_non_tty_auto_denied_flow():
    """Mock daemon sends permission.request → non-TTY → auto-denied → task.done."""
    sock_dir = tempfile.mkdtemp()
    sock_path = os.path.join(sock_dir, "test.sock")

    async def handler(reader, writer):
        buf = b""
        while True:
            chunk = await reader.read(4096)
            if not chunk: break
            buf += chunk
            while b"\n" in buf:
                nl = buf.index(b"\n")
                line = buf[:nl].decode()
                buf = buf[nl + 1:]
                msg = json.loads(line)
                t = msg.get("type")
                if t == "task.dispatch":
                    writer.write(json.dumps({
                        "type": "permission.request",
                        "ts": "2026-01-01T00:00:00.000Z",
                        "permission_id": "perm-2",
                        "req_id": msg["req_id"],
                        "agent_id": msg["agent_id"],
                        "tool": "terminal",
                        "args": {"cmd": "rm"},
                    }).encode() + b"\n")
                    await writer.drain()
                    while True:
                        chunk2 = await reader.read(4096)
                        if not chunk2: return
                        buf += chunk2
                        if b"\n" in buf:
                            nl2 = buf.index(b"\n")
                            line2 = buf[:nl2].decode()
                            buf = buf[nl2 + 1:]
                            resp = json.loads(line2)
                            if resp.get("type") == "permission.response":
                                assert resp["decision"] == "denied"
                                writer.write(json.dumps({
                                    "type": "task.done",
                                    "ts": "2026-01-01T00:00:00.000Z",
                                    "req_id": msg["req_id"],
                                    "agent_id": msg["agent_id"],
                                    "task_id": msg["task_id"],
                                    "outbox": {"summary": "denied-ok", "task_id": msg["task_id"], "agent_id": msg["agent_id"]},
                                }).encode() + b"\n")
                                await writer.drain()
                                return

    async def _run():
        server = await asyncio.start_unix_server(handler, sock_path)
        await asyncio.sleep(0.1)
        import concurrent.futures
        loop = asyncio.get_running_loop()
        with concurrent.futures.ThreadPoolExecutor() as pool:
            # No stdin → non-TTY, auto-deny.
            result = await loop.run_in_executor(
                pool, lambda: _run_cli("run", "test", "--socket", sock_path))
        server.close()
        await server.wait_closed()
        assert result.returncode == 0
    asyncio.run(_run())

def test_run_permission_tty_approved_flow():
    """In-process integration: monkeypatch TTY → approved → task.done."""
    import argparse as ap
    import io as _io

    sock_dir = tempfile.mkdtemp()
    sock_path = os.path.join(sock_dir, "test.sock")

    import daedalus.orch.cli as cli_mod

    async def handler(reader, writer):
        buf = b""
        while True:
            chunk = await reader.read(4096)
            if not chunk: break
            buf += chunk
            while b"\n" in buf:
                nl = buf.index(b"\n")
                line = buf[:nl].decode()
                buf = buf[nl + 1:]
                msg = json.loads(line)
                t = msg.get("type")
                if t == "task.dispatch":
                    writer.write(json.dumps({
                        "type": "permission.request",
                        "ts": "2026-01-01T00:00:00.000Z",
                        "permission_id": "perm-tty-1",
                        "req_id": msg["req_id"],
                        "agent_id": msg["agent_id"],
                        "tool": "terminal",
                        "args": {"cmd": "echo hi"},
                    }).encode() + b"\n")
                    await writer.drain()
                    while True:
                        chunk2 = await reader.read(4096)
                        if not chunk2: return
                        buf += chunk2
                        if b"\n" in buf:
                            nl2 = buf.index(b"\n")
                            line2 = buf[:nl2].decode()
                            buf = buf[nl2 + 1:]
                            resp = json.loads(line2)
                            if resp.get("type") == "permission.response":
                                assert resp["decision"] == "approved", f"expected approved, got {resp}"
                                writer.write(json.dumps({
                                    "type": "task.done",
                                    "ts": "2026-01-01T00:00:00.000Z",
                                    "req_id": msg["req_id"],
                                    "agent_id": msg["agent_id"],
                                    "task_id": msg["task_id"],
                                    "outbox": {"summary": "tty-ok", "task_id": msg["task_id"], "agent_id": msg["agent_id"]},
                                }).encode() + b"\n")
                                await writer.drain()
                                return

    async def _run():
        server = await asyncio.start_unix_server(handler, sock_path)
        await asyncio.sleep(0.1)

        with mock.patch("sys.stdin.isatty", return_value=True), \
             mock.patch("builtins.input", return_value="y"):
            args = ap.Namespace(
                goal="test", agent="daedalus-desktop",
                socket=sock_path, timeout=30.0,
            )
            old_stdout = sys.stdout
            sys.stdout = _io.StringIO()
            try:
                exit_code = 0
                try:
                    await cli_mod._run(args)
                except SystemExit as e:
                    exit_code = e.code
                output = sys.stdout.getvalue()
                assert exit_code == 0, f"exit={exit_code}, output={output}"
                assert "outbox" in output
            finally:
                sys.stdout = old_stdout

        server.close()
        await server.wait_closed()
    asyncio.run(_run())


def test_arg_timeout_zero_rejected():
    result = _run_cli("run", "test", "--timeout", "0", "--socket", "/tmp/test.sock")
    assert result.returncode == 2
