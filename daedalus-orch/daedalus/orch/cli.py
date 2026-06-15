"""Minimal CLI entry point for daedalus-orch."""

from __future__ import annotations

import argparse
import asyncio
import json
import sys

from daedalus.orch.client import DaedalusClient
from daedalus.orch.exceptions import (
    DaedalusClientError,
    DaedalusConnectionError,
    DaedalusError,
    DaedalusTimeoutError,
)

DEFAULT_SOCK = "/tmp/daedalusd.sock"


def main() -> None:
    parser = argparse.ArgumentParser(prog="daedalus")
    sub = parser.add_subparsers(dest="command", required=True)

    ping_p = sub.add_parser("ping", help="send system.ping to daedalusd")
    ping_p.add_argument("--socket", default=DEFAULT_SOCK, help="UDS path")

    args = parser.parse_args()

    if args.command == "ping":
        asyncio.run(_ping(args.socket))


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


if __name__ == "__main__":
    main()
