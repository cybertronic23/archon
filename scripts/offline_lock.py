#!/usr/bin/env python3
"""Generate lockfile offline by stubbing missing vendor crates."""
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor"
LOCK_BAK = ROOT / "Cargo.lock.full.bak"


def checksum(dest: Path) -> None:
    files = {}
    for dirpath, _, filenames in os.walk(dest):
        for f in filenames:
            if f == ".cargo-checksum.json":
                continue
            fp = Path(dirpath) / f
            rel = str(fp.relative_to(dest)).replace("\\", "/")
            files[rel] = hashlib.sha256(fp.read_bytes()).hexdigest()
    (dest / ".cargo-checksum.json").write_text(json.dumps({"files": files, "package": None}))


def version_from_bak(name: str) -> str | None:
    if not LOCK_BAK.exists():
        return None
    text = LOCK_BAK.read_text()
    m = re.search(rf'name = "{re.escape(name)}"\nversion = "([^"]+)"', text)
    return m.group(1) if m else None


def stub(name: str, ver: str) -> None:
    dest = VENDOR / f"{name}-{ver}"
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "src").mkdir(exist_ok=True)
    (dest / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "{ver}"\nedition = "2021"\n'
    )
    (dest / "src" / "lib.rs").write_text("// offline stub\n")
    checksum(dest)
    print(f"stubbed {dest.name}")


def main() -> None:
    os.chdir(ROOT)
    for i in range(40):
        proc = subprocess.run(
            ["cargo", "generate-lockfile", "--offline"],
            capture_output=True,
            text=True,
        )
        out = proc.stdout + proc.stderr
        if proc.returncode == 0:
            print("lockfile OK")
            print(out[-500:])
            return
        m = re.search(r"no matching package named `([^`]+)`", out)
        if not m:
            m = re.search(r"searched package name: `([^`]+)`", out)
        if not m:
            m = re.search(r"failed to select a version for the requirement `([^=` ]+)", out)
            if m:
                name = m.group(1)
                # locked to X?
                m2 = re.search(r"locked to ([0-9][^ )\n]+)", out)
                ver = m2.group(1) if m2 else (version_from_bak(name) or "1.0.0")
                print(out[-800:])
                stub(name, ver)
                continue
            print(out[-1200:])
            raise SystemExit(f"unhandled error at iter {i}")
        name = m.group(1)
        ver = version_from_bak(name) or "0.1.0"
        # also try candidate versions line
        m3 = re.search(r"candidate versions found which didn't match: ([^\n]+)", out)
        if m3 and "didn't match" in out:
            # need specific locked version
            m2 = re.search(r"locked to ([0-9][^ )\n]+)", out)
            if m2:
                ver = m2.group(1)
        print(f"iter {i}: missing {name}@{ver}")
        print("\n".join(out.strip().splitlines()[-8:]))
        stub(name, ver)
    raise SystemExit("too many iterations")


if __name__ == "__main__":
    main()
