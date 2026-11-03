#!/bin/sh
# Keep one ZX-Rustrum MCP server running between shell commands, so the machine,
# its saved states and the observer's record survive from one call to the next.
#   mcp_server.sh start [state-dir]   start it (from the repository root)
#   mcp_server.sh stop  [state-dir]
# The server reads requests from a fifo and writes replies to out.log; call them
# with mcp_call.py. The holder keeps the fifo open, so the server does not see
# end-of-file between calls.
set -e
dir="${2:-${TMPDIR:-/tmp}/zxrs-mcp}"
case "$1" in
start)
    mkdir -p "$dir"
    [ -p "$dir/in.fifo" ] || mkfifo "$dir/in.fifo"
    : > "$dir/out.log"
    rm -f "$dir/id"
    cargo build --release --bin mcp >/dev/null 2>&1
    sleep 2147483647 > "$dir/in.fifo" &
    echo $! > "$dir/holder.pid"
    nohup ./target/release/mcp < "$dir/in.fifo" > "$dir/out.log" 2> "$dir/err.log" &
    echo $! > "$dir/server.pid"
    echo "started; state in $dir"
    ;;
stop)
    kill "$(cat "$dir/holder.pid")" 2>/dev/null || true
    echo stopped
    ;;
*)
    echo "usage: $0 start|stop [state-dir]" >&2
    exit 1
    ;;
esac
