"""Real old-artifact -> candidate upgrade on a separate disposable database."""
import copy
import hashlib
import json
from pathlib import Path
import re
import tarfile
import secrets

from .core import Blocked, require, sha256, wait_until
from .deployment import start_service, restart_service
from .providers import quoted


def sqlite_backup_script(source, destination):
    """Include committed WAL frames, even after an abrupt service stop."""
    from urllib.parse import quote
    uri = "file:" + quote(str(source), safe="/") + "?mode=ro"
    program = (f"import sqlite3; source=sqlite3.connect({uri!r},uri=True,timeout=10); "
               f"destination=sqlite3.connect({str(destination)!r},timeout=10); "
               "source.backup(destination); "
               "assert destination.execute('PRAGMA quick_check').fetchone()[0]=='ok'; "
               "destination.close(); source.close()")
    return "python3 -c " + quoted(program)


def run(scenarios):
    p, c, run = scenarios.p, scenarios.c, scenarios.run
    artifact = p.options.previous_relay_artifact
    expected_hash = p.options.previous_relay_sha256
    if not artifact or not expected_hash:
        raise Blocked("Upgrade requires --previous-relay-artifact and --previous-relay-sha256")
    artifact = Path(artifact).resolve(strict=True)
    require(sha256(artifact) == expected_hash, "Previous relay artifact checksum mismatch")
    if tarfile.is_tarfile(artifact):
        with tarfile.open(artifact) as archive:
            entries = archive.getmembers()
            require(len(entries) == 1 and entries[0].name == "dunlin-relay" and entries[0].isfile(),
                    "Invalid previous relay archive")
            previous = archive.extractfile(entries[0]).read()
    else:
        previous = artifact.read_bytes()
    candidate = p.ssh.exec("cat " + quoted(p.binary)).stdout
    require(hashlib.sha256(candidate).digest() != hashlib.sha256(previous).digest(),
            "An upgrade test needs two distinct relay artifacts")
    root = p.root + "/upgrade-fixture-" + secrets.token_hex(4)
    p.ssh.exec(f"umask 077; mkdir {quoted(root)} {quoted(root + '/project')}; "
               f"printf 'before-upgrade\\n' > {quoted(root + '/project/file.txt')}")
    binary, db = root + "/relay", root + "/relay.db"
    p.ssh.exec(f"cat > {quoted(binary)}; chmod 700 {quoted(binary)}", input=previous)
    def relay(*args):
        return p.ssh.text(" ".join(quoted(a) for a in [binary, *args, "--db", db]))
    # This is a fresh OLD schema, never a downgrade of the candidate's database.
    relay("init")
    relay("project", "add", "upgrade", root + "/project", "--user", "admin")
    old = copy.copy(p)
    old.root = root
    old.service_name = run.id + "-upgrade"
    launch = (f"#!/bin/sh\nexec env -i HOME={quoted(root)} PATH=/usr/bin:/bin:/usr/sbin:/sbin "
              f"{quoted(binary)} serve --db {quoted(db)} --listen 127.0.0.1:8878 "
              f">>{quoted(root + '/relay.log')} 2>&1\n")
    p.ssh.exec(f"cat > {quoted(root + '/serve.sh')}; chmod 700 {quoted(root + '/serve.sh')}", input=launch.encode())
    start_service(old)
    original = (c.url, c.token, c.project_id, c.cwd)
    try:
        c.tunnel(p, port=8878)
        wait_until(lambda: c.request("/health") == "ok", timeout=30)
        code = re.search(r"Pairing code for .*?: ([A-Z0-9]+)", relay("pair", "admin")).group(1)
        c.token = c.pair(code)["token"]
        project = c.call("projects")[0]
        c.project_id, c.cwd = project["id"], project["path"]
        require(c.call("read", path="file.txt")["contents"] == "before-upgrade\n", "Previous relay is not usable")
        # Quiesce before copying the DB. A SQLite/WAL file copied while running
        # is not a valid backup contract.
        if p.platform == "Linux":
            p.ssh.exec(f"systemctl --user stop {quoted(old.service_name)}.service")
        else:
            p.ssh.exec(f"launchctl bootout gui/$(id -u)/{quoted(old.service_name)}")
        p.ssh.exec(f"set -eu; {sqlite_backup_script(db, db + '.pre-upgrade')}; cat > {quoted(binary + '.new')}; "
                   f"chmod 700 {quoted(binary + '.new')}; mv {quoted(binary + '.new')} {quoted(binary)}", input=candidate)
        if p.platform == "Darwin":
            p.ssh.exec(f"launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/{old.service_name}.plist")
        else:
            restart_service(old)
        wait_until(lambda: c.request("/health") == "ok", timeout=30)
        # Same token and same project ID must work, with no re-pairing.
        c.request("/v1/me")
        require(c.call("projects")[0]["id"] == project["id"], "Project identity changed")
        require(c.call("read", path="file.txt")["contents"] == "before-upgrade\n", "Upgrade lost file contents")
        c.call("write", path="file.txt", contents="after-upgrade\n")
        require(p.ssh.text("cat " + quoted(root + "/project/file.txt")) == "after-upgrade", "Candidate cannot save after upgrade")
        run.report["facts"]["upgrade"] = {"previous_archive_sha256": expected_hash,
            "candidate_binary_sha256": hashlib.sha256(candidate).hexdigest(),
            "preserved": ["paired device token", "project identity", "project contents"],
            "not_claimed": ["database downgrade", "old live ACP process continuation"]}
    finally:
        c.url, c.token, c.project_id, c.cwd = original
