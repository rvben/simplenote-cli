"""Check an installed distribution without credentials or network access."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary = str(Path(sys.argv[1]).resolve())
    version = sys.argv[2]
    name = Path(binary).stem
    with tempfile.TemporaryDirectory(prefix="simplenote-package-smoke-") as directory:
        environment = os.environ.copy()
        environment.pop("SIMPLENOTE_TOKEN", None)
        environment["SIMPLENOTE_CONFIG_DIR"] = directory

        def run(*arguments, exit_code=0):
            result = subprocess.run(
                [binary, *arguments], env=environment, stdin=subprocess.DEVNULL,
                capture_output=True, text=True, timeout=20,
            )
            if result.returncode != exit_code:
                raise RuntimeError("installed CLI returned an unexpected exit code")
            if exit_code:
                if result.stdout:
                    raise RuntimeError("failed command polluted stdout")
                return result.stderr
            if result.stderr:
                raise RuntimeError("successful command polluted stderr")
            return result.stdout

        if run("--version").strip() != name + " " + version:
            raise RuntimeError("installed CLI version differs from package metadata")
        if "Usage: " + name + " " not in run("--help"):
            raise RuntimeError("installed CLI help uses the wrong command name")
        if not any(line.lstrip().startswith("complete ") and line.endswith(" " + name)
                   for line in run("completions", "bash").splitlines()):
            raise RuntimeError("installed CLI completions use the wrong command name")
        schema = json.loads(run("schema"))
        if schema["clispec"] != "0.3":
            raise RuntimeError("installed CLI discovery contract changed")
        commands = {command["name"] for command in schema["commands"]}
        if not {"tui", "notes edit"} <= commands:
            raise RuntimeError("installed CLI is missing interactive commands")
        frame = run("tui", "--demo", "--snapshot", "--width", "60", "--height", "24")
        if len(frame.splitlines()) != 24 or "simplenote" not in frame or "q quit" not in frame or "\x1b" in frame:
            raise RuntimeError("installed TUI demo did not produce a clean sample frame")
        error = json.loads(run("-o", "json", "tui", exit_code=12))
        if error["error"]["kind"] != "tty_required":
            raise RuntimeError("installed TUI did not refuse noninteractive use")
        status = json.loads(run("-o", "json", "auth", "status"))
        if status["authenticated"] is not False:
            raise RuntimeError("smoke test borrowed credentials")
        error = json.loads(run("-o", "json", "notes", "list", exit_code=3))
        if error["error"]["kind"] != "auth":
            raise RuntimeError("missing credentials did not produce an auth error")
    print(json.dumps({"version": version, "command": name, "installed_package_smoke": "passed"}))


if __name__ == "__main__":
    main()
