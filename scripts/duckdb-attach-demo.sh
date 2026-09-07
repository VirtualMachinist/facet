#!/usr/bin/env bash
# External DuckDB ATTACH of a Lattice workspace store.
# Primary analytics path (no Rust, no cargo feature). Apiary-only smoke.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DUCKDB="${DUCKDB:-}"
if [[ -z "$DUCKDB" ]]; then
  if command -v duckdb >/dev/null 2>&1; then
    DUCKDB="$(command -v duckdb)"
  elif [[ -x "$HOME/bin/duckdb" ]]; then
    DUCKDB="$HOME/bin/duckdb"
  else
    echo "error: duckdb CLI not found (set DUCKDB= or put it on PATH / ~/bin/duckdb)" >&2
    exit 1
  fi
fi
command -v sqlite3 >/dev/null 2>&1 || {
  echo "error: sqlite3 CLI required to seed the store" >&2
  exit 1
}

WORKDIR="${TMPDIR:-/tmp}/facet-duckdb-attach-$$"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT
mkdir -p "$WORKDIR/.facet"
DB="$WORKDIR/.facet/lattice.db"

sqlite3 "$DB" <"$ROOT/crates/lattice/migrations/workspace/0001_init.sql"
sqlite3 "$DB" <"$ROOT/crates/lattice/migrations/workspace/0002_hash_only_req.sql"
sqlite3 "$DB" <<'SQL'
INSERT INTO runs (id, started_at, duration_ms, request_path, request_hash, method, url, status, actor, res_body_len) VALUES
 ('01JA1',1757000001000,128,'users/list.yml','h1','GET','http://api/users',200,'agent-a',512),
 ('01JA2',1757000002000,407,'users/list.yml','h1','GET','http://api/users',200,'agent-a',512),
 ('01JA3',1757000003000,503,'items/0','h2','POST','http://api/items',500,'human',2048);
INSERT INTO blobs (hash, len, content_type, created_at) VALUES ('abc',512,'application/json',1757000001000);
SQL

HISTOGRAM="$("$DUCKDB" -csv -c "INSTALL sqlite; LOAD sqlite; ATTACH '$DB' AS lattice (TYPE SQLITE);
SELECT status, count(*) AS n, round(avg(duration_ms),1) AS avg_ms FROM lattice.runs GROUP BY status ORDER BY status;")"
echo "$HISTOGRAM"
echo "$HISTOGRAM" | grep -q '^200,2,267.5$' || {
  echo "error: expected status 200 → 2 rows, avg 267.5 ms" >&2
  echo "$HISTOGRAM" >&2
  exit 1
}
echo "$HISTOGRAM" | grep -q '^500,1,503.0$' || {
  echo "error: expected status 500 → 1 row, avg 503.0 ms" >&2
  echo "$HISTOGRAM" >&2
  exit 1
}

echo "ok: duckdb ATTACH analytics over $DB"
