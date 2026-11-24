"""Static source-contract checks only. Does not compile, import or run product code."""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]

# During the hold, admit only this reviewed active workflow prefix. Comparing
# the complete block also covers job defaults, shells, environment and actions;
# checking just run: lines would miss other ways to execute code in CI.
SOURCE_WORKFLOW_PREFIX = """name: Signet source checks (native builds paused)
on:
  push:
  pull_request:
  workflow_dispatch:
permissions:
  contents: read
jobs:
  source-only:
    runs-on: windows-latest
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 # v5
        with:
          persist-credentials: false
      - name: Source-only launch policy checks
        run: python scripts/check-source-safety.py --self-test"""


def workflow_lines(source):
    return [
        line.rstrip()
        for line in source.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


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
    lines = workflow_lines(workflow)
    headings = [line for line in lines if re.match(r"^  \S", line)]
    # The active prefix below also pins which top-level YAML keys are allowed.
    # Reject additional or quoted job headings rather than guessing their keys.
    if headings != ["  push:", "  pull_request:", "  workflow_dispatch:",
                    "  contents: read", "  source-only:", "  windows-x86:"]:
        errors.append("CI jobs and top-level settings must match the security hold allowlist")
    native_index = lines.index("  windows-x86:") if "  windows-x86:" in lines else len(lines)
    if lines[:native_index] != workflow_lines(SOURCE_WORKFLOW_PREFIX):
        errors.append("active CI settings, actions and steps must match the reviewed source-only block")
    native = lines[native_index:]
    conditions = [line for line in native if line.startswith("    if:")]
    if conditions != ["    if: ${{ false }}"]:
        errors.append("native CI builds must remain paused")
    if any(re.match(r"^\S", line) for line in native):
        errors.append("workflow-wide settings after the native job are not admitted during the hold")
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
            (3, "  windows-x86:", "  extra-build:\n    runs-on: windows-latest\n    steps:\n      - run: cargo build\n  windows-x86:"),
            (3, "  windows-x86:", "  renamed-native:"),
            (3, "    if: ${{ false }}", "    if: ${{ false }}\n    if: ${{ true }}"),
            (3, "      - name: Source-only launch policy checks", "      - uses: unreviewed/action@main\n      - name: Source-only launch policy checks"),
            (3, "        run: python scripts/check-source-safety.py --self-test", "        shell: unreviewed-shell {0}\n        run: python scripts/check-source-safety.py --self-test"),
            (3, "fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09", "main"),
            (3, "permissions:\n  contents: read", "permissions:\n  contents: write"),
            (3, "jobs:", "env:\n  PYTHONPATH: unreviewed-code\njobs:"),
            (3, "          exit $LASTEXITCODE", "          exit $LASTEXITCODE\n  extra-build:\n    runs-on: windows-latest\n    steps:\n      - run: cargo build"),
            (3, "          exit $LASTEXITCODE", "          exit $LASTEXITCODE\nenv:\n  PYTHONPATH: unreviewed-code"),
        ]
        for index, before, after in mutations:
            changed = texts.copy()
            if before not in changed[index]:
                raise SystemExit("Static checker fixture needs review")
            changed[index] = changed[index].replace(before, after)
            if not check(*changed):
                raise SystemExit("Static checker failed to reject a source regression")
        print(f"PASS: {len(mutations)} source-contract mutation checks.")
    print("PASS: source-only checks. Native builds, unit tests and launches were not run.")


if __name__ == "__main__":
    main()
