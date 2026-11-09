#!/usr/bin/env python3
"""Guest-only private D-Bus/keyring session, with no login-keyring mutation."""
import json
import os
import subprocess
import sys

def main():
    payload = json.load(sys.stdin)
    subprocess.run(["gnome-keyring-daemon", "--unlock", "--components=secrets"],
                   input=payload["password"].encode(), stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=15, check=True)
    request = {"id": payload["id"], "secret": payload["secret"]}
    first = subprocess.run([payload["helper"]], input=json.dumps(request).encode(),
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
    if first.returncode:
        raise RuntimeError("Production credential-store roundtrip failed in disposable Secret Service")
    result = json.loads(first.stdout)
    env = os.environ.copy()
    env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=/nonexistent/dunlin-private-bus"
    second = subprocess.run([payload["helper"]], env=env,
                            input=json.dumps({**request, "expect_unavailable": True}).encode(),
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
    if second.returncode:
        raise RuntimeError("Production credential store did not fail closed on unavailable Secret Service")
    result.update(json.loads(second.stdout))
    print(json.dumps(result))


if __name__ == "__main__":
    main()
