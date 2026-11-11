#!/usr/bin/env bash
# Vuuut control script
#   ./run.sh start|stop|restart|status|build|rebuild|test|log
set -euo pipefail
cd "$(dirname "$0")"

# Private per-machine settings, never committed (see .env.example). Plain
# KEY=value lines, '#' comments, optional quotes. A variable already set in the
# shell wins over the file, so a one-off `GHOST_SERVER__PORT=2000 ./run.sh …` still works.
load_env_file() {
    local line key value
    while IFS= read -r line || [ -n "$line" ]; do
        line=${line#"${line%%[![:space:]]*}"}
        case $line in ''|'#'*) continue ;; esac
        line=${line#export }
        key=${line%%=*}; value=${line#*=}
        case $key in ''|*[!A-Za-z0-9_]*) echo "ignoring malformed line in $1: $line" >&2; continue ;; esac
        [ -n "${!key+x}" ] && continue
        case $value in \"*\") value=${value#\"}; value=${value%\"} ;; \'*\') value=${value#\'}; value=${value%\'} ;; esac
        export "$key=$value"
    done < "$1"
}
# The port, the content directory, a reference Ghost install for the parity
# tests: every GHOST_* variable is read by the server itself (GHOST_SERVER__PORT
# sets server.port, GHOST_PATHS__CONTENT_PATH sets paths.contentPath, and so
# on), so this is the one place a checkout learns about the machine it landed
# on and nothing in the tree has to name a directory.
[ -f .env ] && load_env_file .env

BIN=./target/release/ghost
# The server binds the configured port and fails if it is taken — it does not
# go looking for a free one, so this is also the port to check for an instance
# that is already up.
PORT=${GHOST_SERVER__PORT:-1971}
URL="http://localhost:$PORT"
# One pidfile per port, so two instances on one machine cannot stop each
# other. The default port keeps the unsuffixed names.
if [ "$PORT" = 1971 ]; then
    PIDFILE=/tmp/ghost-rust.pid
    LOG=/tmp/ghost-rust.log
else
    PIDFILE=/tmp/ghost-rust-$PORT.pid
    LOG=/tmp/ghost-rust-$PORT.log
fi

pid() { [ -f "$PIDFILE" ] && cat "$PIDFILE" 2>/dev/null || true; }

# Is this one of our processes? Pids are reused, and a pidfile that outlives
# its process can otherwise name something else entirely.
is_ghost() { ps -o args= -p "$1" 2>/dev/null | grep -q "target/release/ghost"; }

alive() {
    local p
    p=$(pid)
    [ -n "$p" ] && kill -0 "$p" 2>/dev/null && is_ghost "$p"
}

# Whatever is listening on the port, started by this script or not.
listener() { lsof -ti "tcp:$PORT" -sTCP:LISTEN 2>/dev/null | head -1; }

# The same, but only if it is this project's server. A start that fails
# because something else owns the port is a different problem from a stale
# instance of our own, and only one of the two is ours to kill.
ghost_listener() {
    local p
    p=$(listener)
    if [ -n "$p" ] && is_ghost "$p"; then echo "$p"; fi
}

# What is holding the port, for an error message.
describe_listener() {
    local p
    p=$(listener)
    [ -n "$p" ] && ps -o pid=,args= -p "$p" 2>/dev/null | sed 's/^ *//'
}

case "${1:-start}" in
start)
    if alive; then echo "already running (pid $(pid)) at $URL"; exit 0; fi
    rm -f "$PIDFILE"

    # An instance whose pidfile was lost still serves the site, and starting a
    # second one on the same port only produces a process that exits a moment
    # later. Adopt it instead, so stop/restart can reach it from here on.
    existing=$(ghost_listener)
    if [ -n "$existing" ]; then
        echo "$existing" >"$PIDFILE"
        echo "already running (pid $existing, adopted) at $URL"
        exit 0
    fi
    if [ -n "$(listener)" ]; then
        echo "port $PORT is held by something that is not this server:"
        echo "  $(describe_listener)"
        echo "stop it, or set GHOST_SERVER__PORT to a free port."
        exit 1
    fi

    if [ ! -x "$BIN" ]; then echo "no binary — building first…"; cargo build --release; fi
    if command -v python3 >/dev/null 2>&1; then
        # Keep the server alive when an editor/automation ends its process
        # group. Ignoring SIGHUP alone does not detach that group.
        started=$(python3 - "$BIN" "$LOG" <<'PYTHON'
import subprocess, sys
with open(sys.argv[2], "ab", buffering=0) as log:
    child = subprocess.Popen([sys.argv[1]], stdin=subprocess.DEVNULL,
                             stdout=log, stderr=subprocess.STDOUT,
                             start_new_session=True, close_fds=True)
print(child.pid)
PYTHON
        )
    else
        nohup "$BIN" </dev/null >"$LOG" 2>&1 &
        started=$!
    fi
    echo "$started" >"$PIDFILE"

    # Wait for it to serve rather than assuming it will. Most startup failures
    # — a busy port, a migration that will not apply, a bad config — happen
    # within the first second and leave nothing behind but the log.
    for _ in $(seq 1 50); do
        if ! kill -0 "$started" 2>/dev/null; then
            rm -f "$PIDFILE"
            echo "failed to start — last lines of $LOG:"
            tail -n 15 "$LOG" | sed 's/^/  /'
            exit 1
        fi
        [ "$(listener)" = "$started" ] && break
        sleep 0.1
    done

    if [ "$(listener)" = "$started" ]; then
        echo "started (pid $started) — $URL (log: $LOG)"
    else
        # Running, but nothing on the port yet. Worth saying plainly: the
        # process being up is not the same as the site being up.
        echo "started (pid $started) but nothing is listening on $PORT yet — check $LOG"
    fi
    ;;
stop)
    if alive; then
        target=$(pid)
    else
        # The pidfile is stale or gone; fall back to the port, so an instance
        # this script did not start can still be stopped.
        target=$(ghost_listener)
    fi
    rm -f "$PIDFILE"
    if [ -z "$target" ]; then
        if [ -n "$(listener)" ]; then
            echo "not running — but port $PORT is held by:"
            echo "  $(describe_listener)"
        else
            echo "not running"
        fi
        exit 0
    fi

    kill "$target" 2>/dev/null || true
    for _ in $(seq 1 50); do
        kill -0 "$target" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$target" 2>/dev/null; then
        # Five seconds of ignoring SIGTERM is not a shutdown in progress.
        kill -9 "$target" 2>/dev/null || true
        echo "stopped (pid $target, forced)"
    else
        echo "stopped (pid $target)"
    fi
    ;;
restart) "$0" stop || true; "$0" start ;;
status)
    if alive; then
        echo "running (pid $(pid)) at $URL"
    elif [ -n "$(ghost_listener)" ]; then
        echo "running (pid $(ghost_listener)) at $URL — started outside this script"
    elif [ -n "$(listener)" ]; then
        echo "not running — port $PORT is held by:"
        echo "  $(describe_listener)"
    else
        echo "not running"
    fi
    ;;
build) cargo build --release ;;
rebuild) cargo clean && cargo build --release ;;
# With .env loaded, so GHOST_REFERENCE_DIR / GSCAN_PATH / GHOST_TEST_MYSQL_URL
# reach the tests without being typed each time.
test) shift; cargo test --workspace "$@" ;;
log) tail -f "$LOG" ;;
*) echo "usage: $0 start|stop|restart|status|build|rebuild|test|log   (settings: .env, see .env.example)" ;;
esac
