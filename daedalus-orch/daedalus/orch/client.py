"""Async UDS client for daedalusd — NDJSON over Unix domain socket."""

from __future__ import annotations

import asyncio
import json
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any

from daedalus.orch.exceptions import (
    DaedalusClientError,
    DaedalusConnectionError,
    DaedalusProtocolError,
    DaedalusTimeoutError,
)


# ── helpers ────────────────────────────────────────────────────────────


def _now_utc() -> str:
    """RFC 3339 UTC timestamp with millisecond precision."""
    ts = datetime.now(timezone.utc).isoformat(timespec="milliseconds")
    return ts.replace("+00:00", "Z")


# ── client ─────────────────────────────────────────────────────────────


@dataclass
class DaedalusClient:
    """Async UDS connection to a running daedalusd."""

    sock_path: str
    _reader: asyncio.StreamReader | None = None
    _writer: asyncio.StreamWriter | None = None

    # -- lifecycle -------------------------------------------------------

    async def connect(self) -> None:
        """Open the Unix domain socket connection."""
        try:
            reader, writer = await asyncio.open_unix_connection(self.sock_path)
        except (FileNotFoundError, ConnectionRefusedError, OSError) as exc:
            raise DaedalusConnectionError(
                f"cannot connect to {self.sock_path}: {exc}"
            ) from exc
        self._reader = reader
        self._writer = writer

    async def close(self) -> None:
        """Close the connection gracefully."""
        if self._writer is not None:
            self._writer.close()
            try:
                await self._writer.wait_closed()
            except OSError:
                pass
        self._reader = None
        self._writer = None

    async def __aenter__(self) -> "DaedalusClient":
        await self.connect()
        return self

    async def __aexit__(self, *args: Any) -> None:
        await self.close()

    # -- public API ------------------------------------------------------

    async def ping(self, timeout: float = 30.0) -> dict[str, Any]:
        """Send `system.ping` and return the `system.pong` payload."""
        req_id = uuid.uuid4().hex
        msg = {
            "type": "system.ping",
            "ts": _now_utc(),
            "req_id": req_id,
        }
        resp = await self._request(msg, req_id, expected_type="system.pong", timeout=timeout)
        return resp

    # -- internals -------------------------------------------------------

    async def _request(
        self,
        msg: dict[str, Any],
        req_id: str,
        expected_type: str,
        timeout: float,
    ) -> dict[str, Any]:
        if self._reader is None or self._writer is None:
            raise DaedalusConnectionError("not connected")
        line = json.dumps(msg, ensure_ascii=False) + "\n"

        try:
            async with asyncio.timeout(timeout):
                # Write.
                self._writer.write(line.encode("utf-8"))
                await self._writer.drain()
                # Read one line.
                raw = await self._reader.readline()
        except asyncio.TimeoutError:
            raise DaedalusTimeoutError(
                f"request {req_id} timed out after {timeout}s"
            ) from None
        except OSError as exc:
            raise DaedalusConnectionError(f"I/O error on {req_id}: {exc}") from exc

        if not raw:
            raise DaedalusConnectionError(f"daemon closed connection before responding to {req_id}")

        return _validate_response(raw, req_id, expected_type)


# ── response validation ────────────────────────────────────────────────


def _validate_response(
    raw: bytes,
    req_id: str,
    expected_type: str,
) -> dict[str, Any]:
    # 1. Parse JSON.
    try:
        text = raw.decode("utf-8")
        obj = json.loads(text)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise DaedalusProtocolError(f"malformed response (not JSON): {exc!r}") from exc

    # 2. Must be a JSON object.
    if not isinstance(obj, dict):
        raise DaedalusProtocolError("response is not a JSON object")

    # 3. Must have a string 'type'.
    msg_type = obj.get("type")
    if not isinstance(msg_type, str):
        raise DaedalusProtocolError(f"response missing or non-string 'type': {msg_type!r}")

    # 4. Must have a string 'ts'.
    if not isinstance(obj.get("ts"), str):
        raise DaedalusProtocolError("response missing or non-string 'ts'")

    # 5. Route by type — req_id constraints differ per message type.
    if msg_type == expected_type:
        # system.pong: req_id is required, non-empty, and must match.
        resp_req_id = obj.get("req_id")
        if not isinstance(resp_req_id, str) or not resp_req_id:
            raise DaedalusProtocolError(
                f"{msg_type} missing or empty 'req_id': {resp_req_id!r}"
            )
        if resp_req_id != req_id:
            raise DaedalusProtocolError(
                f"req_id mismatch: expected {req_id}, got {resp_req_id}"
            )
        return obj

    if msg_type == "system.error":
        # system.error: error + detail are required strings.
        error_code = obj.get("error")
        detail = obj.get("detail", "")
        if not isinstance(error_code, str) or not isinstance(detail, str):
            raise DaedalusProtocolError(
                "system.error missing or invalid 'error'/'detail' fields"
            )
        # req_id is optional on system.error.
        resp_req_id = obj.get("req_id")
        if resp_req_id is None:
            # Absent or JSON null → no association.
            raise DaedalusClientError(
                error=error_code, detail=detail, req_id=None
            )
        if not isinstance(resp_req_id, str) or not resp_req_id:
            raise DaedalusProtocolError(
                f"system.error 'req_id' must be non-empty when present: {resp_req_id!r}"
            )
        if resp_req_id != req_id:
            raise DaedalusProtocolError(
                f"req_id mismatch: expected {req_id}, got {resp_req_id}"
            )
        raise DaedalusClientError(
            error=error_code, detail=detail, req_id=resp_req_id
        )

    raise DaedalusProtocolError(f"unexpected response type: {msg_type}")
