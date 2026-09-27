#!/usr/bin/env python3
"""Echo/mock worker for bridge integration tests (no MuJoCo)."""

from __future__ import annotations

import sys
from pathlib import Path

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from protocol import PROTOCOL_VERSION, now_us, read_msg, write_error, write_msg  # noqa: E402


class State:
    def __init__(self) -> None:
        self.names = [f"joint_{i}" for i in range(1, 7)]
        self.pos = [0.0] * 6
        self.grip = 0.5

    def obs(self):
        return {
            "type": "observation",
            "stamp_us": now_us(),
            "proprio": {
                "joint_names": self.names,
                "positions": list(self.pos),
                "gripper_open": self.grip,
            },
            "modalities": [],
        }


def main() -> int:
    st = State()
    hello = read_msg()
    if not hello or hello.get("type") != "hello":
        write_error("expected hello")
        return 1
    platform = hello.get("platform") or "mock"
    write_msg(
        {
            "type": "hello_ok",
            "protocol_version": PROTOCOL_VERSION,
            "platform": platform,
            "joint_names": st.names,
            "dof": 6,
        }
    )
    while True:
        msg = read_msg()
        if msg is None:
            break
        t = msg.get("type")
        if t == "reset":
            st.pos = [0.0] * 6
            st.grip = 0.5
            write_msg(st.obs())
        elif t == "observe":
            write_msg(st.obs())
        elif t == "command":
            pos = msg.get("positions") or []
            for i, v in enumerate(pos[:6]):
                st.pos[i] = float(v)
            if msg.get("gripper_open") is not None:
                st.grip = float(msg["gripper_open"])
            write_msg(st.obs())
        elif t in ("estop", "shutdown"):
            write_msg({"type": "ack"})
            if t == "shutdown":
                break
        else:
            write_error(f"unknown type: {t}")
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
