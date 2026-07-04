"""Tests for DaedalusClient — unit tests with a minimal test server plus
cross-language integration with the real daedalusd binary."""

from __future__ import annotations

import asyncio
import json
import os
import subprocess
import sys
import tempfile
import time

import pytest

from daedalus.orch.client import (
    DaedalusClient,
    DaedalusClientError,
    DaedalusConnectionError,
    DaedalusProtocolError,
    DaedalusTimeoutError,
)


# ── minimal test server ────────────────────────────────────────────────


class _TestServer:
    """Bind a Unix socket, accept one connection, and send back *responses*
    (one per line).  After the list is exhausted the server closes."""

    def __init__(self, sock_path: str, responses: list[bytes]):
        self.sock_path = sock_path
        self.responses = responses
        self._server: asyncio.AbstractServer | None = None

    async def __aenter__(self) -> "_TestServer":
        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            try:
                await asyncio.wait_for(reader.readline(), timeout=2.0)
            except asyncio.TimeoutError:
                pass
            for resp in self.responses:
                writer.write(resp)
                await writer.drain()
            writer.close()
            await writer.wait_closed()

        self._server = await asyncio.start_unix_server(
            _handler, path=self.sock_path
        )
        return self

    async def __aexit__(self, *args):  # type: ignore[no-untyped-def]
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()


# ── helpers ────────────────────────────────────────────────────────────


def _pong_json(req_id: str) -> str:
    return json.dumps(
        {"type": "system.pong", "ts": "2026-06-16T10:00:00.100Z", "req_id": req_id}
    )


def _error_json(req_id: str | None, code: str, detail: str = "test detail") -> str:
    obj: dict = {
        "type": "system.error",
        "ts": "2026-06-16T10:00:00.200Z",
        "error": code,
        "detail": detail,
    }
    if req_id is not None:
        obj["req_id"] = req_id
    return json.dumps(obj)


# ── NDJSON encoding ────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_ndjson_single_line_encoding():
    """Client sends a valid single-line NDJSON ping message."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "enc.sock")
        captured: list[bytes] = []

        async def _cap(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            captured.append(line)
            writer.write(
                (
                    json.dumps({
                        "type": "system.pong",
                        "ts": "2026-06-16T10:00:00.000Z",
                        "req_id": "enc-test",
                    })
                    + "\n"
                ).encode()
            )
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_cap, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                resp = await client._request(
                    {
                        "type": "system.ping",
                        "ts": "2026-06-16T10:00:00.000Z",
                        "req_id": "enc-test",
                    },
                    req_id="enc-test",
                    expected_type="system.pong",
                    timeout=5.0,
                )
                assert resp["type"] == "system.pong"
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()

        assert len(captured) == 1
        raw = captured[0].decode()
        assert "\n" in raw
        obj = json.loads(raw.strip())
        assert obj["type"] == "system.ping"
        assert obj["req_id"] == "enc-test"
        assert isinstance(obj["ts"], str)


# ── connection failure ─────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_connection_failure():
    """Connecting to a path that does not exist raises ConnectionError."""
    with tempfile.TemporaryDirectory() as tmp:
        missing = os.path.join(tmp, "no-such-socket")
        with pytest.raises(DaedalusConnectionError):
            client = DaedalusClient(missing)
            await client.connect()


# ── malformed response ─────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_malformed_response():
    """Server returns non-JSON."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "mal.sock")
        async with _TestServer(sock, [b"not json at all\n"]):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusProtocolError, match="malformed"):
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "r1",
                        },
                        req_id="r1",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
            finally:
                await client.close()


@pytest.mark.asyncio
async def test_response_not_object():
    """Server returns JSON that is not an object."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "arr.sock")
        async with _TestServer(sock, [b'["not", "object"]\n']):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusProtocolError, match="not a JSON object"):
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "r1",
                        },
                        req_id="r1",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
            finally:
                await client.close()


# ── system.error ───────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_system_error_response():
    """Server returns a valid system.error — must raise DaedalusClientError."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "err.sock")
        async with _TestServer(
            sock,
            [(_error_json("r-err", "unknown_message_type") + "\n").encode()],
        ):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError) as exc_info:
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "r-err",
                        },
                        req_id="r-err",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
                assert exc_info.value.error == "unknown_message_type"
                assert exc_info.value.detail == "test detail"
                assert exc_info.value.req_id == "r-err"
            finally:
                await client.close()


@pytest.mark.asyncio
async def test_system_error_without_req_id():
    """system.error missing req_id → DaedalusClientError with req_id=None."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "err-noid.sock")
        async with _TestServer(
            sock,
            [(_error_json(None, "malformed_json", "bad input") + "\n").encode()],
        ):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError) as exc_info:
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "r-noid",
                        },
                        req_id="r-noid",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
                assert exc_info.value.error == "malformed_json"
                assert exc_info.value.req_id is None
            finally:
                await client.close()


@pytest.mark.asyncio
async def test_system_error_null_req_id():
    """system.error with req_id:null → DaedalusClientError with req_id=None."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "err-null.sock")
        # Build JSON with explicit null req_id.
        payload = json.dumps({
            "type": "system.error",
            "ts": "2026-06-16T10:00:00.200Z",
            "req_id": None,
            "error": "missing_type",
            "detail": "no type field",
        })
        async with _TestServer(sock, [(payload + "\n").encode()]):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError) as exc_info:
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "r-null",
                        },
                        req_id="r-null",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
                assert exc_info.value.error == "missing_type"
                assert exc_info.value.req_id is None
            finally:
                await client.close()


@pytest.mark.asyncio
async def test_system_error_req_id_mismatch():
    """system.error with mismatched req_id → DaedalusProtocolError."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "err-mism.sock")
        async with _TestServer(
            sock,
            [(_error_json("wrong", "unknown_message_type") + "\n").encode()],
        ):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusProtocolError, match="req_id mismatch"):
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "right",
                        },
                        req_id="right",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
            finally:
                await client.close()


# ── req_id mismatch (on pong) ──────────────────────────────────────────


@pytest.mark.asyncio
async def test_req_id_mismatch():
    """Response req_id differs from request."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "mism.sock")
        async with _TestServer(
            sock, [(_pong_json("wrong-id") + "\n").encode()]
        ):
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusProtocolError, match="req_id mismatch"):
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "right-id",
                        },
                        req_id="right-id",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
            finally:
                await client.close()


# ── EOF ────────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_eof_no_response():
    """Server closes without sending any data."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "eof.sock")

        async def _close(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            writer.close()
            await writer.wait_closed()

        srv = await asyncio.start_unix_server(_close, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusConnectionError, match="closed connection"):
                    await client._request(
                        {
                            "type": "system.ping",
                            "ts": "2026-06-16T10:00:00.000Z",
                            "req_id": "eof-1",
                        },
                        req_id="eof-1",
                        expected_type="system.pong",
                        timeout=5.0,
                    )
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()


# ── timeout ────────────────────────────────────────────────────────────


@pytest.mark.asyncio
async def test_timeout():
    """Request times out when the reader never delivers a response line."""
    client = DaedalusClient.__new__(DaedalusClient)
    client.sock_path = "/fake"

    class _FakeWriter:
        def write(self, _data: bytes) -> None:
            pass

        async def drain(self) -> None:
            pass

        def close(self) -> None:
            pass

        async def wait_closed(self) -> None:
            pass

    class _SlowReader:
        async def readline(self) -> bytes:
            await asyncio.sleep(999)
            return b""

    client._reader = _SlowReader()  # type: ignore[assignment]
    client._writer = _FakeWriter()  # type: ignore[assignment]

    with pytest.raises(DaedalusTimeoutError):
        await client._request(
            {
                "type": "system.ping",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": "to-1",
            },
            req_id="to-1",
            expected_type="system.pong",
            timeout=0.2,
        )


# ── CLI ────────────────────────────────────────────────────────────────

_DAEDALUS_BIN = os.path.join(os.path.dirname(sys.executable), "daedalus")


def test_cli_help():
    """`daedalus --help` exits 0 and mentions ping."""
    if not os.path.exists(_DAEDALUS_BIN):
        pytest.skip(f"daedalus binary not found at {_DAEDALUS_BIN}")
    result = subprocess.run(
        [_DAEDALUS_BIN, "--help"],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 0
    assert "ping" in result.stdout


def test_cli_missing_command():
    """`daedalus` with no subcommand exits non-zero."""
    if not os.path.exists(_DAEDALUS_BIN):
        pytest.skip(f"daedalus binary not found at {_DAEDALUS_BIN}")
    result = subprocess.run(
        [_DAEDALUS_BIN],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode != 0


def test_cli_ping_socket_not_found():
    """`daedalus ping` to a missing socket exits 1."""
    if not os.path.exists(_DAEDALUS_BIN):
        pytest.skip(f"daedalus binary not found at {_DAEDALUS_BIN}")
    result = subprocess.run(
        [_DAEDALUS_BIN, "ping", "--socket", "/tmp/no-daemon-here.sock"],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert result.returncode == 1
    assert (
        "connection failed" in result.stderr.lower()
        or "connection" in result.stderr.lower()
    )


# ── cross-language: real Rust daemon ───────────────────────────────────


@pytest.mark.asyncio
async def test_real_daemon_ping_pong():
    """Start a real daedalusd, ping it, and verify the pong response."""
    daemon_bin = os.path.join(
        os.path.dirname(__file__), "..", "..", "target", "debug", "daedalusd"
    )
    if not os.path.exists(daemon_bin):
        pytest.skip(f"daedalusd binary not found at {daemon_bin} (run `cargo build` first)")

    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "daedalusd.sock")

        proc = subprocess.Popen(
            [daemon_bin],
            env={**os.environ, "DAEDALUSD_SOCK": sock, "DAEDALUSD_HTTP_ADDR": "127.0.0.1:0"},
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        try:
            deadline = time.monotonic() + 10
            while not os.path.exists(sock):
                if time.monotonic() > deadline:
                    proc.kill()
                    proc.wait()
                    err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
                    pytest.fail(f"daedalusd did not create socket within 10s\nstderr: {err}")
                await asyncio.sleep(0.1)

            client = DaedalusClient(sock)
            await client.connect()
            try:
                resp = await client.ping(timeout=5.0)
                assert resp["type"] == "system.pong"
                assert "req_id" in resp
                assert "ts" in resp
            finally:
                await client.close()

        finally:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()

            await asyncio.sleep(0.2)
            assert not os.path.exists(sock), "daemon did not clean up socket"


# ── P2.5: dispatch() prototype ─────────────────────────────────────────


@pytest.mark.asyncio
async def test_dispatch_send_and_receive_error():
    """dispatch() sends task.dispatch; system.error raises DaedalusClientError."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "disp.sock")

        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            obj = json.loads(line)
            assert obj["type"] == "task.dispatch"
            assert obj["agent_id"] == "claude"
            assert obj["task_id"] == "t-disp"
            # P2.5: daemon returns system.error (not implemented).
            resp = json.dumps({
                "type": "system.error",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": obj["req_id"],
                "error": "invalid_message",
                "detail": "task.dispatch not implemented in Phase 2",
            })
            writer.write((resp + "\n").encode())
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_handler, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError) as exc_info:
                    await client.dispatch(
                        "claude", "t-disp", {"schema_version": "2.8"}, timeout=5.0
                    )
                assert exc_info.value.error == "invalid_message"
                assert "not implemented" in exc_info.value.detail
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()


@pytest.mark.asyncio
async def test_dispatch_success_response():
    """dispatch() reads task.stream first, then task.done — returns {done, streams}."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "disp2.sock")

        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            disp = json.loads(line)
            assert disp["type"] == "task.dispatch"
            rid = disp["req_id"]

            # Send a task.stream first.
            stream = json.dumps({
                "type": "task.stream",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": rid,
                "agent_id": "claude",
                "task_id": "t1",
                "chunk": "hello",
            })
            writer.write((stream + "\n").encode())
            await writer.drain()

            # Then send task.done.
            done = json.dumps({
                "type": "task.done",
                "ts": "2026-06-16T10:00:01.000Z",
                "req_id": rid,
                "agent_id": "claude",
                "task_id": "t1",
                "outbox": {
                    "schema_version": "2.8",
                    "task_id": "t1",
                    "agent_id": "claude",
                    "status": "waiting_for_verification",
                    "summary": "done",
                    "changed_files": [],
                    "verification": {},
                    "evidence": {},
                    "known_risks": [],
                },
            })
            writer.write((done + "\n").encode())
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_handler, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                result = await client.dispatch(
                    "claude", "t1", {"schema_version": "2.8"}, timeout=5.0
                )
                # P2.7: returns {"done": ..., "streams": [...]}
                assert isinstance(result, dict)
                assert "done" in result
                assert "streams" in result
                assert result["done"]["type"] == "task.done"
                assert len(result["streams"]) == 1
                assert result["streams"][0]["chunk"] == "hello"
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()


# ── P2.5: permission handler ───────────────────────────────────────────


@pytest.mark.asyncio
async def test_permission_handler_approved():
    """dispatch() with a permission_handler that returns 'approved'."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "perm.sock")

        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            # Read the task.dispatch.
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            disp = json.loads(line)
            assert disp["type"] == "task.dispatch"

            # Send a permission.request.
            req = json.dumps({
                "type": "permission.request",
                "ts": "2026-06-16T10:00:00.000Z",
                "permission_id": "perm-1",
                "req_id": disp["req_id"],
                "agent_id": "claude",
                "tool": "bash",
                "args": {"cmd": "ls"},
            })
            writer.write((req + "\n").encode())
            await writer.drain()

            # Read the permission.response — must be "approved".
            resp = await asyncio.wait_for(reader.readline(), timeout=2.0)
            pr = json.loads(resp)
            assert pr["type"] == "permission.response"
            assert pr["decision"] == "approved"
            assert pr["permission_id"] == "perm-1"

            # Send terminal system.error to end dispatch loop.
            term = json.dumps({
                "type": "system.error",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": disp["req_id"],
                "error": "invalid_message",
                "detail": "task.dispatch not implemented in Phase 2",
            })
            writer.write((term + "\n").encode())
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_handler, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError) as exc_info:
                    await client.dispatch(
                        "claude",
                        "t1",
                        {"schema_version": "2.8"},
                        permission_handler=lambda _req: "approved",
                        timeout=5.0,
                    )
                assert exc_info.value.error == "invalid_message"
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()


@pytest.mark.asyncio
async def test_permission_handler_denied():
    """dispatch() with a permission_handler that returns 'denied'."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "perm-deny.sock")

        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            disp = json.loads(line)
            assert disp["type"] == "task.dispatch"

            # Send permission.request.
            req = json.dumps({
                "type": "permission.request",
                "ts": "2026-06-16T10:00:00.000Z",
                "permission_id": "perm-2",
                "req_id": disp["req_id"],
                "agent_id": "claude",
                "tool": "bash",
                "args": {"cmd": "rm -rf /"},
            })
            writer.write((req + "\n").encode())
            await writer.drain()

            # Read response — must be "denied".
            resp = await asyncio.wait_for(reader.readline(), timeout=2.0)
            pr = json.loads(resp)
            assert pr["decision"] == "denied"

            # Send terminal.
            term = json.dumps({
                "type": "system.error",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": disp["req_id"],
                "error": "invalid_message",
                "detail": "not implemented",
            })
            writer.write((term + "\n").encode())
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_handler, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError):
                    await client.dispatch(
                        "claude",
                        "t1",
                        {"schema_version": "2.8"},
                        permission_handler=lambda _req: "denied",
                        timeout=5.0,
                    )
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()


@pytest.mark.asyncio
async def test_permission_no_handler_default_denied():
    """dispatch() without handler auto-answers 'denied' to permission.request."""
    with tempfile.TemporaryDirectory() as tmp:
        sock = os.path.join(tmp, "perm-noh.sock")

        async def _handler(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
            line = await asyncio.wait_for(reader.readline(), timeout=2.0)
            disp = json.loads(line)

            # Send permission.request.
            req = json.dumps({
                "type": "permission.request",
                "ts": "2026-06-16T10:00:00.000Z",
                "permission_id": "perm-3",
                "req_id": disp["req_id"],
                "agent_id": "claude",
                "tool": "bash",
                "args": {},
            })
            writer.write((req + "\n").encode())
            await writer.drain()

            # Read response — must be "denied" (default).
            resp = await asyncio.wait_for(reader.readline(), timeout=2.0)
            pr = json.loads(resp)
            assert pr["type"] == "permission.response"
            assert pr["permission_id"] == "perm-3"
            assert pr["decision"] == "denied"

            # Send terminal.
            term = json.dumps({
                "type": "system.error",
                "ts": "2026-06-16T10:00:00.000Z",
                "req_id": disp["req_id"],
                "error": "invalid_message",
                "detail": "not implemented",
            })
            writer.write((term + "\n").encode())
            await writer.drain()
            writer.close()

        srv = await asyncio.start_unix_server(_handler, path=sock)
        try:
            client = DaedalusClient(sock)
            await client.connect()
            try:
                with pytest.raises(DaedalusClientError):
                    await client.dispatch(
                        "claude",
                        "t1",
                        {"schema_version": "2.8"},
                        # No permission_handler → auto "denied"
                        timeout=5.0,
                    )
            finally:
                await client.close()
        finally:
            srv.close()
            await srv.wait_closed()
