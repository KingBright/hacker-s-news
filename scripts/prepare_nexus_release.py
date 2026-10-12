#!/usr/bin/env python3
"""Prepare pinned Nexus source; test/build require separate explicit commands."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = Path("/Users/jinliang/rust-target")
SPEC = Path(__file__).parent / "nexus-release/path-manifests.json"

def sha(data):
    return hashlib.sha256(data).hexdigest()

def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")

def inputs(source):
    result = {}
    for p in sorted(source.rglob("*")):
        if p.is_symlink():
            raise ValueError("snapshot must not contain symlinks")
        if p.is_file():
            result[str(p.relative_to(source))] = sha(p.read_bytes())
    return result

def tree_hash(files):
    return sha(json.dumps(files, sort_keys=True).encode())

def patch_manifest(text, spec):
    if sha(text.encode()) != spec["manifest_sha256"]:
        raise ValueError("pinned path manifest changed")
    for old, new in spec["replacements"]:
        if text.count(old) != 1:
            raise ValueError("manifest replacement is not unique")
        text = text.replace(old, new, 1)
    return text

def archive(repo, revision, destination, paths=()):
    data = subprocess.check_output(
        ["git", "-C", str(repo), "archive", "--format=tar", revision, *paths])
    with tarfile.open(fileobj=io.BytesIO(data)) as tf:
        if any(m.issym() or m.islnk() for m in tf.getmembers()):
            raise ValueError("pinned archive contains links; inspect before preparing")
        tf.extractall(destination, filter="data")

def expected_sources(revision):
    recipes = json.loads(SPEC.read_text())
    if set(recipes) != {"aha", "qwen3-tts-rs"}:
        raise ValueError("unexpected path dependency")
    files = {}
    archives = [(ROOT, revision, "", ("backend", "tts"))]
    archives += [(ROOT / name, s["commit"], name + "/", ())
                 for name, s in recipes.items()]
    for repo, commit, prefix, paths in archives:
        data = subprocess.check_output(
            ["git", "-C", str(repo), "archive", "--format=tar", commit, *paths])
        with tarfile.open(fileobj=io.BytesIO(data)) as tf:
            for member in tf.getmembers():
                if member.issym() or member.islnk():
                    raise ValueError("pinned archive contains links")
                if member.isfile():
                    body = tf.extractfile(member).read()
                    if prefix and member.name == "Cargo.toml":
                        body = patch_manifest(body.decode(), recipes[prefix[:-1]]).encode()
                    files[prefix + member.name] = sha(body)
    return files, recipes

def build_slot_free():
    import fcntl
    for relative in ("debug/.cargo-lock", "x86_64-unknown-linux-musl/release/.cargo-lock"):
        p = TARGET / relative
        if p.exists():
            fd = os.open(p, os.O_RDONLY)
            try:
                query = struct.pack("qqihh", 0, 0, 0, fcntl.F_WRLCK, os.SEEK_SET)
                lock = struct.unpack("qqihh", fcntl.fcntl(fd, fcntl.F_GETLK, query))
                if lock[3] != fcntl.F_UNLCK:
                    raise ValueError("shared build slot occupied; wait for the existing queue")
            finally:
                os.close(fd)

def load(snapshot):
    snapshot = snapshot.resolve()
    m = json.loads((snapshot / "source-manifest.json").read_text())
    if m["target_dir"] != str(TARGET):
        raise ValueError("target changed")
    original, recipes = expected_sources(m["source_commit"])
    if m["files"] != original or m["path_dependencies"] != recipes:
        raise ValueError("source manifest differs from pinned Git objects")
    if m["source_tree_sha256"] != tree_hash(m["files"]):
        raise ValueError("source manifest hash changed")
    if inputs(snapshot / "source") != m["files"]:
        raise ValueError("snapshot source changed; prepare and test again")
    if sha((snapshot / "reading_article.json").read_bytes()) != m["fixture_sha256"]:
        raise ValueError("Reading regression fixture changed")
    return m

def prepare(args):
    if args.snapshot.exists():
        raise ValueError("snapshot already exists; inspect the existing manifest")
    revision = subprocess.check_output(
        ["git", "-C", str(ROOT), "rev-parse", args.source_ref + "^{commit}"],
        text=True).strip()
    artifact = json.loads(args.reading_fixture.read_text())
    if not 80_000 < len(artifact["reader_markdown"].encode()) <= 2_000_000:
        raise ValueError("use a real complete source larger than the legacy limit")
    args.snapshot.mkdir(parents=True, mode=0o700)
    source = args.snapshot / "source"
    source.mkdir()
    archive(ROOT, revision, source, ("backend", "tts"))
    dependencies = json.loads(SPEC.read_text())
    for name, spec in dependencies.items():
        destination = source / name
        destination.mkdir()
        archive(ROOT / name, spec["commit"], destination)
        p = destination / "Cargo.toml"
        p.write_text(patch_manifest(p.read_text(), spec))
    shutil.copyfile(args.reading_fixture, args.snapshot / "reading_article.json")
    os.chmod(args.snapshot / "reading_article.json", 0o600)
    files = inputs(source)
    manifest = dict(schema_version=1, source_commit=revision,
                    target_dir=str(TARGET), files=files, source_tree_sha256=tree_hash(files),
                    fixture_sha256=sha((args.snapshot / "reading_article.json").read_bytes()),
                    path_dependencies=dependencies, no_runtime_config_copied=True)
    save(args.snapshot / "source-manifest.json", manifest)
    print(json.dumps(dict(prepared=str(args.snapshot.resolve()), source_commit=revision,
                          source_tree_sha256=manifest["source_tree_sha256"],
                          cargo_started=False, deployment_started=False)))

def run_cargo(snapshot, command, logfile):
    m = load(snapshot)
    build_slot_free()
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TARGET)
    env["CARGO_NET_OFFLINE"] = "true"
    env["FRESHLOOP_READING_TEST_ARTIFACT"] = str(snapshot.resolve() / "reading_article.json")
    with logfile.open("w") as output:
        p = subprocess.Popen(command, cwd=snapshot / "source/backend", env=env,
                             stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            rc = p.wait(timeout=5400)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGTERM)
            try:
                p.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                p.wait()
            rc = 124
    load(snapshot)
    return m, rc

def test(args):
    command = ["cargo", "test", "--locked", "--offline", "-p", "nexus",
               "-j", "2", "--", "--test-threads=1"]
    log = args.snapshot / "nexus-tests.log"
    m, rc = run_cargo(args.snapshot, command, log)
    count = sum(map(int, re.findall(r"test result: ok\. (\d+) passed;", log.read_text())))
    passed = rc == 0 and count >= 54
    save(args.snapshot / "test-result.json",
         dict(source_commit=m["source_commit"], source_tree_sha256=m["source_tree_sha256"],
              command=command, exit_code=rc, full_nexus_suite=True,
              passed=passed, tests_passed=count, log_sha256=sha(log.read_bytes())))
    print(json.dumps(dict(exit_code=rc, tests_passed=count, passed=passed)))
    if not passed:
        raise SystemExit(1)

def test_proof(snapshot, m):
    t = json.loads((snapshot / "test-result.json").read_text())
    expected = ["cargo", "test", "--locked", "--offline", "-p", "nexus",
                "-j", "2", "--", "--test-threads=1"]
    if (not t.get("passed") or t.get("exit_code") != 0 or not t.get("full_nexus_suite")
            or t.get("command") != expected or t.get("tests_passed", 0) < 54
            or t["source_commit"] != m["source_commit"]
            or t["source_tree_sha256"] != m["source_tree_sha256"]
            or t["log_sha256"] != sha((snapshot / "nexus-tests.log").read_bytes())):
        raise ValueError("complete Nexus tests for this exact snapshot have not passed")
    return t

def elf_x86_64(data):
    return len(data) >= 64 and data[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", data, 18)[0] == 62

def build(args):
    m = load(args.snapshot)
    t = test_proof(args.snapshot, m)
    command = ["cargo", "build", "--locked", "--offline", "-p", "nexus",
               "--target", "x86_64-unknown-linux-musl", "--release", "-j", "2"]
    log = args.snapshot / "nexus-build.log"
    m, rc = run_cargo(args.snapshot, command, log)
    if rc:
        raise SystemExit("Nexus release build failed; no deployment evidence created")
    binary = TARGET / "x86_64-unknown-linux-musl/release/nexus"
    data = binary.read_bytes()
    if not elf_x86_64(data):
        raise ValueError("candidate is not Linux ELF64 x86_64")
    candidate = args.snapshot / "nexus"
    shutil.copyfile(binary, candidate)
    os.chmod(candidate, 0o755)
    evidence = dict(schema_version=1, source_commit=m["source_commit"],
                    source_tree_sha256=m["source_tree_sha256"], candidate_sha256=sha(data),
                    release_id=m["source_commit"] + "-" + sha(data)[:12],
                    target="x86_64-unknown-linux-musl", build_command=command,
                    build_exit_code=0, build_log_sha256=sha(log.read_bytes()),
                    test_result=t, deployment_started=False)
    save(args.snapshot / "verified-release.json", evidence)
    print(json.dumps(evidence))

def check_release(path):
    snapshot = path.resolve().parent
    m = load(snapshot)
    t = test_proof(snapshot, m)
    e = json.loads(path.read_text())
    data = (snapshot / "nexus").read_bytes()
    if (e["source_commit"] != m["source_commit"]
            or e["source_tree_sha256"] != m["source_tree_sha256"]
            or e["test_result"] != t or e["build_exit_code"] != 0
            or e["build_log_sha256"] != sha((snapshot / "nexus-build.log").read_bytes())
            or e["target"] != "x86_64-unknown-linux-musl"
            or e["build_command"] != ["cargo", "build", "--locked", "--offline", "-p", "nexus",
                                     "--target", "x86_64-unknown-linux-musl", "--release", "-j", "2"]
            or e["candidate_sha256"] != sha(data) or not elf_x86_64(data)
            or e["release_id"] != m["source_commit"] + "-" + sha(data)[:12]):
        raise ValueError("release evidence or candidate changed")
    candidate = str(snapshot / "nexus")
    if any(c in candidate for c in "\t\r\n"):
        raise ValueError("candidate path contains control characters")
    return candidate, e["release_id"], e["candidate_sha256"]

def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="action", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--source-ref", required=True)
    prep.add_argument("--snapshot", type=Path, required=True)
    prep.add_argument("--reading-fixture", type=Path, required=True)
    for name in ("test", "build", "inputs"):
        q = sub.add_parser(name)
        q.add_argument("--snapshot", type=Path, required=True)
    check = sub.add_parser("check")
    check.add_argument("--evidence", type=Path, required=True)
    args = p.parse_args()
    if args.action == "check":
        print("\t".join(check_release(args.evidence)))
    elif args.action == "inputs":
        print(json.dumps(load(args.snapshot)))
    else:
        globals()[args.action](args)

if __name__ == "__main__":
    main()
