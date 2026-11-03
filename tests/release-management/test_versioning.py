"""Exercise the real cargo-release transaction only inside an owned temporary repository."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import versioning


class VersioningTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hps-version-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "repo"
        self.root.mkdir()
        self.env = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1", CARGO_NET_OFFLINE="true",
                        CARGO_TARGET_DIR=str(self.root / "target"))
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "NEW_VERSION", "DRY_RUN"):
            self.env.pop(key, None)
        self.write("Cargo.toml", '[workspace]\nmembers = ["apps/cli", "crates/ebpf-abi"]\nexclude = ["src-tauri", "ebpf"]\nresolver = "2"\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n')
        self.write(".gitignore", "target/\n**/target/\n")
        shutil.copyfile(ROOT / "release.toml", self.root / "release.toml")
        self.write("apps/cli/Cargo.toml", '[package]\nname = "hps-cli"\nversion.workspace = true\nedition.workspace = true\n[package.metadata.release]\npre-release-hook = ["' + sys.executable.replace('\\', '\\\\') + '", "../../scripts/versioning.py", "--hook"]\n[dependencies]\nhps-ebpf-abi = { path = "../../crates/ebpf-abi" }\n')
        self.write("apps/cli/src/main.rs", "fn main() {}\n")
        self.write("crates/ebpf-abi/Cargo.toml", '[package]\nname = "hps-ebpf-abi"\nversion.workspace = true\nedition.workspace = true\n')
        self.write("crates/ebpf-abi/src/lib.rs", "pub const ABI: u32 = 4;\n")
        for relative, name in (("src-tauri", "hps-desktop"), ("ebpf/hps-ebpf", "hps-ebpf")):
            dependency = "../crates/ebpf-abi" if relative == "src-tauri" else "../../crates/ebpf-abi"
            self.write(relative + "/Cargo.toml", f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\nhps-ebpf-abi = {{ path = "{dependency}" }}\n')
            self.write(relative + "/src/main.rs", "fn main() {}\n")
        self.write("ebpf/Cargo.toml", '[workspace]\nmembers = ["hps-ebpf"]\nresolver = "2"\n')
        self.write("package.json", json.dumps({"name": "fixture", "version": "0.1.0", "private": True}))
        self.write("src-tauri/tauri.conf.json", json.dumps({"version": "0.1.0"}))
        self.write("scripts/versioning.py", (ROOT / "scripts/versioning.py").read_text())
        for manifest in ("Cargo.toml", "src-tauri/Cargo.toml", "ebpf/Cargo.toml"):
            self.run_command("cargo", "generate-lockfile", "--offline", "--manifest-path", manifest)
        self.run_command("git", "init", "-b", "develop")
        self.run_command("git", "config", "user.name", "Release fixture")
        self.run_command("git", "config", "user.email", "fixture@example.invalid")
        self.run_command("git", "config", "commit.gpgsign", "false")
        self.run_command("git", "config", "tag.gpgsign", "false")
        self.run_command("git", "config", "core.excludesFile", str(self.root / ".git/fixture-excludes"))
        self.run_command("git", "config", "core.hooksPath", str(self.root / ".git/fixture-hooks"))
        self.run_command("git", "add", ".")
        self.run_command("git", "commit", "-m", "fixture: initial product")
        self.run_command("git", "tag", "-a", "v0.1.0", "-m", "fixture baseline")
        # A deliberately unusable remote makes an accidental push fail the test.
        self.run_command("git", "remote", "add", "origin", str(self.root / "does-not-exist.git"))
        self.run_command("git", "config", "branch.develop.remote", "origin")
        self.run_command("git", "config", "branch.develop.merge", "refs/heads/develop")

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def run_command(self, *args):
        result = subprocess.run(args, cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout.strip()

    def test_real_dry_run_and_execute_keep_all_versions_and_one_tag(self):
        before = {str(p.relative_to(self.root)): p.read_bytes() for p in self.root.rglob("*")
                  if p.is_file() and ".git" not in p.parts}
        result = self.run_command("cargo", "release", "patch", "--workspace", "--no-confirm")
        self.assertIn("Would synchronize", result)
        self.assertEqual(before, {p: (self.root / p).read_bytes() for p in before})
        self.assertEqual(self.run_command("git", "tag", "--list"), "v0.1.0")
        result = self.run_command("cargo", "release", "patch", "--workspace", "--execute", "--no-confirm")
        self.assertEqual(result.count("Synchronized all product manifests"), 1)
        self.assertEqual(versioning.check_versions(self.root, "v0.1.1"), "0.1.1")
        self.assertEqual(self.run_command("git", "rev-list", "--count", "HEAD"), "2")
        self.assertEqual(self.run_command("git", "log", "-1", "--format=%s"), "chore(release): v0.1.1")
        self.assertEqual(self.run_command("git", "status", "--porcelain"), "")
        self.assertEqual(self.run_command("git", "tag", "--list"), "v0.1.0\nv0.1.1")
        self.assertEqual(self.run_command("git", "rev-parse", "v0.1.1^{commit}"), self.run_command("git", "rev-parse", "HEAD"))
        # Cargo must accept all three resulting locks without re-resolving anything.
        for manifest in ("Cargo.toml", "src-tauri/Cargo.toml", "ebpf/Cargo.toml"):
            self.run_command("cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1", "--manifest-path", manifest)
        self.run_command("cargo", "release", "0.2.0-rc.1", "--workspace", "--execute", "--no-confirm")
        self.assertEqual(versioning.check_versions(self.root, "v0.2.0-rc.1"), "0.2.0-rc.1")
        self.assertEqual(self.run_command("git", "rev-list", "--count", "HEAD"), "3")
        self.run_command("cargo", "release", "0.3.0", "--metadata", "build.7", "--workspace", "--execute", "--no-confirm")
        self.assertEqual(versioning.check_versions(self.root, "v0.3.0"), "0.3.0+build.7")
        self.assertEqual(self.run_command("git", "rev-list", "--count", "HEAD"), "4")

    def test_rejects_mismatched_tag_and_independent_version(self):
        with self.assertRaisesRegex(ValueError, "does not match"):
            versioning.check_versions(self.root, "v9.0.0")
        self.write("package.json", '{"version": "9.0.0"}')
        with self.assertRaisesRegex(ValueError, "package.json"):
            versioning.check_versions(self.root)


if __name__ == "__main__":
    unittest.main()
