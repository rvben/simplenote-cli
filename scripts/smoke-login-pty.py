"""Exercise the real login prompt in a Unix terminal, without live credentials."""
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading


def main():
    if os.name != "posix":
        print("Unix login PTY smoke test skipped on this platform")
        return

    import fcntl
    import pty
    import select
    import struct
    import termios
    import time

    subprocess.run(["cargo", "build", "--locked", "--bins"], check=True)
    binary = str(Path("target/debug/simplenote").resolve())
    secret = b"probe@example.invalid:PRIVATECODE"

    for action in ["submit", "escape", "interrupt", "eof", "clear"]:
        master, slave = pty.openpty()
        child = None
        proxy = socket.socket()
        proxy.bind(("127.0.0.1", 0))
        proxy.listen()
        proxy.settimeout(10)
        requests = []
        worker = None
        with tempfile.TemporaryDirectory(prefix="simplenote-login-pty-") as directory:
            try:
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
                original = termios.tcgetattr(slave)
                environment = {key: value for key, value in os.environ.items()
                               if key.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy", "simplenote_token"}}
                environment.update(TERM="xterm-256color", SIMPLENOTE_CONFIG_DIR=directory,
                                   HTTPS_PROXY=f"http://127.0.0.1:{proxy.getsockname()[1]}", NO_PROXY="")

                child = subprocess.Popen(
                    [binary, "--output", "json", "auth", "login"], stdin=slave,
                    stdout=subprocess.PIPE, stderr=slave, env=environment,
                )
                output = bytearray()

                def until(marker):
                    deadline = time.monotonic() + 10
                    while marker not in output:
                        if time.monotonic() > deadline:
                            raise RuntimeError(f"Login prompt timed out waiting for {marker!r}")
                        if select.select([master], [], [], .1)[0]:
                            output.extend(os.read(master, 65536))
                    return len(output)

                until(b"Paste email:CODE: ")
                assert not termios.tcgetattr(slave)[3] & termios.ECHO, "Terminal echo is enabled"
                # A bracketed paste with a trailing newline must not submit itself.
                os.write(master, b"\x1b[200~" + secret + b"\r\n\x1b[201~")
                until(b"*" * len(secret))
                assert secret not in output, "Masked input leaked"
                assert child.poll() is None, "Pasting submitted the credential"
                if action in {"submit", "escape"}:
                    os.write(master, b"\x12")  # Ctrl+R: deliberate reveal.
                    until(secret)
                    if action == "submit":
                        start = len(output)
                        os.write(master, b"\x12")
                        deadline = time.monotonic() + 5
                        while b"*" * len(secret) not in output[start:]:
                            assert time.monotonic() < deadline, "Input did not hide again"
                            if select.select([master], [], [], .1)[0]:
                                output.extend(os.read(master, 65536))
                        assert secret not in output[start:], "Hide toggle leaked input"

                        def reject_proxy():
                            connection, _ = proxy.accept()
                            with connection:
                                connection.settimeout(5)
                                request = bytearray()
                                while not request.endswith(b"\r\n\r\n"):
                                    data = connection.recv(4096)
                                    if not data:
                                        break
                                    request.extend(data)
                                requests.append(bytes(request))
                                connection.sendall(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")

                        worker = threading.Thread(target=reject_proxy)
                        worker.start()
                        # Backspace and type demonstrate editing without echoing.
                        os.write(master, b"\x7fE\r")
                        until(b"Credential received.")
                    else:
                        os.write(master, b"\x1b")
                elif action == "clear":
                    os.write(master, b"\x15\r")  # Clear, then empty Enter.
                    until(b"Expected email:CODE")
                    assert child.poll() is None, "Empty input was submitted"
                    os.write(master, b"\x1b")
                else:
                    os.write(master, b"\x03" if action == "interrupt" else b"\x04")
                if action != "submit":
                    until(b"Login cancelled.")
                child.wait(timeout=10)
                while select.select([master], [], [], 0)[0]:
                    output.extend(os.read(master, 65536))
                stdout = child.stdout.read()
                assert stdout == b"", "Prompt polluted structured stdout"
                assert child.returncode == (5 if action == "submit" else 0)
                assert not Path(directory, "auth.json").exists(), "Failed/cancelled login saved credentials"
                assert b"\x1b[6n" not in output, "Prompt queried cursor position"
                assert b"\x1b[2K\x1b[?2004l" in output, "Prompt input was not cleared on exit"
                after = termios.tcgetattr(slave)
                original[3] &= ~getattr(termios, "PENDIN", 0)
                after[3] &= ~getattr(termios, "PENDIN", 0)
                assert after == original, "Terminal attributes were not restored"
                if worker is not None:
                    worker.join(timeout=10)
                    assert not worker.is_alive()
                    assert len(requests) == 1, "Submitted request was missing or retried"
                    assert requests[0].startswith(b"CONNECT app.simplenote.com:443 HTTP/1.1")
                    assert secret not in requests[0], "Login secret escaped the TLS tunnel"
                else:
                    proxy.settimeout(.05)
                    try:
                        connection, _ = proxy.accept()
                    except socket.timeout:
                        pass
                    else:
                        connection.close()
                        raise AssertionError("Cancelled login made a network request")
            finally:
                os.close(master)
                os.close(slave)
                if child is not None and child.poll() is None:
                    child.kill()
                    child.wait(timeout=10)
                proxy.close()
                if worker is not None:
                    worker.join(timeout=11)
    print("Login prompt: masking, reveal/hide, paste, editing, clear, submit, cancellation, and terminal restoration passed")


if __name__ == "__main__":
    main()
