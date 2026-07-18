#!/usr/bin/env python3
"""Live cross-language interop matrix.

Starts the Rust Thunder server and runs each language's client against it over a
real socket, exercising the standard HELLO handshake + PING/ECHO/typed-error.
The server is Rust-only (SPEC-004), so the matrix is one server × four clients —
the real product topology (a Rust RPC server, clients in every language).

    python interop/run.py            # build what's needed, then run
    python interop/run.py --no-build # assume artifacts are already built

Exits 0 only if every client passes.
"""

import os
import socket
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = ".exe" if os.name == "nt" else ""
RUST_BIN = os.path.join(ROOT, "rust", "target", "debug", "examples", f"interop{EXE}")
CS_DLL = os.path.join(
    ROOT, "csharp", "interop-probe", "bin", "Release", "net8.0", "interop-probe.dll"
)

# Each client: (command string, cwd). Run for role `client` against <port>.
CLIENTS = [
    ("rust", lambda port: (f'"{RUST_BIN}" client {port}', ROOT)),
    ("python", lambda port: (f'"{sys.executable}" "{os.path.join(ROOT, "interop", "probe.py")}" client {port}', ROOT)),
    ("typescript", lambda port: (f"npx tsx interop-probe.ts client {port}", os.path.join(ROOT, "typescript"))),
    ("csharp", lambda port: (f'dotnet "{CS_DLL}" client {port}', ROOT)),
]


def log(msg: str) -> None:
    print(msg, flush=True)


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def build() -> None:
    log("building the Rust server/probe and the C# client probe ...")
    subprocess.run(
        "cargo build -q -p thunder-rpc --example interop",
        cwd=os.path.join(ROOT, "rust"), shell=True, check=True,
    )
    subprocess.run(
        "dotnet build -c Release csharp/interop-probe --nologo -v q",
        cwd=ROOT, shell=True, check=True,
    )


def start_server(port: int) -> subprocess.Popen:
    proc = subprocess.Popen(
        [RUST_BIN, "server", str(port)],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, encoding="utf-8", errors="replace",
    )
    # Wait for the server to announce it is bound.
    deadline = time.time() + 30
    while time.time() < deadline:
        line = proc.stdout.readline()
        if line.strip() == "READY":
            return proc
        if proc.poll() is not None:
            raise RuntimeError(f"server exited early: {line!r}")
    raise RuntimeError("server never printed READY")


def run_client(name: str, cmd: str, cwd: str) -> tuple[bool, str]:
    result = subprocess.run(
        cmd, cwd=cwd, shell=True, capture_output=True,
        text=True, encoding="utf-8", errors="replace", timeout=120,
    )
    out = (result.stdout or "").strip().splitlines()
    verdict = out[-1] if out else ""
    ok = result.returncode == 0 and verdict == "OK"
    detail = verdict or (result.stderr or "").strip().splitlines()[-1:] or "no output"
    return ok, (verdict if ok else f"{detail} (exit {result.returncode})")


def main() -> int:
    if "--no-build" not in sys.argv:
        build()

    results: list[tuple[str, bool, str]] = []
    for name, make in CLIENTS:
        port = free_port()
        server = start_server(port)
        try:
            cmd, cwd = make(port)
            ok, detail = run_client(name, cmd, cwd)
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
        results.append((name, ok, detail))
        log(f"  rust server  <-  {name:<11} client : {'PASS' if ok else 'FAIL'}  {detail}")

    log("\ninterop matrix (rust server vs client):")
    for name, ok, detail in results:
        log(f"  {name:<11} {'PASS' if ok else 'FAIL — ' + detail}")

    passed = sum(1 for _, ok, _ in results if ok)
    log(f"\n{passed}/{len(results)} clients interoperate with the Rust server.")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
