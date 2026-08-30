#!/usr/bin/env python3
# Probe the oxibrain daemon's serve path over its Unix-domain socket.
#
# Sends one newline-delimited JSON-RPC frame ({"jsonrpc":"2.0","id":1,
# "method":"ping"}) and asserts the daemon answers with a non-empty
# JSON-RPC result envelope. The daemon's serve_socket path is
# unauthenticated newline JSON-RPC (oxibrain-mcp server.rs:408 handles
# "ping" => success(id, {})); repo tests drive exactly this exchange.
#
# Env:
#   SOCK             path to the daemon's Unix-domain socket
#   DAEMON_PROBE_OUT file to write the result to ("ok" or "FAIL: <reason>")
#
# A bare connect()+close() would only prove bind/accept — a wedged session
# loop would pass. This probe proves the dispatch path actually runs.

import json
import os
import socket
import sys

path = os.environ["SOCK"]
out = os.environ.get("DAEMON_PROBE_OUT", "")
result = "ok"

try:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(5)
    s.connect(path)
    s.sendall(b'{"jsonrpc":"2.0","id":1,"method":"ping"}\n')
    s.shutdown(socket.SHUT_WR)

    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(4096)
        if not chunk:
            break
        buf += chunk
    s.close()

    if not buf.strip():
        raise RuntimeError("daemon closed without answering the ping")
    resp = json.loads(buf.split(b"\n", 1)[0])
    if resp.get("id") != 1 or "result" not in resp:
        raise RuntimeError(f"unexpected ping reply: {buf!r}")
except Exception as e:
    result = f"FAIL: {e}"

if out:
    with open(out, "w") as f:
        f.write(result + "\n")
else:
    sys.stdout.write(result + "\n")
sys.exit(0)
