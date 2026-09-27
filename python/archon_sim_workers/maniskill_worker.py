#!/usr/bin/env python3
"""ManiSkill NDJSON worker stub (M2b). Same protocol as mujoco_worker."""

from __future__ import annotations

import sys
import traceback
from pathlib import Path

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from protocol import PROTOCOL_VERSION, now_us, read_msg, write_error, write_msg  # noqa: E402


def main() -> int:
    try:
        hello = read_msg()
        if hello is None or hello.get("type") != "hello":
            write_error("expected hello")
            return 1
        # Intentionally stubbed until M2b implementation.
        write_error(
            "ManiSkill worker not implemented yet (M2b). "
            "Use --backend mujoco or --backend sim. "
            "See notes/design/m2-multi-sim-adapters.md"
        )
        return 1
        # Unreachable placeholder for future hello_ok:
        # write_msg({"type": "hello_ok", "protocol_version": PROTOCOL_VERSION, "platform": "maniskill"})
    except Exception as e:
        write_error(f"{e}\n{traceback.format_exc()}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
