"""Focused native-player recovery regressions. Own fixture/pipe host only; no game."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import urllib.error
import urllib.request


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def terminate_owned_host(pid, executable):
    """Hold an exact process handle and verify its image before simulated failure."""
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.QueryFullProcessImageNameW.argtypes = [wintypes.HANDLE, wintypes.DWORD, wintypes.LPWSTR, ctypes.POINTER(wintypes.DWORD)]
    kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x1000 | 0x100000 | 1, False, pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        image = ctypes.create_unicode_buffer(32768)
        length = wintypes.DWORD(len(image))
        assert kernel.QueryFullProcessImageNameW(handle, 0, image, ctypes.byref(length))
        assert Path(image.value).resolve() == executable.resolve()
        assert kernel.TerminateProcess(handle, 93)
        assert kernel.WaitForSingleObject(handle, 5000) == 0
    finally:
        kernel.CloseHandle(handle)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--server", type=Path, required=True)
    ap.add_argument("--host", type=Path, required=True)
    ap.add_argument("--ui", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--browser-test", choices=["native-recovery.mjs", "native-state-regressions.mjs", "native-concurrency.mjs", "native-addon-catalog.mjs"], default="native-recovery.mjs")
    ap.add_argument("--browser-mode", choices=["names", "addons", "selection", "packages", "busy", "draft", "switch", "copy", "restore", "all"])
    ap.add_argument("--dist", type=Path, help="Preserved built UI directory; defaults to --ui/dist")
    ap.add_argument("--case", choices=["stale", "exit", "browser", "transport", "damaged", "all"], default="all")
    args = ap.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    workspace = args.output / "workspace"
    (workspace / "profiles").mkdir(parents=True)
    (workspace / "addons/example").mkdir(parents=True)
    (workspace / "addons/example/example.lua").write_text("return 1\n", encoding="utf-8")
    document = dict(schema_version=1, name="Classic", width=1280, height=720, ui_scale=1.0,
                    profile=dict(schema_version=1, root=str(workspace.resolve()),
                                 addons=[dict(name="example", ecosystem="native", engine="lua51")]))
    if args.browser_test == "native-state-regressions.mjs":
        for name in ["constructor", "__proto__"]:
            (workspace / "addons" / name).mkdir()
            (workspace / "addons" / name / (name + ".lua")).write_text("return 1\n", encoding="utf-8")
            document["profile"]["addons"].append(dict(name=name, ecosystem="native", engine="lua51"))
    original = workspace / "profiles/Classic-0001.json"
    original.write_text(json.dumps(document), encoding="utf-8")
    original_hash = sha(original)
    if args.browser_test == "native-addon-catalog.mjs":
        neighbor = json.loads(json.dumps(document))
        neighbor["name"] = "Evening"
        neighbor["profile"]["addons"][0].update(name="Example", ecosystem="windower4")
        (workspace / "profiles/Evening-0001.json").write_text(json.dumps(neighbor), encoding="utf-8")
        disabled = json.loads(json.dumps(document))
        disabled["profile"]["addons"] = []
        (workspace / "profiles/Classic-0002.json").write_text(json.dumps(disabled), encoding="utf-8")
    flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    transcript, checks = [], []
    stderr = (args.output / "server.stderr").open("wb")
    process = subprocess.Popen([str(args.server), str(workspace.resolve()), str((args.dist or args.ui / "dist").resolve()),
                                "0", str(args.host), sha(args.host)],
                               stdout=subprocess.PIPE, stderr=stderr, text=True, creationflags=flags)
    url = None
    try:
        identity = json.loads(process.stdout.readline())
        assert identity["process_id"] == process.pid
        (args.output / "server-identity.json").write_text(json.dumps(identity, indent=2))
        url = "http://" + identity["address"]

        def request(body=None):
            path = "/api/v1/snapshot" if body is None else "/api/v1/rpc"
            req = urllib.request.Request(url + path,
                data=None if body is None else json.dumps(body).encode(),
                headers={"Origin": url, "Content-Type": "application/json", "X-Signet-Request": "1"})
            try:
                with urllib.request.urlopen(req, timeout=20) as response:
                    status, raw = response.status, response.read()
            except urllib.error.HTTPError as error:
                status, raw = error.code, error.read()
            result = json.loads(raw)
            transcript.append(dict(request=body, status=status, response=result))
            return status, result

        def rpc(action, **fields):
            status, reply = request(dict(schema_version=1, action=action, **fields))
            assert status == 200 and reply["ok"], (status, reply)
            return reply["value"]

        if args.case in ("stale", "all"):
            rpc("profile", request=dict(schema_version=1, action="save_profile", id="Classic-0001",
                name="Classic", width=1280, height=720, ui_scale=1.0, enabled=[]))
            status, reply = request(dict(schema_version=1, action="start_offline", id="Classic-0001"))
            assert status == 409 and not reply["ok"], "stale revision started an obsolete addon selection"
            assert request()[1]["session"] is None
            assert not (workspace / "player-sessions").exists(), "stale start wrote session artifacts"
            checks.append("stale startup refused before session creation")
        if args.case in ("exit", "all"):
            latest_id = request()[1]["profiles"][0]["id"]
            session = rpc("start_offline", id=latest_id)
            terminate_owned_host(session["process_id"], args.host)
            assert request()[1]["session"] is None, "snapshot advertises an exited child as active"
            checks.append("snapshot reaps exited owned host")
            rpc("start_offline", id=latest_id)
            stopped = rpc("stop_runtime")
            assert stopped["exit_code"] == 0, "shutdown lacks a confirmed normal child exit"
            assert request()[1]["session"] is None
            checks.append("normal shutdown exit code observed")
        if args.case == "damaged":
            (workspace / "profiles/Classic-0002.json").write_text("{partial", encoding="utf-8")
        if args.case in ("browser", "transport", "damaged", "all"):
            result = subprocess.run(["node", str(args.ui / "tests" / args.browser_test), url,
                                     str(args.output / "browser"), args.browser_mode or (args.case if args.case in ("transport", "damaged") else "all")], cwd=args.ui,
                                     capture_output=True, text=True, creationflags=flags, timeout=120)
            (args.output / "browser.log").write_text(result.stdout + result.stderr, encoding="utf-8")
            assert result.returncode == 0, result.stderr
            checks.append("browser draft isolation and runtime transport recovery")
        assert sha(original) == original_hash
        rpc("shutdown")
        process.wait(timeout=10)
        assert process.returncode == 0
        assert not (workspace / ".signet-player.lock").exists()
        report = dict(checks_passed=True, checks=checks, game_started=False,
                      binary_sha256={str(p): sha(p) for p in (args.server, args.host)})
        (args.output / "report.json").write_text(json.dumps(report, indent=2))
        print(json.dumps(report))
    finally:
        (args.output / "transcript.json").write_text(json.dumps(transcript, indent=2))
        if process.poll() is None:
            if url:
                try:
                    request(dict(schema_version=1, action="shutdown"))
                except (OSError, ValueError):
                    pass
            try:
                process.wait(timeout=12)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        stderr.close()


if __name__ == "__main__":
    main()