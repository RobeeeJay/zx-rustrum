#!/usr/bin/env python3
"""One tools/call to the server mcp_server.sh started, printing the text it returns.

    mcp_call.py TOOL '{"json": "arguments"}'
    mcp_call.py --raw METHOD '{"params"}'

Images are written beside the state as img<N>.png and their path printed. Set
ZXRS_MCP_DIR if the server was started with a state directory of its own.
From Python, import call() and use it directly: it is far quicker than a process
a call when a script makes thousands of them.
"""
import json
import os
import sys
import time
import base64

DIR = os.environ.get("ZXRS_MCP_DIR", os.path.join(os.environ.get("TMPDIR", "/tmp"), "zxrs-mcp"))


def call(tool, args=None, method=None, timeout=600):
    idf = os.path.join(DIR, "id")
    n = int(open(idf).read()) + 1 if os.path.exists(idf) else 1
    open(idf, "w").write(str(n))
    msg = {"jsonrpc": "2.0", "id": n}
    if method:
        msg["method"], msg["params"] = method, args or {}
    else:
        msg["method"] = "tools/call"
        msg["params"] = {"name": tool, "arguments": args or {}}
    log = os.path.join(DIR, "out.log")
    # Read on from where the log stood: rereading it whole every call makes a
    # long session slower and slower.
    start = os.path.getsize(log)
    with open(os.path.join(DIR, "in.fifo"), "w") as f:
        f.write(json.dumps(msg) + "\n")
    began = time.time()
    with open(log) as f:
        f.seek(start)
        pending = ""
        while time.time() - began < timeout:
            chunk = f.read()
            if not chunk:
                time.sleep(0.02)
                continue
            pending += chunk
            *whole, pending = pending.split("\n")
            for line in whole:
                try:
                    reply = json.loads(line)
                except ValueError:
                    continue
                if reply.get("id") != n:
                    continue
                if "error" in reply:
                    return "ERROR " + json.dumps(reply["error"])
                result = reply["result"]
                if "content" not in result:
                    return json.dumps(result)
                out = []
                for item in result["content"]:
                    if item.get("type") == "text":
                        out.append(item["text"])
                    elif item.get("type") == "image":
                        path = os.path.join(DIR, f"img{n}.png")
                        open(path, "wb").write(base64.b64decode(item["data"]))
                        out.append(f"[image] {path}")
                if result.get("isError"):
                    out.append("(isError)")
                return "\n".join(out)
    return "TIMEOUT"


if __name__ == "__main__":
    if sys.argv[1] == "--raw":
        print(call(None, json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}, method=sys.argv[2]))
    else:
        print(call(sys.argv[1], json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}))
