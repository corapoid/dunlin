#!/usr/bin/env python3
"""Strict PTY driver for the *real* installer; also copied into disposable guests."""
import errno
import os
import pty
import select
import signal
import subprocess
import sys
import time
import re


def install(script):
    prompts = [("Install or update the Dunlin server?", "y"),
               ("Server name", "remote-e2e-installer"),
               ("Install/update Tailscale", "n"),
               ("Start the relay automatically", "y"),
               ("Install GitHub and GitLab", "n"),
               ("Harnesses:", "1"), ("Proceed with these changes?", "y")]
    master, slave = pty.openpty()
    # setsid + acquiring the controlling terminal is essential for /dev/tty.
    def terminal():
        import fcntl
        import termios
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
    process = subprocess.Popen(["bash", script], stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
    os.close(slave)
    index, text = 0, ""
    deadline = time.monotonic() + 1100
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.2)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not data:
                    break
                text = (text + data.decode(errors="replace"))[-65536:]
                if index < len(prompts) and prompts[index][0] in text:
                    os.write(master, (prompts[index][1] + "\n").encode())
                    index += 1
                    text = ""
            if process.poll() is not None:
                break
        if time.monotonic() >= deadline and process.poll() is None:
            raise TimeoutError("Installer deadline or unrecognized prompt")
        process.wait(timeout=10)
        if process.returncode or index != len(prompts):
            # Do not leak codes/output (or any future secret prompt) on failure.
            safe = re.sub(r"(?im)^.*(?:pairing code|login key|password|authorization:).*$", "<redacted>", text)
            raise RuntimeError(f"Installer failed: exit={process.returncode}, dialogue_step={index}: {safe[-8000:]}")
        print("PASS: real installer completed without account authentication")
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=10)
        os.close(master)


if __name__ == "__main__":
    install(sys.argv[1])
