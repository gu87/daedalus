"""Minimal exception hierarchy for the daedalusd UDS client."""


class DaedalusError(Exception):
    """Base for all client-side errors."""


class DaedalusConnectionError(DaedalusError):
    """Connection or I/O failure (socket missing, broken pipe, EOF)."""


class DaedalusTimeoutError(DaedalusError):
    """Request did not complete within the deadline."""


class DaedalusProtocolError(DaedalusError):
    """Response violated the NDJSON protocol contract."""


class DaedalusClientError(DaedalusError):
    """Server responded with `system.error`."""

    def __init__(self, error: str, detail: str, req_id: str | None) -> None:
        super().__init__(f"[{error}] {detail}")
        self.error = error
        self.detail = detail
        self.req_id = req_id
