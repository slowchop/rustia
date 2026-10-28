#!/usr/bin/env python3
"""CI-only, unprivileged observation of one pinned benchmark invocation.

Includes startup and warmup. Never use these observations as acceptance data.
Exports no process IDs, file paths, device labels or host identifiers.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import re
import subprocess
import sys
import time


def counter_column(text, cpu):
    lines = text.splitlines()
    column = lines[0].split().index(f"CPU{cpu}")
    total = 0
    for line in lines[1:]:
        fields = line.split(":", 1)[-1].split()
        if len(fields) > column and fields[column].isdigit():
            total += int(fields[column])
    return total


def frequency_states(text):
    return {frequency: int(ticks) for frequency, ticks in
            (line.split() for line in text.splitlines())}


def layout(text, binary):
    result = {}
    for line in text.splitlines():
        fields = line.split()
        name = fields[-1]
        kind = "code" if name == binary and fields[1] == "r-xp" else (
            "heap" if name == "[heap]" else "stack" if name == "[stack]" else None)
        if kind:
            start, end = fields[0].split("-")
            result[kind] = {"start": start, "bytes": int(end, 16) - int(start, 16)}
    return result


def optional(read):
    try:
        return {"available": True, "value": read()}
    except (OSError, ValueError, IndexError) as error:
        return {"available": False, "reason": type(error).__name__}


def snapshot(cpu):
    frequency = Path(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq")
    return {
        "interrupts": optional(lambda: counter_column(Path("/proc/interrupts").read_text(), cpu)),
        "softirqs": optional(lambda: counter_column(Path("/proc/softirqs").read_text(), cpu)),
        "policy_residency_ticks": optional(lambda: frequency_states(
            (frequency / "stats/time_in_state").read_text())),
        "policy_transitions": optional(lambda: int((frequency / "stats/total_trans").read_text())),
        "policy_current_khz": optional(lambda: int((frequency / "scaling_cur_freq").read_text())),
    }


def resource_delta(before, after):
    fields = ["ru_utime", "ru_stime", "ru_minflt", "ru_majflt", "ru_nvcsw", "ru_nivcsw"]
    values = {field.removeprefix("ru_"): getattr(after, field) - getattr(before, field)
              for field in fields}
    if any(value < 0 for value in values.values()):
        raise ValueError("negative child resource delta")
    return values


def observe(args):
    binary = str(Path(args.binary).resolve(strict=True))
    stat = Path(binary).stat()
    file_token = hashlib.sha256(f"{stat.st_dev}:{stat.st_ino}".encode()).hexdigest()
    before = snapshot(args.cpu)
    usage_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    child = subprocess.Popen(args.command)
    mapping = {"available": False, "reason": "process-exited-before-observation"}
    try:
        # A fixed, bounded read after exec. No signals or child suspension.
        # Only this diagnostic is instrumented; ordinary measurements are unchanged.
        deadline = started + 0.25
        while child.poll() is None and time.monotonic() < deadline:
            if os.path.realpath(f"/proc/{child.pid}/exe") == binary:
                time.sleep(0.05)
                def read_mapping():
                    status = Path(f"/proc/{child.pid}/status").read_text()
                    affinity = next(line.split(":", 1)[1].strip() for line in status.splitlines()
                                    if line.startswith("Cpus_allowed_list:"))
                    if affinity != str(args.cpu):
                        raise ValueError("child is not pinned to the declared CPU")
                    return layout(Path(f"/proc/{child.pid}/maps").read_text(), binary)
                mapping = optional(read_mapping)
                break
            time.sleep(0.001)
        code = child.wait()
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait()
    elapsed = time.monotonic() - started
    usage = resource_delta(usage_before, resource.getrusage(resource.RUSAGE_CHILDREN))
    result = {
        "schema": "rustia.bench.process-observation.v1",
        "binary_sha256": args.binary_sha256, "binary_file_token": file_token,
        "binary_identity": "Enclosing runner verifies bytes before quiet and after the workload leg; this helper does not prefault the executable.",
        "exit_code": code, "wall_seconds": elapsed,
        "resources": usage, "layout": mapping, "before": before, "after": snapshot(args.cpu),
        "clock_ticks_per_second": os.sysconf("SC_CLK_TCK"),
        "aslr_policy": optional(lambda: int(Path("/proc/sys/kernel/randomize_va_space").read_text())),
        "limits": "Includes startup/warmup. Policy residency is not effective CPU frequency. Diagnostic only.",
    }
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
    return code if code >= 0 else 128 - code


def self_test():
    from types import SimpleNamespace
    from tempfile import TemporaryDirectory
    from unittest.mock import patch
    assert counter_column(" CPU0 CPU4\n X: 1 2 driver\n NMI: 3 4\n ERR: 0\n", 4) == 6
    try:
        counter_column(" CPU0\n X: 1\n", 4)
        raise AssertionError("missing CPU accepted")
    except ValueError:
        pass
    assert frequency_states("3800000 25\n2200000 2\n") == {"3800000": 25, "2200000": 2}
    maps = "1000-2000 r-xp 0 0 0 /private/binary\n3000-5000 rw-p 0 0 0 [heap]\n"
    assert layout(maps, "/private/binary") == {
        "code": {"start": "1000", "bytes": 4096}, "heap": {"start": "3000", "bytes": 8192}}
    assert "/private" not in json.dumps(layout(maps, "/private/binary"))
    assert optional(lambda: int("missing")) == {"available": False, "reason": "ValueError"}
    fields = ["ru_utime", "ru_stime", "ru_minflt", "ru_majflt", "ru_nvcsw", "ru_nivcsw"]
    baseline = SimpleNamespace(**dict.fromkeys(fields, 1))
    assert resource_delta(baseline, SimpleNamespace(**dict.fromkeys(fields, 3)))["minflt"] == 2
    try:
        resource_delta(baseline, SimpleNamespace(**dict.fromkeys(fields, 0)))
        raise AssertionError("negative resource delta accepted")
    except ValueError:
        pass
    # Exercise lifecycle/resource export with a fake child, never a local benchmark.
    class Child:
        pid = 42
        done = False

        def poll(self):
            return 7 if self.done else None

        def wait(self):
            self.done = True
            return 7

        def terminate(self):
            raise AssertionError("completed child must not be terminated")

    with TemporaryDirectory() as temporary:
        fake_child = Child()
        binary = Path(temporary) / "fixture-binary"
        binary.write_bytes(b"nonexecutable fixture")
        resolved_binary = str(binary.resolve(strict=True))
        output = Path(temporary) / "observation.json"
        options = SimpleNamespace(binary=str(binary), binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                                  output=str(output), cpu=4, command=["fixture"])

        def read_fixture(file, *unused, **kwargs):
            if file.name == "status":
                return "Cpus_allowed_list:\t4\n"
            if file.name == "maps":
                return f"1000-2000 r-xp 0 0 0 {resolved_binary}\n"
            return "2"

        with patch(__name__ + ".snapshot", return_value={}), \
                patch(__name__ + ".subprocess.Popen", return_value=fake_child) as launch, \
                patch(__name__ + ".resource.getrusage", side_effect=[baseline, SimpleNamespace(**dict.fromkeys(fields, 3))]), \
                patch(__name__ + ".os.path.realpath", return_value=resolved_binary), \
                patch(__name__ + ".time.monotonic", side_effect=lambda: 101 if fake_child.done else 100), \
                patch(__name__ + ".time.sleep"), patch.object(Path, "read_text", read_fixture):
            assert observe(options) == 7
            launch.assert_called_once_with(["fixture"])
        record = json.loads(output.read_text())
        assert record["exit_code"] == 7 and record["resources"]["minflt"] == 2
        assert record["wall_seconds"] == 1 and record["layout"]["available"]
        assert record["binary_sha256"] == hashlib.sha256(binary.read_bytes()).hexdigest()
        assert len(record["binary_file_token"]) == 64
        assert temporary not in output.read_text()
    print("process observation offline parser/resource tests passed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--cpu", required=True, type=int)
        parser.add_argument("--binary", required=True)
        parser.add_argument("--binary-sha256", required=True)
        parser.add_argument("--output", required=True)
        parser.add_argument("command", nargs=argparse.REMAINDER)
        args = parser.parse_args()
        if args.command[:1] == ["--"]:
            args.command = args.command[1:]
        if (os.environ.get("GITHUB_ACTIONS") != "true" or not args.command or args.cpu < 0
                or not re.fullmatch(r"[0-9a-f]{64}", args.binary_sha256)):
            parser.error("process observation requires a CI command and a valid pinned CPU")
        sys.exit(observe(args))
