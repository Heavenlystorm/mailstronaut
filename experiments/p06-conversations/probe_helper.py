"""Synthetic real-executable helper limits and bounded component timing only."""
import json
import os
import pathlib
import resource
import subprocess
import sys
import time

binary = pathlib.Path(sys.argv[1]).resolve(strict=True)
environment = {k: v for k, v in os.environ.items() if k not in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR")}
child = subprocess.Popen([str(binary), "--internal-project-threads"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
try:
    limits = ""
    for _ in range(200):
        limits = pathlib.Path(f"/proc/{child.pid}/limits").read_text()
        if "1073741824" in limits:
            break
        if child.poll() is not None:
            raise RuntimeError("helper exited before accepting input")
        time.sleep(0.01)
    required = {"Max cpu time": ("5", "6"), "Max address space": ("1073741824", "1073741824"), "Max core file size": ("0", "0")}
    observed = {}
    for name, expected in required.items():
        line = next(line for line in limits.splitlines() if line.startswith(name))
        values = line[len(name):].split()[:2]
        assert tuple(values) == expected, (name, values)
        observed[name] = values
    messages = [{"id": i + 1, "message_id": f"{i}@benchmark.test", "parent": f"{i - 1}@benchmark.test" if i % 2 else None, "references": [], "received_at": i} for i in range(10000)]
    payload = json.dumps(messages).encode()
    started = time.monotonic()
    stdout, stderr = child.communicate(payload, timeout=10)
    elapsed = time.monotonic() - started
    assert child.returncode == 0 and stderr == b"", (child.returncode, stderr[:100])
    groups = json.loads(stdout)["groups"]
    assert len(groups) == 5000 and all(len(group) == 2 for group in groups)
    assert sorted(mid for group in groups for mid in group) == list(range(1, 10001))
    print(json.dumps({"messages": 10000, "groups": 5000, "input_bytes": len(payload), "output_bytes": len(stdout), "elapsed_seconds": elapsed, "peak_child_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss, "observed_limits_before_input": observed}, indent=2))
finally:
    if child.poll() is None:
        child.kill()
    child.wait()
