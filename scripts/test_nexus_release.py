import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_prep", ROOT / "scripts/prepare_nexus_release.py")
prep = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prep)

class PreparationTests(unittest.TestCase):
    def test_minimal_patches_apply_only_to_pinned_manifests(self):
        recipes = json.loads(prep.SPEC.read_text())
        for name, recipe in recipes.items():
            original = subprocess.check_output(
                ["git", "-C", str(ROOT / name), "show", recipe["commit"] + ":Cargo.toml"], text=True)
            result = prep.patch_manifest(original, recipe)
            self.assertNotIn('name = "auto_download"', result)
            self.assertIn('candle-flash-attn = { version = "0.9', result)
            with self.assertRaises(ValueError):
                prep.patch_manifest(original + "\n", recipe)

    def test_archive_rejects_links_before_extraction(self):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode="w") as tf:
            member = tarfile.TarInfo("escape")
            member.type = tarfile.SYMTYPE
            member.linkname = "../../outside"
            tf.addfile(member)
        with tempfile.TemporaryDirectory() as t, patch.object(prep.subprocess, "check_output", return_value=data.getvalue()):
            with self.assertRaises(ValueError):
                prep.archive(Path(t), "pinned", Path(t))

    def test_changed_source_and_fixture_are_rejected(self):
        with tempfile.TemporaryDirectory() as t, patch.object(prep, "expected_sources") as origin:
            p = Path(t)
            (p / "source").mkdir()
            (p / "source/Cargo.toml").write_text("original")
            (p / "reading_article.json").write_text("{}")
            files = prep.inputs(p / "source")
            origin.return_value = (files.copy(), {})
            prep.save(p / "source-manifest.json",
                      dict(source_commit="a"*40, path_dependencies={}, target_dir=str(prep.TARGET), files=files,
                           source_tree_sha256=prep.tree_hash(files),
                           fixture_sha256=prep.sha(b"{}")))
            prep.load(p)
            (p / "source/Cargo.toml").write_text("changed")
            with self.assertRaises(ValueError):
                prep.load(p)
            (p / "source/Cargo.toml").write_text("original")
            (p / "reading_article.json").write_text("changed")
            with self.assertRaises(ValueError):
                prep.load(p)

    def test_rewritten_manifest_cannot_hide_changed_source(self):
        with tempfile.TemporaryDirectory() as t, patch.object(prep, "expected_sources") as origin:
            p = Path(t)
            (p / "source").mkdir()
            (p / "source/Cargo.toml").write_text("modified")
            (p / "reading_article.json").write_text("{}")
            files = prep.inputs(p / "source")
            origin.return_value = ({"Cargo.toml": prep.sha(b"original")}, {})
            prep.save(p / "source-manifest.json",
                      dict(source_commit="a"*40, path_dependencies={},
                           target_dir=str(prep.TARGET), files=files,
                           source_tree_sha256=prep.tree_hash(files),
                           fixture_sha256=prep.sha(b"{}")))
            with self.assertRaises(ValueError):
                prep.load(p)

    def test_partial_reading_only_proof_does_not_qualify_as_full_suite(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t)
            log = b"test result: ok. 8 passed; 0 failed;\n"
            (p / "nexus-tests.log").write_bytes(log)
            m = dict(source_commit="a"*40, source_tree_sha256="b"*64)
            proof = dict(m, passed=True, exit_code=0, full_nexus_suite=True,
                         tests_passed=8, log_sha256=prep.sha(log),
                         command=["cargo","test","--locked","--offline","-p","nexus",
                                  "-j","2","--","--test-threads=1"])
            prep.save(p / "test-result.json", proof)
            with self.assertRaises(ValueError):
                prep.test_proof(p, m)

    def test_no_test_proof_cannot_build(self):
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(FileNotFoundError):
                prep.test_proof(Path(t), {})

    def test_platform_gate_rejects_macos_and_wrong_elf_arch(self):
        self.assertFalse(prep.elf_x86_64(b"\xcf\xfa\xed\xfe" + bytes(100)))
        data = bytearray(64)
        data[:6] = b"\x7fELF\x02\x01"
        data[18] = 183
        self.assertFalse(prep.elf_x86_64(data))

class RemoteBinaryContractTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.app = self.base / "app"
        self.app.mkdir()
        self.old = b"old binary"
        self.new = b"new binary"
        (self.app / "nexus").write_bytes(self.old)
        (self.base / "running").write_bytes(self.old)
        self.candidate = self.base / "candidate"
        self.candidate.write_bytes(self.new)
        self.old_sha = hashlib.sha256(self.old).hexdigest()
        self.new_sha = hashlib.sha256(self.new).hexdigest()
        self.release_id = "a" * 40 + "-" + self.new_sha[:12]
        self.backup = self.app / ("nexus.rollback." + self.release_id)
        for f in ("config.env", "nexus.service", "nexus.db"):
            (self.app / f).write_text("protected " + f)
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.env = dict(os.environ, PATH=str(self.bin) + ":" + os.environ["PATH"], FIXTURE=str(self.base))
        fake = """#!/usr/bin/env python3
import hashlib,os,pathlib,shutil,sys,signal
b=pathlib.Path(os.environ['FIXTURE']); name=pathlib.Path(sys.argv[0]).name; a=sys.argv[1:]
with (b/'calls').open('a') as f:f.write(name+' '+' '.join(a)+'\\n')
if name=='sha256sum':
 p=b/'running' if a[0].startswith('/proc/') else pathlib.Path(a[0])
 print(hashlib.sha256(p.read_bytes()).hexdigest()+'  '+a[0])
elif name=='systemctl':
 if a[0]=='show':print('123')
 elif a==['restart','nexus']:
  if (b/'interrupt_restart').exists() and (b/'app/nexus').read_bytes()==b'new binary':os.kill(os.getppid(),signal.SIGHUP)
  if (b/'fail_restart').exists() and (b/'app/nexus').read_bytes()==b'new binary':sys.exit(1)
  shutil.copyfile(b/'app/nexus',b/'running')
 else:sys.exit('unexpected service mutation')
elif name=='curl':
 if '--write-out' in a:print('404' if (b/'missing_route').exists() else '405',end='')
 elif (b/'fail_health').exists() and (b/'running').read_bytes()==b'new binary':sys.exit(22)
 else:print('ok')
elif name=='sleep':pass
"""
        for name in ("sha256sum", "systemctl", "curl", "sleep"):
            p = self.bin / name
            p.write_text(fake)
            p.chmod(0o755)

    def tearDown(self):
        for f in ("config.env", "nexus.service", "nexus.db"):
            self.assertEqual((self.app / f).read_text(), "protected " + f)
        self.tmp.cleanup()

    def run_release(self, expected=None):
        return subprocess.run(
            ["bash", str(ROOT / "scripts/nexus_binary_release.sh"), str(self.app),
             self.release_id, self.new_sha, expected or self.old_sha, str(self.candidate)],
            env=self.env, capture_output=True, text=True, timeout=15)

    def calls(self):
        p = self.base / "calls"
        return p.read_text() if p.exists() else ""

    def test_success_retains_old_binary_and_only_restarts_nexus(self):
        p = self.run_release()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(self.backup.read_bytes(), self.old)
        self.assertEqual((self.app / "nexus").read_bytes(), self.new)
        calls = self.calls()
        self.assertEqual(calls.count("systemctl restart nexus\n"), 1)
        self.assertNotIn("POST", calls)
        self.assertNotIn("voice-jobs", calls)
        self.assertNotIn("cortex", calls)
        self.assertFalse((self.app / ".nexus-release.lock").exists())

    def test_health_failure_restores_old_binary_and_preserves_backup(self):
        (self.base / "fail_health").touch()
        p = self.run_release()
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("ROLLBACK_OK", p.stdout)
        self.assertEqual((self.app / "nexus").read_bytes(), self.old)
        self.assertEqual(self.backup.read_bytes(), self.old)
        self.assertEqual(self.calls().count("systemctl restart nexus\n"), 2)

    def test_missing_repair_route_rolls_back(self):
        (self.base / "missing_route").touch()
        p = self.run_release()
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual((self.app / "nexus").read_bytes(), self.old)

    def test_restart_failure_rolls_back(self):
        (self.base / "fail_restart").touch()
        p = self.run_release()
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("ROLLBACK_OK", p.stdout)
        self.assertEqual((self.app / "nexus").read_bytes(), self.old)

    def test_interrupted_restart_rolls_back(self):
        (self.base / "interrupt_restart").touch()
        p = self.run_release()
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("ROLLBACK_OK", p.stdout)
        self.assertEqual((self.app / "nexus").read_bytes(), self.old)
        self.assertEqual(self.backup.read_bytes(), self.old)

    def test_single_transfer_pipeline_uses_mock_ssh_once(self):
        import re
        core = (ROOT / "scripts/deploy_core.sh").read_text()
        function = re.search(r"^deploy_verified_nexus\(\) \{.*?^\}", core, re.M | re.S).group()
        release = self.base / "release"
        release.mkdir()
        (release / "nexus").write_bytes(self.new)
        env = dict(self.env, SCRIPT_DIR=str(ROOT / "scripts"), APP_DIR=str(self.app),
                   VERIFIED_NEXUS_RELEASE="fixture-only", EXPECTED_LIVE_NEXUS_SHA=self.old_sha,
                   FIXTURE_CANDIDATE=str(release / "nexus"), FIXTURE_RELEASE_ID=self.release_id,
                   FIXTURE_SHA=self.new_sha)
        code = """set -euo pipefail
python3() { printf '%s\\t%s\\t%s\\n' "$FIXTURE_CANDIDATE" "$FIXTURE_RELEASE_ID" "$FIXTURE_SHA"; }
ssh_remote() { printf 'ssh fixture call\\n' >> "$FIXTURE/ssh_calls"; bash -c "$1"; }
""" + function + "\ndeploy_verified_nexus\n"
        p = subprocess.run(["bash", "-c", code], env=env, capture_output=True, text=True, timeout=15)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual((self.base / "ssh_calls").read_text().count("ssh fixture call"), 1)
        self.assertEqual((self.app / "nexus").read_bytes(), self.new)
        self.assertEqual(self.backup.read_bytes(), self.old)

    def test_changed_baseline_is_rejected_before_replacement(self):
        p = self.run_release("b" * 64)
        self.assertNotEqual(p.returncode, 0)
        self.assertNotIn("systemctl restart", self.calls())
        self.assertFalse(self.backup.exists())
        self.assertEqual((self.app / "nexus").read_bytes(), self.old)

    def test_existing_lock_is_preserved(self):
        lock = self.app / ".nexus-release.lock"
        lock.mkdir()
        (lock / "owner").write_text("foreign operation")
        p = self.run_release()
        self.assertNotEqual(p.returncode, 0)
        self.assertEqual((lock / "owner").read_text(), "foreign operation")
        self.assertNotIn("systemctl restart", self.calls())

    def test_mixed_frontend_release_is_rejected_without_ssh(self):
        env = dict(self.env, SERVER="unused", SSH_PORT="22", APP_DIR=str(self.app),
                   DATA_DIR=str(self.app), AUDIO_DIR=str(self.app), DOMAIN="unused")
        p = subprocess.run(["bash", str(ROOT / "scripts/deploy_core.sh"),
                            "--nexus", "--frontend", "--verified-release", "absent.json"],
                           env=env, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("Verified release requires only --nexus", p.stderr)
        self.assertFalse((ROOT / ".secrets/nexus_key").exists() and "Generated" in p.stdout)

if __name__ == "__main__":
    unittest.main()
