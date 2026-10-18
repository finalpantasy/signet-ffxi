"""Static source-contract checks only. Does not compile, import or run product code."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]


def branch(source, name):
    start = source.find('        Some("' + name + '")')
    if start < 0:
        return ""
    end = source.find("        Some(", start + 1)
    return source[start:end if end >= 0 else len(source)]


def check(main, launch, player, workflow):
    errors = []
    for command in ["launch-client", "launch-local"]:
        body = branch(main, command)
        if not body or "return Err(" not in body or any(
            value in body for value in ["fs::", "launch_interactive_client", "discovery::"]
        ):
            errors.append(command + " must remain refused before input or launch work")
    if launch.count("    runtime: &NativeLibrary,") != 4 or "runtime: &Path" in launch:
        errors.append("all startup APIs must require an explicit runtime identity")
    start = launch[launch.find("fn start_child("):]
    guard = start.find("lock_pinned_runtime(runtime)?")
    create = start.find("CreateProcessW(")
    if guard < 0 or create < 0 or guard >= create:
        errors.append("runtime verification must precede process creation")
    for required in [
        '.eq_ignore_ascii_case(&pin.sha256)',
        'validate_i686_file(&mut file, true)?',
        '.share_mode(FILE_SHARE_READ)',
        '.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)',
        'FILE_FLAG_OPEN_REPARSE_POINT',
    ]:
        if required not in launch:
            errors.append("runtime identity or stable-path guard is missing")
    if "FILE_SHARE_DELETE" in launch:
        errors.append("startup identity guards must not permit deletion")
    if '"game_launch":{"available":false' not in player:
        errors.append("Play must remain unavailable during the security hold")
    native = workflow[workflow.find("  windows-x86:"):]
    if not re.search(r"(?m)^    if: \$\{\{ false \}\}$", native):
        errors.append("native CI builds must remain paused")
    source_job = workflow[workflow.find("  source-only:"):workflow.find("  windows-x86:")]
    commands = re.findall(r"(?m)^\s+run: (.+)$", source_job)
    if commands != ["python scripts/check-source-safety.py --self-test"]:
        errors.append("source-only CI must run only this static checker")
    return errors


def main():
    texts = [
        (ROOT / path).read_text(encoding="utf-8")
        for path in ["src/main.rs", "src/launch.rs", "src/player.rs", ".github/workflows/rust.yml"]
    ]
    errors = check(*texts)
    if errors:
        raise SystemExit("Source check failed: " + "; ".join(errors))
    if "--self-test" in sys.argv[1:]:
        mutations = [
            (0, 'Some("launch-client")', 'Some("removed-client-gate")'),
            (0, 'Some("launch-local")', 'Some("removed-local-gate")'),
            (1, "lock_pinned_runtime(runtime)?", "removed_runtime_check(runtime)?"),
            (1, "runtime: &NativeLibrary,", "runtime: &Path,"),
            (1, ".eq_ignore_ascii_case(&pin.sha256)", ".eq_ignore_ascii_case(&other_digest)"),
            (2, '"game_launch":{"available":false', '"game_launch":{"available":true'),
            (3, "if: ${{ false }}", "if: ${{ true }}"),
        ]
        for index, before, after in mutations:
            changed = texts.copy()
            if before not in changed[index]:
                raise SystemExit("Static checker fixture needs review")
            changed[index] = changed[index].replace(before, after)
            if not check(*changed):
                raise SystemExit("Static checker failed to reject a source regression")
        print("PASS: seven source-contract mutation checks.")
    print("PASS: source-only checks. Native builds, unit tests and launches were not run.")


if __name__ == "__main__":
    main()
