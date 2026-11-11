"""Check real terminal output, color opt-outs, and the piped JSON contract."""
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    if os.name != "posix":
        print("Unix output PTY smoke test skipped on this platform")
        return
    import fcntl
    import pty
    import select
    import struct
    import termios
    import time

    subprocess.run(["cargo", "build", "--locked", "--bins"], check=True)
    for name in ["simplenote", "simplenote-cli"]:
        binary = str(Path("target/debug", name).resolve())
        with tempfile.TemporaryDirectory(prefix="simplenote-output-pty-") as directory:
            environment = dict(os.environ, TERM="xterm-256color", SIMPLENOTE_CONFIG_DIR=directory)
            environment.pop("SIMPLENOTE_TOKEN", None)
            environment.pop("NO_COLOR", None)
            for width, overrides, args, expected, code, color in [
                (80, {}, ["auth", "status"], b"Not signed in", 0, True),
                (40, {"NO_COLOR": "1"}, ["auth", "status"], b"Not signed in", 0, False),
                (80, {"TERM": "dumb"}, ["auth", "status"], b"Not signed in", 0, False),
                (40, {}, ["notes", "list"], b"Sign-in required", 3, True),
                (80, {"NO_COLOR": "1"}, ["--help"], b"Usage:", 0, False),
                (80, {}, ["unknown"], b"Usage error", 2, True),
            ]:
                master, slave = pty.openpty()
                child = None
                try:
                    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 28, width, 0, 0))
                    original = termios.tcgetattr(slave)
                    child = subprocess.Popen([binary, *args], stdin=subprocess.DEVNULL,
                                             stdout=slave, stderr=slave, env=dict(environment, **overrides))
                    output = bytearray()
                    deadline = time.monotonic() + 15
                    while child.poll() is None:
                        assert time.monotonic() < deadline, "Terminal output timed out"
                        if select.select([master], [], [], .1)[0]:
                            output.extend(os.read(master, 65536))
                    while select.select([master], [], [], 0)[0]:
                        output.extend(os.read(master, 65536))
                    assert child.returncode == code
                    assert expected in output
                    assert (b"\x1b[" in output) == color, "Incorrect terminal color behavior"
                    assert termios.tcgetattr(slave) == original, "Output changed terminal attributes"
                finally:
                    os.close(master)
                    os.close(slave)
                    if child is not None and child.poll() is None:
                        child.kill()
                        child.wait(timeout=10)

            result = subprocess.run([binary, "auth", "status"], capture_output=True, env=environment, check=True)
            assert json.loads(result.stdout) == {"authenticated": False, "source": "none", "verified": False}
            assert result.stderr == b""
            result = subprocess.run([binary, "-o", "text", "auth", "status"], capture_output=True, env=environment, check=True)
            assert b"Not signed in" in result.stdout and b"\x1b" not in result.stdout
            result = subprocess.run([binary, "notes", "list"], capture_output=True, env=environment)
            assert result.returncode == 3 and result.stdout == b""
            assert json.loads(result.stderr)["error"]["kind"] == "auth"
    print("Command output: both aliases, terminal colors, NO_COLOR, TERM=dumb, errors, and piped JSON passed")


if __name__ == "__main__":
    main()
