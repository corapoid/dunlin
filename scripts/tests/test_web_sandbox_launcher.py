"""Launcher safety regression tests. No Docker daemon or tested app is executed."""
import argparse
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import types
import unittest
from unittest.mock import patch, MagicMock

SPEC = importlib.util.spec_from_file_location("dunlin_sandbox_launcher_under_test", Path(__file__).parents[1] / "test_web_sandbox.py")
launcher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(launcher)


class FilterTests(unittest.TestCase):
    def test_rust_filter_is_a_bounded_identifier_not_a_shell_command_or_cargo_option(self):
        self.assertEqual(launcher.rust_test_arg(types.SimpleNamespace(suite="server", rust_test="api::test_name")), " api::test_name")
        for value in ["--ignored", "x; touch /tmp/payload", "x$(payload)", "x`payload`", "x\nnext", "x" * 257]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                launcher.rust_test_arg(types.SimpleNamespace(suite="server", rust_test=value))
        with self.assertRaises(ValueError):
            launcher.rust_test_arg(types.SimpleNamespace(suite="browser", rust_test="test_name"))


class FileSizeTests(unittest.TestCase):
    def test_browser_and_server_share_the_bounded_large_test_binary_budget(self):
        for suite in ["browser","server","runtime"]:
            self.assertEqual(launcher.memory_limit(suite,True),"3584m")
        for suite in ["macos-profile"]:
            self.assertEqual(launcher.memory_limit(suite,True),"3328m")
        self.assertEqual(launcher.memory_limit("clippy",True),"2304m")
        self.assertEqual(launcher.memory_limit("mcp",True),"1536m")
        self.assertEqual(launcher.memory_limit("frontend",False),"512m")
    def test_large_rust_test_artifacts_and_sparse_cases_have_a_bounded_distinct_limit(self):
        for suite in ["runtime","workspace","browser","server","macos-profile"]:
            self.assertEqual(launcher.file_size_limit(suite),2147483648)
        for suite in ["clippy","domain","sessions","usage","mcp","frontend","typecheck","build","probe","docs","tooling"]:
            self.assertEqual(launcher.file_size_limit(suite),134217728)


class WorkspaceTests(unittest.TestCase):
    def test_workspace_accepts_only_the_same_bounded_rust_identifier(self):
        self.assertEqual(launcher.rust_test_arg(types.SimpleNamespace(suite="workspace",rust_test="comparison::tests::case")), " comparison::tests::case")
        self.assertEqual(launcher.rust_test_arg(types.SimpleNamespace(suite="mcp",rust_test="metadata::tests")), " metadata::tests")
        with self.assertRaises(ValueError):
            launcher.rust_test_arg(types.SimpleNamespace(suite="workspace",rust_test="--ignored; payload"))


    def test_workspace_mcp_and_server_packages_keep_default_containment(self):
        for suite, package, file_limit in [("workspace","dunlin-workspace",2147483648),("mcp","dunlin-mcp-client -p dunlin-http -p dunlin-vault",134217728),("server","dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage",2147483648)]:
            with tempfile.TemporaryDirectory() as directory:
                root=Path(directory)
                source=root/"source"; source.mkdir()
                registry=root/"registry"; registry.mkdir()
                args=argparse.Namespace(suite=suite,rust_test="bounded::case",
                    rust_image="fixture-rust",rust_home="/usr/local/rustup",registry=registry,
                    tmp_root=directory,wall_seconds=1,docker_host="unix:///fixture/docker.sock")
                process=MagicMock(returncode=0); process.wait.return_value=0
                inspected=types.SimpleNamespace(stdout=json.dumps({"Os":"linux","Architecture":"arm64","Id":"sha256:fixture"}))
                with patch.object(launcher.shutil,"which",return_value="/fixture/docker"), \
                     patch.object(launcher,"source_snapshot",return_value=(source,"fixture-hash")), \
                     patch.object(launcher,"runtime_security_options") as policies, \
                     patch.object(launcher.subprocess,"run",return_value=inspected), \
                     patch.object(launcher.subprocess,"Popen",return_value=process) as spawned:
                    self.assertEqual(launcher.run(args),0)
                command=spawned.call_args.args[0]
                for value in ["--network=none","--read-only","--user=65534:65534","--cap-drop=ALL",
                              "--pids-limit=96","3584m" if suite == "server" else "1536m",
                              f"/scratch:rw,exec,nosuid,nodev,size={'1536m' if suite == 'server' else '768m'},mode=1777",f"fsize={file_limit}:{file_limit}"]:
                    self.assertIn(value,command)
                self.assertNotIn("--privileged",command)
                self.assertNotIn("systempaths=unconfined",command)
                self.assertIn(f"cargo test -p {package} bounded::case --offline --locked",command[-1])
                if suite == "server":
                    self.assertIn("RUSTFLAGS=-C debuginfo=0 -C codegen-units=256 -C strip=symbols",command)
                    self.assertIn("CARGO_BUILD_JOBS=1",command)
                    self.assertIn("df -k /scratch\ndu -sk /scratch/target",command[-1])
                else:
                    self.assertIn("RUSTFLAGS=-C debuginfo=0 -C codegen-units=8",command)
                policies.assert_not_called()
                if suite == "mcp":
                    self.assertIn("&& cargo clippy -p dunlin-mcp-client -p dunlin-http -p dunlin-vault --all-targets --offline --locked --no-deps -- -D warnings",command[-1])


class RuntimeBrowserTests(unittest.TestCase):
    def test_browser_opt_in_is_exclusive_and_cannot_hide_behind_another_filter(self):
        self.assertFalse(launcher.runtime_browser_enabled(types.SimpleNamespace(suite="runtime")))
        for selected in [None, "production_browser_acceptance_runtime_remote_session_journal", "production_browser_acceptance_runtime_remote_mcp_product_pipeline", "production_browser_acceptance_runtime_remote_scoped_http", "production_browser_acceptance_runtime_local_scoped_http", "production_browser_acceptance_runtime_local_scoped_", "production_browser_acceptance_runtime_local_scoped_recipe_complete", "production_browser_acceptance_runtime_remote_mcp_product_urls", "production_browser_acceptance_runtime_mcp_oauth_revocation", "production_browser_acceptance_runtime_remote_"]:
            self.assertTrue(launcher.runtime_browser_enabled(types.SimpleNamespace(suite="runtime", runtime_browser=True, rust_test=selected)))
        for suite, selected in [("browser", None), ("server", None), ("runtime", "production_browser_acceptance_runtime_worker_execution")]:
            with self.subTest(suite=suite, selected=selected), self.assertRaises(ValueError):
                launcher.runtime_browser_enabled(types.SimpleNamespace(suite=suite, runtime_browser=True, rust_test=selected))

    def test_browser_runtime_keeps_containment_and_uses_only_readonly_dependency_caches(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            (source / "apps/web/e2e").mkdir(parents=True)
            for name in ["registry", "node", "browser"]:
                (root / name).mkdir()
            args = argparse.Namespace(suite="runtime", runtime_browser=True, rust_test="production_browser_acceptance_runtime_remote_session_journal",
                runtime_image="fixture-runtime", runtime_policy_dir=root / "policy", rust_home="/usr/local/rustup",
                registry=root / "registry", node_modules=root / "node", browser_node_modules=root / "browser", tmp_root=directory,
                wall_seconds=1, docker_host="unix:///fixture/docker.sock")
            process = MagicMock(returncode=0)
            process.wait.return_value = 0
            inspected = types.SimpleNamespace(stdout=json.dumps({"Os":"linux", "Architecture":"arm64", "Id":"sha256:fixture"}))
            security = ["--security-opt", "apparmor=dunlin-runtime-checks", "--security-opt", "seccomp=/fixture/pinned.json"]
            with patch.object(launcher.shutil,"which",return_value="/fixture/docker"), \
                 patch.object(launcher,"source_snapshot",return_value=(source,"fixture-hash")), \
                 patch.object(launcher,"runtime_security_options",return_value=security), \
                 patch.object(launcher.subprocess,"run",return_value=inspected), \
                 patch.object(launcher.subprocess,"Popen",return_value=process) as spawned:
                self.assertEqual(launcher.run(args),0)
            command=spawned.call_args.args[0]
            for value in ["--network=none","--read-only","--user=65534:65534","apparmor=dunlin-runtime-checks","seccomp=/fixture/pinned.json","--pids-limit=256","3584m","DUNLIN_REMOTE_SESSION_BROWSER=1","DUNLIN_WEB_E2E_ROOT=/scratch/dist"]:
                self.assertIn(value,command)
            self.assertIn("RUSTFLAGS=-C debuginfo=0 -C codegen-units=256 -C strip=symbols",command)
            self.assertIn("CARGO_BUILD_JOBS=1",command)
            self.assertIn(f"type=bind,src={(root / 'node').resolve()},dst=/repo/apps/web/node_modules,readonly",command)
            self.assertIn(f"type=bind,src={(root / 'browser').resolve()},dst=/repo/apps/web/e2e/node_modules,readonly",command)
            self.assertNotIn("--privileged",command)
            payload=command[-1]
            self.assertIn("cd /repo/apps/web\nnode node_modules/vite/bin/vite.js build",payload)
            self.assertIn("cd /repo\ntest -x /usr/bin/bwrap",payload)
            self.assertIn("cargo build -p dunlin-relay --offline --locked",payload)
            self.assertIn("production_browser_acceptance_runtime_remote_session_journal --offline --locked -- --ignored",payload)


class RuntimePolicyTests(unittest.TestCase):
    def test_runtime_policy_rejects_changed_bytes_or_symlinks_before_use(self):
        import hashlib
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            contents = {"runtime-seccomp.json": b"fixture-seccomp", "runtime.apparmor": b"fixture-apparmor"}
            hashes = {name: hashlib.sha256(data).hexdigest() for name, data in contents.items()}
            for name, data in contents.items():
                (root / name).write_bytes(data)
            with patch.object(launcher, "RUNTIME_POLICIES", hashes):
                options = launcher.runtime_security_options(root)
                self.assertIn("apparmor=dunlin-runtime-checks", options)
                self.assertIn("systempaths=unconfined", options)
                (root / "runtime-seccomp.json").write_bytes(b"changed")
                with self.assertRaises(ValueError):
                    launcher.runtime_security_options(root)
                (root / "runtime-seccomp.json").unlink()
                (root / "other").write_bytes(contents["runtime-seccomp.json"])
                (root / "runtime-seccomp.json").symlink_to("other")
                with self.assertRaises(ValueError):
                    launcher.runtime_security_options(root)


class SourceTests(unittest.TestCase):
    def test_source_copy_excludes_secrets_and_build_caches_without_following_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            source = root / "src"
            source.mkdir()
            (source / "ok.txt").write_text("fixture")
            for name in [".env", ".env.local", "local.db"]:
                (source / name).write_text("private")
            for name in ["node_modules", "target", ".git", "dist"]:
                (source / name).mkdir()
                (source / name / "private").write_text("private")
            (source / "link").symlink_to("/not-a-source-path")
            output = Path(directory) / "out"
            output.mkdir()
            with patch.object(launcher, "ROOT", root):
                snapshot, digest = launcher.source_snapshot(output, ["src"])
            self.assertEqual(sorted(p.name for p in (snapshot / "src").iterdir()), ["link", "ok.txt"])
            self.assertTrue((snapshot / "src/link").is_symlink())
            self.assertEqual(len(digest), 64)

    def test_oversized_file_is_rejected_before_copying(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            root.mkdir()
            huge = root / "huge"
            huge.write_text("fixture")
            original_stat = Path.stat

            def oversized_stat(path, *args, **kwargs):
                result = original_stat(path, *args, **kwargs)
                if path == huge:
                    fields = list(result)
                    fields[6] = 128 * 1024**2 + 1
                    return os.stat_result(fields)
                return result
            output = Path(directory) / "out"
            output.mkdir()
            with patch.object(launcher, "ROOT", root), patch.object(Path, "stat", oversized_stat), self.assertRaises(ValueError):
                launcher.source_snapshot(output, ["huge"])
            self.assertFalse((output / "source/huge").exists())


class CleanupTests(unittest.TestCase):
    def launch(self, error):
        with tempfile.TemporaryDirectory() as directory:
            args = argparse.Namespace(suite="probe", docker_host="unix:///fixture/docker.sock", tmp_root=directory,
                                      node_image="fixture-node", wall_seconds=1)
            process = MagicMock()
            process.returncode = None
            process.wait.side_effect = [error, 0]
            image = types.SimpleNamespace(stdout=json.dumps({"Os": "linux", "Architecture": "arm64", "Id": "sha256:fixture"}))
            with patch.object(launcher.shutil, "which", return_value="/fixture/docker"), \
                 patch.object(launcher, "source_snapshot", return_value=(Path(directory), "fixture-hash")), \
                 patch.object(launcher.subprocess, "run", return_value=image) as commands, \
                 patch.object(launcher.subprocess, "Popen", return_value=process) as spawned:
                with self.assertRaises(type(error)):
                    launcher.run(args)
                command = spawned.call_args.args[0]
                name = command[command.index("--name") + 1]
                self.assertTrue(name.startswith("dunlin-check-"))
                self.assertEqual(commands.call_args.args[0][-3:], ["rm", "--force", name])
                self.assertEqual(set(spawned.call_args.kwargs["env"]), {"HOME", "PATH"})
                for flag in ["--network=none", "--read-only", "--cap-drop=ALL", "--user=65534:65534", "--pull=never"]:
                    self.assertIn(flag, command)
                self.assertIn("/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777", command)
                process.kill.assert_called_once()

    def test_timeout_removes_only_this_invocations_container(self):
        self.launch(subprocess.TimeoutExpired("fixture", 1))

    def test_termination_unwinds_through_scoped_cleanup(self):
        self.launch(SystemExit(143))


class MacProfileTests(unittest.TestCase):
    def fixture(self, repo):
        repo = repo.resolve()
        root = repo / "target/web-checks" / ("macos-metadata-" + "a" * 32)
        (root / "home").mkdir(parents=True)
        (root / "fixture.json").write_text(json.dumps({"schema_version": 1, "fixture_root": str(root), "home": str(root / "home")}))
        return root

    def test_profile_precheck_filter_is_validated_and_cannot_select_ignored_flags(self):
        self.assertEqual(launcher.rust_test_arg(types.SimpleNamespace(suite="macos-profile", rust_test="execution")), " execution")
        for value in ["--ignored", "execution;payload", "execution$(payload)", "execution\n--ignored", "x" * 257]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                launcher.rust_test_arg(types.SimpleNamespace(suite="macos-profile", rust_test=value))

    def test_fixture_flag_is_exclusive_required_canonical_and_symlink_free(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launcher, "ROOT", Path(directory)):
            root = self.fixture(Path(directory))
            options = types.SimpleNamespace(suite="macos-profile", macos_profile_fixture=root)
            self.assertEqual(launcher.macos_profile_fixture(options), root)
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(types.SimpleNamespace(suite="server", macos_profile_fixture=root))
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(types.SimpleNamespace(suite="macos-profile"))
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(types.SimpleNamespace(suite="macos-profile", macos_profile_fixture=Path("relative")))
            (root / "alias").symlink_to(root / "home")
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(options)

    def test_fixture_rejects_descriptor_scope_drift_and_oversized_regular_file(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(launcher, "ROOT", Path(directory)):
            root = self.fixture(Path(directory))
            options = types.SimpleNamespace(suite="macos-profile", macos_profile_fixture=root)
            (root / "fixture.json").write_text(json.dumps({"schema_version": 1, "fixture_root": str(root), "home": "/not-the-fixture-home"}))
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(options)
            (root / "oversized").write_bytes(b"x" * (1024 * 1024 + 1))
            with self.assertRaises(ValueError):
                launcher.macos_profile_fixture(options)

    def test_generator_keeps_readonly_samepath_mount_and_separate_cargo_home_without_lsm(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            root = self.fixture(repo)
            registry = repo / "registry"
            registry.mkdir()
            source = repo / "source"
            source.mkdir()
            args = argparse.Namespace(suite="macos-profile", rust_test="execution", macos_profile_fixture=root, docker_host="unix:///fixture/docker.sock", tmp_root=directory,
                                      rust_image="fixture-rust", rust_home="/usr/local/rustup", wall_seconds=1, registry=registry)
            process = MagicMock(returncode=0)
            process.wait.return_value = 0
            inspected = types.SimpleNamespace(stdout=json.dumps({"Os": "linux", "Architecture": "arm64", "Id": "sha256:fixture"}))
            with patch.object(launcher, "ROOT", repo), patch.object(launcher.shutil, "which", return_value="/fixture/docker"), \
                 patch.object(launcher, "source_snapshot", return_value=(source, "fixture-hash")), \
                 patch.object(launcher.subprocess, "run", return_value=inspected), \
                 patch.object(launcher.subprocess, "Popen", return_value=process) as spawned:
                self.assertEqual(launcher.run(args), 0)
            command = spawned.call_args.args[0]
            self.assertIn(f"type=bind,src={root},dst={root},readonly", command)
            self.assertIn(f"HOME={root / 'home'}", command)
            self.assertNotIn("HOME=/scratch", command)
            for value in ["CARGO_HOME=/scratch/cargo", "RUSTUP_HOME=/usr/local/rustup", "TMPDIR=/scratch", "--network=none", "--read-only", "--user=65534:65534"]:
                self.assertIn(value, command)
            self.assertNotIn("apparmor=dunlin-runtime-checks", command)
            payload = command[-1]
            self.assertIn("--exact acp::metadata_tests::profile_export::export_production_macos_metadata_profiles --ignored --nocapture", payload)
            self.assertNotIn("production_browser_acceptance_runtime_", payload)
            precheck = "cargo test -p dunlin-relay execution --offline --locked"
            self.assertIn(precheck, payload)
            self.assertLess(payload.index(precheck), payload.index("--exact acp::metadata_tests::profile_export::export_production_macos_metadata_profiles"))
            self.assertNotIn("--ignored", payload[payload.index(precheck):payload.index(precheck) + len(precheck)])


class RuntimeFilterTests(unittest.TestCase):
    def test_only_bounded_runtime_acceptance_names_are_accepted(self):
        selected = "worker_fixture::production_browser_acceptance_runtime_worker_execution_transport"
        self.assertEqual(launcher.rust_test_arg(types.SimpleNamespace(suite="runtime", rust_test=selected)), " " + selected)
        for value in ["helper", "export_production_macos_metadata_profiles", "execution", "production_browser_acceptance_runtime_x;payload", "production_browser_acceptance_runtime_$(payload)", "production_browser_acceptance_runtime_x\n--ignored", "module::helper", "--ignored", "production_browser_acceptance_runtime_" + "x" * 257]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                launcher.rust_test_arg(types.SimpleNamespace(suite="runtime", rust_test=value))
        with self.assertRaises(ValueError):
            launcher.rust_test_arg(types.SimpleNamespace(suite="browser", rust_test=selected))

    def test_focused_runtime_retains_actual_containment_and_binary_build_but_not_full_pretests(self):
        selected = "worker_fixture::production_browser_acceptance_runtime_worker_execution_transport"
        for selection in [selected, None]:
            with self.subTest(selection=selection), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source = root / "source"
                source.mkdir()
                registry = root / "registry"
                registry.mkdir()
                args = argparse.Namespace(suite="runtime", rust_test=selection, runtime_image="fixture-runtime", runtime_policy_dir=root / "policy",
                                          rust_home="/usr/local/rustup", registry=registry, tmp_root=directory, wall_seconds=1,
                                          docker_host="unix:///fixture/docker.sock")
                process = MagicMock(returncode=0)
                process.wait.return_value = 0
                inspected = types.SimpleNamespace(stdout=json.dumps({"Os": "linux", "Architecture": "arm64", "Id": "sha256:runtime-fixture"}))
                security = ["--security-opt", "apparmor=dunlin-runtime-checks", "--security-opt", "systempaths=unconfined", "--security-opt", "seccomp=/fixture/pinned-seccomp.json"]
                output = io.StringIO()
                with patch.object(launcher.shutil, "which", return_value="/fixture/docker"), \
                     patch.object(launcher, "source_snapshot", return_value=(source, "fixture-hash")), \
                     patch.object(launcher, "runtime_security_options", return_value=security) as policy, \
                     patch.object(launcher.subprocess, "run", return_value=inspected), \
                     patch.object(launcher.subprocess, "Popen", return_value=process) as spawned, contextlib.redirect_stdout(output):
                    self.assertEqual(launcher.run(args), 0)
                policy.assert_called_once_with(args.runtime_policy_dir)
                command = spawned.call_args.args[0]
                self.assertIn("RUSTFLAGS=-C debuginfo=0 -C codegen-units=256 -C strip=symbols", command)
                self.assertIn("CARGO_BUILD_JOBS=1", command)
                for required in ["apparmor=dunlin-runtime-checks", "systempaths=unconfined", "seccomp=/fixture/pinned-seccomp.json", "--network=none", "--read-only", "--user=65534:65534", "3584m", "/scratch:rw,exec,nosuid,nodev,size=1536m,mode=1777", "HOME=/scratch", "CARGO_HOME=/scratch/cargo", "DUNLIN_RELAY_E2E_BIN=/scratch/target/debug/dunlin-relay"]:
                    self.assertIn(required, command)
                payload = command[-1]
                self.assertIn("bwrap --die-with-parent --new-session --unshare-all", payload)
                self.assertIn("cargo build -p dunlin-relay --offline --locked", payload)
                self.assertIn("runtime_resources", payload)
                # The test binary is linked without retaining the production
                # executable; acceptance still runs only after the real build.
                build = payload.index("cargo build -p dunlin-relay --offline --locked")
                tests = payload.index("cargo test -p dunlin-relay")
                self.assertLess(tests, build)
                self.assertNotIn("cargo clean", payload)
                self.assertIn("df -k /scratch\ndu -sk /scratch/target", payload)
                full = "cargo test -p dunlin-relay -p dunlin-identity -p dunlin-session-runtime -p dunlin-terminal -p dunlin-usage -p dunlin-workspace --offline --locked --no-fail-fast"
                if selection:
                    precompile = "cargo test -p dunlin-relay --offline --locked --no-run"
                    self.assertIn(precompile, payload)
                    self.assertLess(payload.index(precompile), build)
                    acceptance = f"cargo test -p dunlin-relay {selected} --offline --locked -- --ignored --nocapture"
                    self.assertLess(build, payload.index(acceptance))
                    self.assertNotIn(full, payload)
                    self.assertIn(f"cargo test -p dunlin-relay {selected} --offline --locked -- --ignored --nocapture", payload)
                    self.assertIn(f"scope=runtime-focused-acceptance-only filter={selected}", output.getvalue())
                else:
                    self.assertIn(full, payload)
                    precompile = full.replace("--no-fail-fast", "--no-run")
                    self.assertIn(precompile, payload)
                    self.assertLess(payload.index(precompile), build)
                    self.assertLess(build, payload.index(full))
                    self.assertLess(payload.index(full), payload.index("cargo test -p dunlin-relay production_browser_acceptance_runtime_"))
                    self.assertIn("cargo test -p dunlin-relay production_browser_acceptance_runtime_ --offline --locked -- --ignored --nocapture", payload)
                    self.assertIn("scope=runtime-full-regression", output.getvalue())


if __name__ == "__main__":
    unittest.main()
