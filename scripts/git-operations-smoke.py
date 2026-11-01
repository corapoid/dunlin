#!/usr/bin/env python3
"""Actual paired HTTP Git smoke; disposable DB/repo only, no ACP or provider calls."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--relay", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/dunlin-relay")
    parser.add_argument("--tmp-root", type=Path)
    options = parser.parse_args()
    binary = options.relay.resolve(strict=True)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("GIT_", "DUNLIN_RELAY"))}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
               GIT_TERMINAL_PROMPT="0", GIT_NO_LAZY_FETCH="1")
    with tempfile.TemporaryDirectory(prefix="dunlin-git-relay-smoke-", dir=options.tmp_root) as directory:
        temporary = Path(directory)
        root = temporary / "project"
        root.mkdir()
        database = temporary / "relay.db"

        def relay(*args):
            return subprocess.check_output([str(binary), *args, "--db", str(database)], env=env, text=True)

        def git(*args):
            return subprocess.check_output(["git", "-C", str(root), *args], env=env, text=True, stderr=subprocess.PIPE)

        def write(name, text):
            (root / name).write_text(text, encoding="utf-8")

        def commit(message):
            git("add", "--all")
            git("commit", "-qm", message)

        git("init", "--template=", "-qb", "main")
        (root / ".git/hooks").mkdir()
        for key, value in {"user.name": "HTTP Smoke", "user.email": "smoke@example.invalid",
                           "commit.gpgsign": "false", "core.hooksPath": str(root / ".git/hooks"),
                           "core.fsmonitor": "false", "core.attributesFile": os.devnull}.items():
            git("config", key, value)
        write("file", "base\n")
        commit("base")
        relay("init")
        relay("project", "add", "fixture", str(root), "--user", "admin")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect)

        def request(path, token=None, body=None, expected=200):
            headers = {"Content-Type": "application/json"}
            if token:
                headers["Authorization"] = f"Bearer {token}"
            data = None if body is None else json.dumps(body).encode()
            req = urllib.request.Request(origin + path, data=data, headers=headers)
            try:
                response = opener.open(req, timeout=30)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                assert response.code == expected, f"{path}: HTTP {response.code}, expected {expected}"
                payload = response.read(2 * 1024 * 1024 + 1)
                assert len(payload) <= 2 * 1024 * 1024
                if path == "/health":
                    return payload.decode("utf-8")
                return json.loads(payload)

        with (temporary / "server.log").open("wb") as log:
            server = subprocess.Popen([str(binary), "serve", "--db", str(database),
                                       "--listen", f"127.0.0.1:{port}"], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while True:
                    assert server.poll() is None, "Isolated relay stopped unexpectedly"
                    try:
                        request("/health")
                        break
                    except urllib.error.URLError:
                        assert time.monotonic() < deadline, "Isolated relay did not become ready"
                        time.sleep(0.05)

                def pair(user):
                    output = relay("pair", user, "--ttl", "10")
                    code = next(line.rsplit(": ", 1)[1] for line in output.splitlines()
                                if line.startswith("Pairing code for "))
                    return request("/v1/pair", body={"code": code, "device_name": "isolated Git smoke"})["token"]

                token = pair("admin")
                project = next(p["id"] for p in request("/v1/projects", token) if p["name"] == "fixture")

                def prepare(kind, **fields):
                    return request("/v1/git/operation/prepare", token,
                                   {"project": project, "action": {"kind": kind, **fields}})

                def execute(plan, success=True):
                    result = request("/v1/git/operation/execute", token, {"project": project, "id": plan["id"]})
                    assert result["success"] is success, f"Git result: {result['message']}"
                    return result

                def action(kind, success=True, **fields):
                    return execute(prepare(kind, **fields), success)

                write("file", "staged\n")
                git("add", "file")
                write("file", "working\n")
                write("untracked", "keep\n")
                plan = prepare("stash_push", message="HTTP fixture", include_untracked=True)
                assert (root / "file").read_text() == "working\n" and git("show", ":file") == "staged\n"
                result = execute(plan)
                assert (root / "file").read_text() == "base\n"
                request("/v1/git/operation/execute", token, {"project": project, "id": plan["id"]}, 409)
                action("stash_pop", oid=result["overview"]["stashes"][0]["oid"])
                assert (root / "file").read_text() == "working\n" and git("show", ":file") == "staged\n"
                assert (root / "untracked").read_text() == "keep\n"
                commit("restored fixture")

                git("checkout", "-qb", "topic")
                write("topic", "topic\n")
                commit("topic")
                git("checkout", "-q", "main")
                action("merge", revision="topic")
                git("checkout", "-qb", "upstream")
                write("upstream", "upstream\n")
                commit("upstream")
                git("checkout", "-q", "main")
                write("local", "local\n")
                commit("local")
                action("rebase", revision="upstream")
                git("checkout", "-qb", "pick-source")
                write("picked", "picked\n")
                commit("pick")
                picked = git("rev-parse", "HEAD").strip()
                git("checkout", "-q", "main")
                action("cherry_pick", revision=picked, mainline=None)
                assert (root / "picked").read_text() == "picked\n"

                result = action("worktree_add", name="network-fixture", revision="HEAD")
                tree = next(w for w in result["overview"]["worktrees"] if not w["current"])
                action("worktree_remove", path=tree["path"])
                assert not Path(tree["path"]).exists()
                action("worktree_prune")

                for suffix, verb in [("continue", "continue"), ("abort", "abort")]:
                    branch = "conflict-" + suffix
                    git("checkout", "-qb", branch)
                    write("file", "theirs " + suffix + "\n")
                    commit(branch)
                    git("checkout", "-q", "main")
                    write("file", "ours " + suffix + "\n")
                    commit("ours " + suffix)
                    previous = git("rev-parse", "HEAD")
                    result = action("merge", success=False, revision=branch)
                    assert result["overview"]["conflicts"]
                    if verb == "continue":
                        write("file", "resolved\n")
                        git("add", "file")
                    result = action(verb, operation="merge")
                    assert not result["overview"]["operations"]
                    if verb == "abort":
                        assert git("rev-parse", "HEAD") == previous

                head = git("rev-parse", "HEAD").strip()
                query = urllib.parse.urlencode({"project": project, "path": "file", "head": head})
                value = request("/v1/git/blame?" + query, token)["value"]
                assert value["head"] == head and value["contents"] == (root / "file").read_text()
                assert value["lines"][0]["author"] == "HTTP Smoke"
                relay("user", "add", "outsider")
                outsider = pair("outsider")
                request("/v1/git/blame?" + query, outsider, expected=403)
                request("/v1/git/operation/prepare", outsider,
                        {"project": project, "action": {"kind": "worktree_prune"}}, 403)
                plan = prepare("stash_push", message="stale", include_untracked=False)
                write("file", "later edit\n")
                request("/v1/git/operation/execute", token, {"project": project, "id": plan["id"]}, 409)
                assert git("stash", "list") == "" and (root / "file").read_text() == "later edit\n"
                print("PASS: real paired loopback HTTP, stash/index round-trip, merge/rebase/cherry-pick, worktree add/remove/prune, conflict continue/abort, HEAD blame, grants, replay and stale-plan rejection; no ACP/provider calls or user files")
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()
