#!/bin/sh
# Remove leftover `ghost_test_%` databases from a MySQL test server.
#
# `ghost_db::pool::connect_test` drops each test's database when the test
# ends — killing the connections still holding it first — so a run normally
# leaves nothing behind. This script is for the cases it cannot: a run
# interrupted with Ctrl-C, a crashed binary, a drop that lost its retries.
#
# Usage:
#   MYSQL_HOST=127.0.0.1 MYSQL_PORT=13306 MYSQL_USER=root MYSQL_PASS=rootpw \
#     tools/mysql-test-cleanup.sh
#
# Defaults match the server the suite's examples name
# (`mysql://root:rootpw@127.0.0.1:13306`).
set -eu

host="${MYSQL_HOST:-127.0.0.1}"
port="${MYSQL_PORT:-13306}"
user="${MYSQL_USER:-root}"
pass="${MYSQL_PASS:-rootpw}"

m() {
  MYSQL_PWD="$pass" mysql -h "$host" -P "$port" -u "$user" "$@"
}

dbs=$(m -N -e "SELECT schema_name FROM information_schema.schemata WHERE schema_name LIKE 'ghost_test_%'" 2>/dev/null || true)

if [ -z "$dbs" ]; then
  echo "no ghost_test_% databases to remove"
  exit 0
fi

for db in $dbs; do
  echo "dropping $db"
  # Kill the connections still using it (they hold metadata locks), then drop.
  kills=$(m -N -e "SELECT CONCAT('KILL ', id, ';') FROM information_schema.processlist WHERE db = '$db' AND id <> CONNECTION_ID()" 2>/dev/null || true)
  [ -n "$kills" ] && m -e "$kills" 2>/dev/null || true
  m -e "SET SESSION lock_wait_timeout = 10; DROP DATABASE IF EXISTS \`$db\`" 2>/dev/null || true
done
echo "done"
