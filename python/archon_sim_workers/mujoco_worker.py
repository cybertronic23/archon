#!/usr/bin/env python3
"""MuJoCo NDJSON worker for Archon BridgedSimBackend (protocol v1)."""

from __future__ import annotations

import os
import sys
import time
import traceback
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from protocol import PROTOCOL_VERSION, now_us, read_msg, write_error, write_msg, write_ppm  # noqa: E402


def _default_model_path() -> Path:
    return _HERE.parent / "models" / "desktop_arm_red_blob.xml"


def discover_actuated_joints(model) -> Tuple[List[str], List[int]]:
    """Return (joint_names, actuator_ids) for position/motor actuators with joints."""
    import mujoco

    names: List[str] = []
    acts: List[int] = []
    for i in range(model.nu):
        # trnid[i, 0] is joint id for joint actuators
        jid = int(model.actuator_trnid[i][0])
        if jid < 0:
            continue
        name = mujoco.mj_id2name(model, mujoco.mjtObj.mjOBJ_JOINT, jid)
        if not name:
            name = f"joint_{len(names) + 1}"
        names.append(name)
        acts.append(i)
    if not names:
        # Fall back to all hinge joints
        for j in range(model.njnt):
            if model.jnt_type[j] == mujoco.mjtJoint.mjJNT_HINGE:
                name = mujoco.mj_id2name(model, mujoco.mjtObj.mjOBJ_JOINT, j) or f"joint_{j}"
                names.append(name)
    return names, acts


class MujocoSession:
    def __init__(self) -> None:
        self.model = None
        self.data = None
        self.renderer = None
        self.joint_names: List[str] = []
        self.actuator_ids: List[int] = []
        self.dof = 6
        self.render = True
        self.media_root: Optional[Path] = None
        self.frame_seq = 0
        self.gripper_open = 0.5
        self.camera_name = "scene"
        self.width = 64
        self.height = 48
        self.model_path: Optional[Path] = None
        self.viewer = None
        self.want_viewer = False
        self.record_dir: Optional[Path] = None
        self.video_out: Optional[Path] = None
        self.video_frame_seq = 0
        self.record_every_n = 2  # subsample for smoother file size

    def load(self, hello: Dict[str, Any]) -> None:
        try:
            import mujoco
        except ImportError as e:
            raise RuntimeError(
                "mujoco Python package not installed. "
                "pip install -r python/requirements-mujoco.txt"
            ) from e

        self.render = bool(hello.get("render", True))
        self.want_viewer = bool(hello.get("viewer", False))
        media = hello.get("media_root")
        self.media_root = Path(media) if media else None
        if hello.get("record_dir"):
            self.record_dir = Path(hello["record_dir"])
            self.record_dir.mkdir(parents=True, exist_ok=True)
        if hello.get("video_out"):
            self.video_out = Path(hello["video_out"])
            if self.record_dir is None:
                self.record_dir = self.video_out.parent / f".frames_{self.video_out.stem}"
                self.record_dir.mkdir(parents=True, exist_ok=True)
        if hello.get("camera"):
            self.camera_name = str(hello["camera"])
        # High-res only when recording MP4; episode thumbnails stay cheap
        if self.record_dir is not None:
            self.width = int(hello.get("record_width") or 1280)
            self.height = int(hello.get("record_height") or 720)

        model_path = hello.get("model_path") or os.environ.get("ARCHON_MUJOCO_MODEL")
        if model_path:
            self.model_path = Path(model_path)
        else:
            self.model_path = _default_model_path()
        if not self.model_path.exists():
            raise RuntimeError(f"MJCF not found: {self.model_path}")

        # Load relative mesh assets from model directory
        self.model = mujoco.MjModel.from_xml_path(str(self.model_path))
        self.data = mujoco.MjData(self.model)
        mujoco.mj_resetData(self.model, self.data)
        mujoco.mj_forward(self.model, self.data)

        auto = bool(hello.get("auto_joints", True))
        requested = list(hello.get("joint_names") or [])
        if auto or not requested:
            self.joint_names, self.actuator_ids = discover_actuated_joints(self.model)
        else:
            self.joint_names = requested
            self.actuator_ids = list(range(min(len(requested), self.model.nu)))
        self.dof = len(self.joint_names)
        if self.dof == 0:
            raise RuntimeError("no actuated joints found in model")

        is_car = any(n.startswith("root_") for n in self.joint_names)

        if self.want_viewer:
            try:
                import mujoco.viewer

                self.viewer = mujoco.viewer.launch_passive(self.model, self.data)
                if is_car:
                    self.viewer.cam.lookat[:] = [0.4, 0.0, 0.1]
                    self.viewer.cam.distance = 2.4
                    self.viewer.cam.azimuth = 140
                    self.viewer.cam.elevation = -25
                else:
                    self.viewer.cam.lookat[:] = [0.12, 0.0, 0.28]
                    self.viewer.cam.distance = 1.15
                    self.viewer.cam.azimuth = 140
                    self.viewer.cam.elevation = -20
                sys.stderr.write(
                    "[mujoco_worker] interactive viewer opened "
                    "(window stays open until you close it after motion)\n"
                )
            except Exception as e:
                sys.stderr.write(f"[mujoco_worker] viewer failed to open: {e}\n")
                self.viewer = None
                self.want_viewer = False

        # Always try offscreen renderer when recording video
        need_renderer = self.render or self.record_dir is not None
        if need_renderer:
            try:
                self.renderer = mujoco.Renderer(self.model, height=self.height, width=self.width)
                self.render = True
            except Exception as e:
                sys.stderr.write(f"[mujoco_worker] offscreen render disabled: {e}\n")
                self.renderer = None
                if self.record_dir is not None:
                    sys.stderr.write(
                        "[mujoco_worker] WARNING: cannot record MP4 without offscreen renderer. "
                        "Use macOS screen capture (Cmd+Shift+5) on the viewer window instead.\n"
                    )

        # Pick a usable camera if configured one is missing
        if self.renderer is not None:
            cid = mujoco.mj_name2id(self.model, mujoco.mjtObj.mjOBJ_CAMERA, self.camera_name)
            if cid < 0 and self.model.ncam > 0:
                alt = mujoco.mj_id2name(self.model, mujoco.mjtObj.mjOBJ_CAMERA, 0)
                if alt:
                    self.camera_name = alt

    def _sync_viewer(self) -> None:
        if self.viewer is None:
            return
        try:
            if not self.viewer.is_running():
                self.viewer = None
                return
            self.viewer.sync()
        except Exception as e:
            sys.stderr.write(f"[mujoco_worker] viewer sync error: {e}\n")
            self.viewer = None

    def close_viewer(self) -> None:
        if self.viewer is None:
            return
        try:
            self.viewer.close()
        except Exception:
            pass
        self.viewer = None

    def hold_viewer_until_closed(self) -> None:
        """Keep window open after the run so the user can inspect the scene."""
        if self.viewer is None:
            return
        sys.stderr.write(
            "[mujoco_worker] motion finished — close the MuJoCo window to exit\n"
        )
        try:
            while self.viewer is not None and self.viewer.is_running():
                self.viewer.sync()
                time.sleep(0.03)
        except Exception:
            pass
        self.viewer = None

    def _joint_positions(self) -> List[float]:
        import mujoco

        assert self.model is not None and self.data is not None
        q = []
        for name in self.joint_names:
            jid = mujoco.mj_name2id(self.model, mujoco.mjtObj.mjOBJ_JOINT, name)
            if jid < 0:
                q.append(0.0)
                continue
            qadr = self.model.jnt_qposadr[jid]
            q.append(float(self.data.qpos[qadr]))
        return q

    def _maybe_record_frame(self) -> None:
        if self.record_dir is None or self.renderer is None:
            return
        assert self.model is not None and self.data is not None
        try:
            self.renderer.update_scene(self.data, camera=self.camera_name)
            pixels = self.renderer.render()
        except Exception as e:
            sys.stderr.write(f"[mujoco_worker] record frame skipped: {e}\n")
            return
        self.video_frame_seq += 1
        if self.video_frame_seq % self.record_every_n != 0:
            return
        h, w, _ = pixels.shape
        idx = self.video_frame_seq // self.record_every_n
        path = self.record_dir / f"frame_{idx:06d}.ppm"
        write_ppm(str(path), w, h, pixels.tobytes())

    def encode_video(self) -> None:
        if self.video_out is None or self.record_dir is None:
            return
        frames = sorted(self.record_dir.glob("frame_*.ppm"))
        if not frames:
            sys.stderr.write("[mujoco_worker] no frames to encode\n")
            return
        self.video_out.parent.mkdir(parents=True, exist_ok=True)
        pattern = str(self.record_dir / "frame_%06d.ppm")
        cmd = [
            "ffmpeg",
            "-y",
            "-framerate",
            "30",
            "-i",
            pattern,
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            str(self.video_out),
        ]
        import subprocess

        try:
            subprocess.run(cmd, check=True, capture_output=True)
            sys.stderr.write(f"[mujoco_worker] video saved: {self.video_out}\n")
        except FileNotFoundError:
            sys.stderr.write(
                "[mujoco_worker] ffmpeg not found. Install with: brew install ffmpeg\n"
                f"Frames are in: {self.record_dir}\n"
            )
        except subprocess.CalledProcessError as e:
            sys.stderr.write(
                f"[mujoco_worker] ffmpeg failed: {e.stderr.decode('utf-8', errors='ignore')}\n"
                f"Frames are in: {self.record_dir}\n"
            )

    def _maybe_render(self) -> Optional[Dict[str, Any]]:
        if not self.render or self.renderer is None or self.media_root is None:
            return None
        assert self.model is not None and self.data is not None
        try:
            self.renderer.update_scene(self.data, camera=self.camera_name)
            pixels = self.renderer.render()
        except Exception as e:
            sys.stderr.write(f"[mujoco_worker] render frame skipped: {e}\n")
            return None
        h, w, _ = pixels.shape
        self.frame_seq += 1
        rel = f"media/images.primary/{self.frame_seq:06d}.ppm"
        path = self.media_root / rel
        write_ppm(str(path), w, h, pixels.tobytes())
        return {
            "key": "images.primary",
            "stamp_us": now_us(),
            "frame_id": self.camera_name,
            "encoding": "rgb8",
            "width": w,
            "height": h,
            "uri": str(path),
        }

    def observation(self) -> Dict[str, Any]:
        modalities = []
        mod = self._maybe_render()
        if mod is not None:
            modalities.append(mod)
        return {
            "type": "observation",
            "stamp_us": now_us(),
            "proprio": {
                "joint_names": self.joint_names,
                "positions": self._joint_positions(),
                "gripper_open": self.gripper_open,
            },
            "modalities": modalities,
        }

    def reset(self) -> Dict[str, Any]:
        import mujoco

        assert self.model is not None and self.data is not None
        mujoco.mj_resetData(self.model, self.data)
        mujoco.mj_forward(self.model, self.data)
        self.gripper_open = 0.5
        self._sync_viewer()
        self._maybe_record_frame()
        return self.observation()

    def command(self, msg: Dict[str, Any]) -> Dict[str, Any]:
        import mujoco

        assert self.model is not None and self.data is not None
        positions = list(msg.get("positions") or [])
        names = list(msg.get("names") or [])

        # Map by name when possible
        name_to_pos = {}
        if names and len(names) == len(positions):
            name_to_pos = {n: float(p) for n, p in zip(names, positions)}

        for i, jname in enumerate(self.joint_names):
            if jname in name_to_pos:
                val = name_to_pos[jname]
            elif i < len(positions):
                val = float(positions[i])
            else:
                continue
            if i < len(self.actuator_ids):
                aid = self.actuator_ids[i]
                self.data.ctrl[aid] = val
            elif i < self.model.nu:
                self.data.ctrl[i] = val

        if msg.get("gripper_open") is not None:
            self.gripper_open = float(msg["gripper_open"])

        # With a viewer: step near real-time so motion is visible.
        # Headless / video-only: burst steps and capture frames.
        n_sub = 15 if self.viewer is not None else 10
        dt = float(self.model.opt.timestep)
        for _ in range(n_sub):
            mujoco.mj_step(self.model, self.data)
            if self.viewer is not None:
                self._sync_viewer()
                time.sleep(dt)
            self._maybe_record_frame()
        if self.viewer is None:
            self._sync_viewer()
        return self.observation()


def main() -> int:
    session = MujocoSession()
    try:
        hello = read_msg()
        if hello is None or hello.get("type") != "hello":
            write_error("expected hello")
            return 1
        if hello.get("platform") not in (None, "mujoco"):
            write_error(f"this worker is mujoco, got platform={hello.get('platform')}")
            return 1
        session.load(hello)
        write_msg(
            {
                "type": "hello_ok",
                "protocol_version": PROTOCOL_VERSION,
                "platform": "mujoco",
                "joint_names": session.joint_names,
                "dof": session.dof,
                "model_path": str(session.model_path) if session.model_path else None,
            }
        )

        while True:
            msg = read_msg()
            if msg is None:
                break
            t = msg.get("type")
            if t == "reset":
                write_msg(session.reset())
            elif t == "observe":
                write_msg(session.observation())
            elif t == "command":
                write_msg(session.command(msg))
            elif t == "estop":
                write_msg({"type": "ack"})
            elif t == "shutdown":
                session.encode_video()
                if session.viewer is not None:
                    session.hold_viewer_until_closed()
                else:
                    session.close_viewer()
                write_msg({"type": "ack"})
                break
            else:
                write_error(f"unknown type: {t}")
                return 1
    except Exception as e:
        write_error(f"{e}\n{traceback.format_exc()}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
