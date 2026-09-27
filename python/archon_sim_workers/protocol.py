"""Shared helpers for Archon sim workers (NDJSON protocol v1)."""

from __future__ import annotations

import json
import sys
import time
from typing import Any, Dict, Optional


PROTOCOL_VERSION = 1


def now_us() -> int:
    return int(time.time() * 1_000_000)


def read_msg() -> Optional[Dict[str, Any]]:
    line = sys.stdin.readline()
    if not line:
        return None
    line = line.strip()
    if not line:
        return read_msg()
    return json.loads(line)


def write_msg(obj: Dict[str, Any]) -> None:
    sys.stdout.write(json.dumps(obj, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def write_error(message: str) -> None:
    write_msg({"type": "error", "message": message})


def write_ppm(path: str, width: int, height: int, rgb: bytes) -> None:
    import os

    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "wb") as f:
        f.write(f"P6\n{width} {height}\n255\n".encode("ascii"))
        f.write(rgb)
