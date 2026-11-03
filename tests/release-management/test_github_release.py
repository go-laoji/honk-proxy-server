"""Test artifact integrity and the GitHub publish state machine without any network calls."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
spec = importlib.util.spec_from_file_location("github_release", ROOT / "scripts/github-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="hps-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "downloads"
        self.output = self.root / "assets"
        self.source.mkdir()
        self.revision = "a" * 40
        self.web = "b" * 64
        self.version = "0.2.0-rc.1"
        self.tag = "v" + self.version
        for target, (platform, archive, installer) in release.TARGETS.items():
            for package, suffix in (("headless", archive), ("desktop", installer)):
                folder = self.source / f"{package}-{target}"
                folder.mkdir()
                binary = folder / f"hps-{package}-{self.version}-{target}{suffix}"
                binary.write_bytes((target + package).encode())
                if package == "headless":
                    binary.with_name(binary.name + ".sha256").write_text(f"{release.digest(binary)}  {binary.name}\n")
                names = ("hps", "hpsd") if package == "headless" else ("hpsd", "desktop")
                metadata = {name: {"format_version": 1, "binary": "hps-desktop" if name == "desktop" else name,
                                   "source_revision": self.revision, "source_dirty": False,
                                   "platform": platform, "architecture": target.split("-")[0],
                                   "version": {"version": self.version, "web_build_id": self.web}}
                            for name in names}
                release.write_json(folder / "manifest.json", {
                    "package": package, "target": target, "source_revision": self.revision,
                    "web_build_id": self.web, "binaries": metadata, "artifact_kind": "ci-build",
                    "release_qualified": False, "ebpf_object_included": False,
                })
                self.checksums(folder)
        self.remote = None
        self.calls = []
        self.fail_upload = False

    def checksums(self, folder):
        (folder / "SHA256SUMS").write_text("".join(f"{release.digest(path)}  {path.name}\n"
            for path in sorted(folder.iterdir()) if path.name != "SHA256SUMS"))

    def prepare(self):
        return release.prepare(self.source, self.output, self.tag, self.revision, self.version)

    def fake_gh(self, *args, check=True):
        self.calls.append(args)
        if args[0] == "api":
            if "--slurp" in args:
                pages = [[self.remote]] if self.remote is not None else [[]]
                return subprocess.CompletedProcess(args, 0, json.dumps(pages), "")
            # Match the real API: /releases/tags/{tag} cannot read drafts.
            status = 404 if self.remote is None or self.remote["draft"] else 200
            return subprocess.CompletedProcess(args, int(status == 404),
                f"HTTP/2.0 {status} Test\nContent-Type: application/json\n\n" + json.dumps(self.remote), "")
        if args[:2] == ("release", "create"):
            self.assertIn("--verify-tag", args)
            self.assertIn("--draft", args)
            self.remote = {"tag_name": self.tag, "prerelease": "--prerelease" in args, "draft": True, "assets": []}
        elif args[:2] == ("release", "upload"):
            self.assertNotIn("--clobber", args)
            for item in args[5:]:
                path = Path(item)
                self.remote["assets"].append({"name": path.name, "digest": "sha256:" + release.digest(path)})
                if self.fail_upload:
                    raise subprocess.CalledProcessError(1, args, stderr="fixture upload interrupted")
        elif args[:2] == ("release", "edit"):
            self.assertIn("--draft=false", args)
            self.remote["draft"] = False
        else:
            self.fail(f"unexpected GitHub operation: {args}")
        return subprocess.CompletedProcess(args, 0, "", "")

    def test_prepares_ten_packages_and_unique_manifests_with_global_checksums(self):
        manifest = self.prepare()
        self.assertEqual(len(list(self.output.iterdir())), 27)
        self.assertEqual(len(list(self.output.glob("*-manifest.json"))), 11)
        self.assertEqual(len(manifest["targets"]), 5)
        self.assertFalse(manifest["release_qualified"])
        release.verified_files(self.output)

    def test_missing_corrupt_stale_and_wrong_version_artifacts_are_rejected(self):
        target = next(iter(release.TARGETS))
        folder = self.source / ("headless-" + target)
        manifest_path = folder / "manifest.json"
        original = json.loads(manifest_path.read_text())
        for change in ({"source_revision": "c" * 40}, {"web_build_id": "c" * 64}, {"release_qualified": True}):
            with self.subTest(change=change):
                manifest = copy.deepcopy(original)
                manifest.update(change)
                release.write_json(manifest_path, manifest)
                self.checksums(folder)
                with self.assertRaises(ValueError):
                    self.prepare()
        release.write_json(manifest_path, original)
        self.checksums(folder)
        with self.assertRaisesRegex(ValueError, "tag does not match"):
            release.prepare(self.source, self.output, "v9.0.0", self.revision, self.version)
        manifest_path.write_text("corrupt")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.prepare()
        shutil.rmtree(folder)
        with self.assertRaisesRegex(ValueError, "all ten"):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_initial_publish_and_replay_do_not_replace_assets(self):
        self.prepare()
        with mock.patch.object(release, "gh", side_effect=self.fake_gh):
            release.publish(self.output, "fixture/repo", self.tag, self.revision)
            self.assertFalse(self.remote["draft"])
            self.assertTrue(self.remote["prerelease"])
            before = list(self.calls)
            release.publish(self.output, "fixture/repo", self.tag, self.revision)
            self.assertTrue(all(call[0] == "api" for call in self.calls[len(before):]))
            self.assertEqual(len(self.remote["assets"]), 27)

    def test_interrupted_upload_keeps_draft_and_retry_completes_it(self):
        self.prepare()
        self.fail_upload = True
        with mock.patch.object(release, "gh", side_effect=self.fake_gh):
            with self.assertRaises(subprocess.CalledProcessError):
                release.publish(self.output, "fixture/repo", self.tag, self.revision)
            self.assertTrue(self.remote["draft"])
            self.fail_upload = False
            release.publish(self.output, "fixture/repo", self.tag, self.revision)
        self.assertFalse(self.remote["draft"])
        self.assertEqual(len(self.remote["assets"]), 27)
        self.assertEqual(sum(call[:2] == ("release", "create") for call in self.calls), 1)

    def test_existing_different_asset_is_rejected_before_any_write(self):
        self.prepare()
        first = next(self.output.iterdir())
        self.remote = {"tag_name": self.tag, "prerelease": True, "draft": False,
                       "assets": [{"name": first.name, "digest": "sha256:" + "0" * 64}]}
        with mock.patch.object(release, "gh", side_effect=self.fake_gh):
            with self.assertRaisesRegex(ValueError, "refusing to replace"):
                release.publish(self.output, "fixture/repo", self.tag, self.revision)
        self.assertTrue(all(call[0] == "api" for call in self.calls))

    def test_auth_failure_is_not_treated_as_a_missing_release(self):
        result = subprocess.CompletedProcess([], 1, "HTTP/2.0 403 Forbidden\n\n{}", "denied")
        with mock.patch.object(release, "gh", return_value=result):
            with self.assertRaisesRegex(ValueError, "lookup failed"):
                release.remote_release("fixture/repo", self.tag)

    def test_draft_is_found_through_authenticated_list_after_tag_404(self):
        self.remote = {"tag_name": self.tag, "prerelease": True, "draft": True, "assets": []}
        with mock.patch.object(release, "gh", side_effect=self.fake_gh):
            self.assertEqual(release.remote_release("fixture/repo", self.tag), self.remote)
        self.assertIn("--paginate", self.calls[1])
        self.assertIn("--slurp", self.calls[1])

    def test_ambiguous_drafts_fail_before_any_write(self):
        draft = {"tag_name": self.tag, "draft": True}
        responses = [subprocess.CompletedProcess([], 1, "HTTP/2.0 404 Not Found\n\n{}", ""),
                     subprocess.CompletedProcess([], 0, json.dumps([[draft, draft]]), "")]
        with mock.patch.object(release, "gh", side_effect=responses):
            with self.assertRaisesRegex(ValueError, "multiple Releases"):
                release.remote_release("fixture/repo", self.tag)


if __name__ == "__main__":
    unittest.main()
