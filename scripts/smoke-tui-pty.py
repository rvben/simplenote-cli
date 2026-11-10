"""Exercise editor handoff without a cursor-position responder (Unix)."""
import json
import os
import subprocess
import sys


def main():
    if os.name != "posix":
        print("Unix PTY smoke test skipped on this platform")
        return

    import fcntl
    import pty
    import select
    import struct
    import termios
    import time

    build = subprocess.run(
        ["cargo", "test", "--lib", "--no-run", "--message-format=json", "--locked"],
        text=True, capture_output=True, check=True,
    )
    artifacts = [json.loads(line) for line in build.stdout.splitlines() if line.startswith("{")]
    binary = next(
        a["executable"] for a in artifacts
        if a.get("reason") == "compiler-artifact" and a.get("executable")
        and "lib" in a["target"]["kind"] and a["profile"]["test"]
    )
    master, slave = pty.openpty()
    child = None
    try:
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 28, 100, 0, 0))
        original = termios.tcgetattr(slave)
        environment = dict(os.environ, TERM="xterm-256color")
        child = subprocess.Popen(
            [binary, "--exact", "tui::tests::terminal_suspend_resume_fixture", "--ignored", "--nocapture"],
            stdin=slave, stdout=slave, stderr=slave, env=environment,
        )
        output = bytearray()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if select.select([master], [], [], .1)[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    break
            if child.poll() is not None:
                # Drain the final test report written before process exit.
                while select.select([master], [], [], 0)[0]:
                    output.extend(os.read(master, 65536))
                break
        if child.poll() is None:
            raise RuntimeError("Editor handoff timed out")
        assert child.returncode == 0, output.decode(errors="replace")
        assert b"1 passed" in output and b"After" in output, output
        assert b"\x1b[6n" not in output, "Editor handoff queried cursor position"
        assert output.count(b"\x1b[?1049h") == 2, output
        assert output.count(b"\x1b[?1049l") == 2, output
        # Darwin sets PENDIN while reprocessing cooked input after raw mode.
        after = termios.tcgetattr(slave)
        original[3] &= ~getattr(termios, "PENDIN", 0)
        after[3] &= ~getattr(termios, "PENDIN", 0)
        assert after == original, "Terminal attributes not restored"
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)
    print("Editor handoff: suspend, resume, and final terminal restoration passed")


if __name__ == "__main__":
    main()
