#!/usr/bin/env bash
# Ghost-rust control script
#   ./run.sh start|stop|restart|status|build|rebuild|log
set -euo pipefail
cd "$(dirname "$0")"

BIN=./target/release/ghost
PIDFILE=/tmp/ghost-rust.pid
LOG=/tmp/ghost-rust.log

pid() { [ -f "$PIDFILE" ] && cat "$PIDFILE" 2>/dev/null || true; }

alive() {
    local p
    p=$(pid)
    [ -n "$p" ] && kill -0 "$p" 2>/dev/null
}

case "${1:-start}" in
start)
    if alive; then echo "already running (pid $(pid)) at http://localhost:2368"; exit 0; fi
    if [ ! -x "$BIN" ]; then echo "no binary — building first…"; cargo build --release; fi
    nohup "$BIN" >"$LOG" 2>&1 &
    echo $! >"$PIDFILE"
    sleep 1
    echo "started (pid $(pid)) — http://localhost:2368 (log: $LOG)"
    ;;
stop)
    if alive; then kill "$(pid)" && rm -f "$PIDFILE" && echo "stopped"; else echo "not running"; fi
    ;;
restart) "$0" stop || true; "$0" start ;;
status)
    if alive; then echo "running (pid $(pid)) at http://localhost:2368"; else echo "not running"; fi
    ;;
build) cargo build --release ;;
rebuild) cargo clean && cargo build --release ;;
log) tail -f "$LOG" ;;
*) echo "usage: $0 start|stop|restart|status|build|rebuild|log" ;;
esac
