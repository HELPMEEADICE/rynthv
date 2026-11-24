#!/usr/bin/env python3
"""Helper library and CLI runner for RynthV headless `--stdio` JSON Lines sessions."""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


class RynthVError(RuntimeError):
    """Structured error returned by `rynthv -cli`."""

    def __init__(self, command: str, error: dict[str, Any]):
        self.command = command
        self.code = error.get("code", "UNKNOWN")
        self.message = error.get("message", str(error))
        self.details = error.get("details", {})
        super().__init__(f"{self.command or 'cli'} [{self.code}]: {self.message}")


def run_once(
    *args: Any,
    binary: str | os.PathLike[str] = "rynthv",
    input_text: str | None = None,
    cwd: str | os.PathLike[str] | None = None,
    env: dict[str, str] | None = None,
) -> dict[str, Any]:
    """Execute one non-interactive `rynthv -cli --json --quiet` command."""
    cmd = [os.fspath(binary), "-cli", "--non-interactive", "--json", "--quiet", *map(str, args)]
    proc = subprocess.run(
        cmd,
        input=input_text,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        cwd=cwd,
        env=env,
        check=False,
    )
    if not proc.stdout.strip():
        raise RuntimeError(f"Empty JSON output (exit={proc.returncode}): {proc.stderr.strip()}")
    reply = json.loads(proc.stdout)
    if not reply.get("ok"):
        raise RynthVError(reply.get("command", ""), reply.get("error", {}))
    return reply["result"]


class RynthVStdio:
    """Persistent `rynthv -cli --stdio` JSON Lines session."""

    def __init__(
        self,
        binary: str | os.PathLike[str] = "rynthv",
        cwd: str | os.PathLike[str] | None = None,
        env: dict[str, str] | None = None,
    ) -> None:
        self._proc = subprocess.Popen(
            [os.fspath(binary), "-cli", "--stdio"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
            cwd=cwd,
            env=env,
        )
        self._next_id = 1
        self._closed = False
        self._exited = False

    def __enter__(self) -> RynthVStdio:
        return self

    def __exit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.close(discard=exc_type is not None)

    @property
    def running(self) -> bool:
        return not self._closed and not self._exited and self._proc.poll() is None

    def call_raw(self, *args: Any, input_data: Any = None) -> dict[str, Any]:
        """Send one JSON Lines request and return the raw reply envelope."""
        if not self.running:
            raise RuntimeError("RynthV stdio session is not running")
        assert self._proc.stdin is not None and self._proc.stdout is not None
        req_id = self._next_id
        self._next_id += 1
        str_args = [str(a) for a in args]
        payload: dict[str, Any] = {"id": req_id, "args": str_args}
        if input_data is not None:
            payload["input"] = input_data
        self._proc.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
        self._proc.stdin.flush()
        line = self._proc.stdout.readline()
        if not line:
            stderr = self._proc.stderr.read() if self._proc.stderr else ""
            raise RuntimeError(f"RynthV stdio closed unexpectedly: {stderr.strip()}")
        reply = json.loads(line)
        if reply.get("id") != req_id:
            raise RuntimeError(f"Mismatched response id: expected {req_id}, got {reply.get('id')}")
        if str_args == ["exit"] and reply.get("ok"):
            self._exited = True
        return reply

    def call(self, *args: Any, input_data: Any = None) -> dict[str, Any]:
        """Send one request and return `result`, raising `RynthVError` on failure."""
        reply = self.call_raw(*args, input_data=input_data)
        if not reply.get("ok"):
            raise RynthVError(reply.get("command", ""), reply.get("error", {}))
        return reply["result"]

    def close(self, discard: bool = False) -> None:
        """Cleanly exit the stdio session, optionally discarding unsaved changes."""
        if self._closed:
            return
        try:
            if self._exited:
                self._proc.wait(timeout=5)
            elif self._proc.poll() is None:
                if discard:
                    try:
                        status = self.call("project", "status")
                        if status.get("open") and status.get("dirty"):
                            self.call("project", "close", "--discard")
                    except Exception:
                        pass
                try:
                    if discard:
                        self.call_raw("exit")
                    else:
                        self.call("exit")
                finally:
                    if self._proc.stdin and not self._proc.stdin.closed:
                        self._proc.stdin.close()
                self._proc.wait(timeout=5)
        finally:
            self._closed = True
            for stream in (self._proc.stdin, self._proc.stdout, self._proc.stderr):
                if stream and not stream.closed:
                    stream.close()
            if self._proc.poll() is None:
                self._proc.terminate()
                self._proc.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", default="rynthv", help="Path to rynthv executable")
    parser.add_argument(
        "--requests",
        help="JSON file (or '-' for stdin) containing [{\"args\": [...], \"input\": ...}, ...]",
    )
    parser.add_argument("--discard", action="store_true", help="Discard unsaved changes on exit")
    args = parser.parse_args()

    if not args.requests:
        print(json.dumps(run_once("doctor", binary=args.bin), ensure_ascii=False, indent=2))
        return 0

    raw = sys.stdin.read() if args.requests == "-" else Path(args.requests).read_text(encoding="utf-8")
    jobs = json.loads(raw)
    if not isinstance(jobs, list):
        raise ValueError("Requests payload must be a JSON array")

    results = []
    with RynthVStdio(binary=args.bin) as session:
        for job in jobs:
            results.append(session.call(*job["args"], input_data=job.get("input")))
        if not args.discard and session.running:
            status = session.call("project", "status")
            if status.get("open") and status.get("dirty"):
                session.call("project", "save")
        session.close(discard=args.discard)
    print(json.dumps(results, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
