"""CI download integrity tests; no application or OS configuration is started."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("ci_build", ROOT / "scripts/build-ci.py")
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class ArchiveTests(unittest.TestCase):
    def test_archives_preserve_binaries_and_checksums(self):
        for windows in (False, True):
            with self.subTest(windows=windows), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                stage = root / "hps-ci"
                (stage / "bin").mkdir(parents=True)
                suffix = ".exe" if windows else ""
                for name in ("hps", "hpsd"):
                    binary = stage / "bin" / (name + suffix)
                    binary.write_bytes(b"compiled binary fixture\x00\xff")
                    binary.chmod(0o755)
                ci.write_json(stage / "manifest.json", {"release_qualified": False})
                ci.checksums(stage)
                archive = ci.archive_headless(stage, root, windows)
                checksum = archive.with_name(archive.name + ".sha256").read_text().split()[0]
                self.assertEqual(checksum, ci.digest(archive))
                if windows:
                    with zipfile.ZipFile(archive) as contents:
                        self.assertEqual(contents.read("hps-ci/bin/hpsd.exe"), binary.read_bytes())
                        self.assertIn("hps-ci/SHA256SUMS", contents.namelist())
                else:
                    with tarfile.open(archive) as contents:
                        member = contents.getmember("hps-ci/bin/hpsd")
                        self.assertEqual(member.mode & 0o111, 0o111)
                        self.assertEqual(contents.extractfile(member).read(), binary.read_bytes())
                for line in (stage / "SHA256SUMS").read_text().splitlines():
                    expected, path = line.split("  ", 1)
                    self.assertEqual(expected, ci.digest(stage / path))
                with self.assertRaises(FileExistsError):
                    ci.archive_headless(stage, root, windows)


class WebTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        (self.root / "web/console").mkdir(parents=True)
        ci.write_json(self.root / "web/console/pages.manifest.json", [{"file": "index.html"}])
        (self.dist / "index.html").write_bytes(b"<html>CI page</html>")
        self.web = {"pages": ["index.html"], "assets": {"index.html": ci.digest(self.dist / "index.html")}}
        self.save()
        self.patch = mock.patch.object(ci, "ROOT", self.root)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def save(self):
        canonical = json.dumps(self.web["assets"], sort_keys=True, separators=(",", ":")).encode()
        self.web["web_build_id"] = hashlib.sha256(canonical).hexdigest()
        ci.write_json(self.dist / "web-manifest.json", self.web)

    def test_valid_web_and_tampered_bytes(self):
        self.assertEqual(ci.verify_web(self.dist), self.web)
        (self.dist / "index.html").write_bytes(b"modified")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            ci.verify_web(self.dist)

    def test_rejects_incomplete_pages_and_wrong_build_id(self):
        self.web["assets"] = {}
        self.save()
        with self.assertRaisesRegex(ValueError, "page missing"):
            ci.verify_web(self.dist)
        self.web["assets"] = {"index.html": ci.digest(self.dist / "index.html")}
        self.save()
        self.web["web_build_id"] = "stale"
        ci.write_json(self.dist / "web-manifest.json", self.web)
        with self.assertRaisesRegex(ValueError, "build ID mismatch"):
            ci.verify_web(self.dist)

    def test_rejects_unsafe_paths_on_every_platform(self):
        for path in ("../outside", "/tmp/outside", "C:/outside", "..\\outside"):
            with self.subTest(path=path):
                self.web["assets"] = {path: "unused"}
                self.save()
                with self.assertRaisesRegex(ValueError, "unsafe Web asset path"):
                    ci.verify_web(self.dist)


class MetadataTests(unittest.TestCase):
    def test_checks_source_web_and_architecture_for_all_targets(self):
        for target, (platform, suffix, _) in ci.TARGETS.items():
            with self.subTest(target=target):
                info = {"format_version": 1, "binary": "hpsd", "source_revision": "abc123",
                        "source_dirty": False, "architecture": target.split("-")[0],
                        "platform": platform, "version": {"web_build_id": "web123"}}
                for change in ({}, {"source_dirty": True}, {"source_revision": "old"},
                               {"architecture": "other"}, {"version": {"web_build_id": "old"}}):
                    result = copy.deepcopy(info)
                    result.update(change)
                    completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(result))
                    with mock.patch.object(ci.subprocess, "run", return_value=completed):
                        if change:
                            with self.assertRaises(ValueError):
                                ci.metadata(Path("hpsd" + suffix), "hpsd", target, "abc123", {"web_build_id": "web123"}, {})
                        else:
                            self.assertEqual(ci.metadata(Path("hpsd" + suffix), "hpsd", target, "abc123", {"web_build_id": "web123"}, {}), info)


if __name__ == "__main__":
    unittest.main()
