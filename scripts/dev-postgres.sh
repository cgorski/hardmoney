#!/usr/bin/env sh
# A local Postgres for hardmoney's tests and tutorials, via docker-compose.yml
# at the repository root. Mirrors the CI service container (role, password,
# and database `hardmoney`/`hardmoney`/`hardmoney_test`), on host port 5433
# so an existing Postgres on 5432 is untouched.
#
#   scripts/dev-postgres.sh up      # start and wait until it accepts connections; prints the URL
#   scripts/dev-postgres.sh test    # `cargo test --all-features` with HARDMONEY_TEST_DATABASE_URL set
#   scripts/dev-postgres.sh psql    # a psql session in the database
#   scripts/dev-postgres.sh url     # print the URL (for `export DATABASE_URL=$(...)`)
#   scripts/dev-postgres.sh down    # stop and delete the data
#
#   PG_VERSION=15 scripts/dev-postgres.sh up   # the FEC's own Postgres major
#   PG_PORT=5444  scripts/dev-postgres.sh up   # another host port
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
port="${PG_PORT:-5433}"
url="postgres://hardmoney:hardmoney@127.0.0.1:${port}/hardmoney_test"
compose() { docker compose -f "$root/docker-compose.yml" "$@"; }

case "${1:-}" in
    up)
        compose up -d --wait
        echo "Postgres ${PG_VERSION:-18} is up. For the test suite and the tutorials:"
        echo "  export HARDMONEY_TEST_DATABASE_URL=$url"
        echo "  export DATABASE_URL=$url"
        ;;
    test)
        compose up -d --wait >/dev/null
        shift
        cd "$root"
        HARDMONEY_TEST_DATABASE_URL="$url" cargo test --all-features "$@"
        ;;
    psql)
        compose exec postgres psql -U hardmoney -d hardmoney_test
        ;;
    url)
        echo "$url"
        ;;
    down)
        compose down -v
        ;;
    *)
        sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
