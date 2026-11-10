"""Build from the sdist and verify the wheel's metadata and installed CLI."""

from email.parser import BytesParser
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def run(*arguments, **kwargs):
    return subprocess.run(arguments, check=True, **kwargs)


def main():
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
    ], cwd=root))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "simplenote-cli")
    with tempfile.TemporaryDirectory(prefix="simplenote-python-package-") as directory:
        work = Path(directory)
        dist = work / "dist"
        environment = os.environ.copy()
        environment["CARGO_TARGET_DIR"] = str(root / "target")
        arguments = [
            "uvx", "--from", "maturin>=1.8,<2", "maturin", "build", "--locked",
            "--sdist", "--out", str(dist),
        ]
        if sys.platform.startswith("linux"):
            # This is a local install test; release builds enforce manylinux 2.28.
            arguments.extend(["--compatibility", "linux"])
        run(*arguments, cwd=root, env=environment)
        wheels = list(dist.glob("*.whl"))
        sources = list(dist.glob("*.tar.gz"))
        if len(wheels) != 1 or len(sources) != 1:
            raise RuntimeError("expected one wheel and one source distribution")
        with zipfile.ZipFile(wheels[0]) as wheel:
            path = next(n for n in wheel.namelist() if n.endswith(".dist-info/METADATA"))
            info = BytesParser().parsebytes(wheel.read(path))
            if info["Name"] != "simplenote-cli" or info["Version"] != version:
                raise RuntimeError("wheel metadata differs from Cargo")
            if info.get_all("Requires-Dist"):
                raise RuntimeError("native CLI unexpectedly requires Python packages")
            executables = [name + (".exe" if os.name == "nt" else "")
                           for name in ("simplenote", "simplenote-cli")]
            for executable in executables:
                if not any(n.endswith(".data/scripts/" + executable) for n in wheel.namelist()):
                    raise RuntimeError("wheel is missing " + executable)
        with tarfile.open(sources[0]) as archive:
            names = {n.split("/", 1)[-1] for n in archive.getnames()}
            required = {"pyproject.toml", "Cargo.toml", "Cargo.lock", "src/main.rs", "src/bin/simplenote.rs",
                        "schemas/clispec-v0.3.json", "LICENSE", "README.md"}
            if not required <= names:
                raise RuntimeError("source distribution is incomplete")
        run("uvx", "--from", "twine==6.2.0", "twine", "check", "--strict",
            str(wheels[0]), str(sources[0]), cwd=root)
        venv = work / "venv"
        run("uv", "venv", "--python", sys.executable, str(venv))
        run("uv", "pip", "install", "--python", str(venv), "--no-index", str(wheels[0]))
        for executable in executables:
            binary = venv / ("Scripts" if os.name == "nt" else "bin") / executable
            run(sys.executable, str(root / "scripts/smoke-release.py"), str(binary), version)
    print(json.dumps({"version": version, "sdist_build_and_wheel_install": "passed"}))


if __name__ == "__main__":
    main()
