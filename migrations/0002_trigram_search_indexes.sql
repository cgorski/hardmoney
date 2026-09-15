-- hardmoney migration 0002: trigram indexes for the API's substring searches.
--
-- Every `?q=` / `?name=` / `?employer=` filter in the REST API is
-- `col ILIKE '%term%'`. A btree index cannot serve that predicate at all
-- (verified: idx_scan = 0 on the old `*_name_idx` indexes under load), so
-- those searches were full table scans -- ~40 ms per 100k rows locally and
-- far worse on Aurora PostgreSQL 18, where parallel query is off by default.
-- A GIN trigram index serves `ILIKE '%term%'` directly (49x faster at 100k
-- rows; the gap widens with table size).
--
-- `pg_trgm` is a trusted extension on community Postgres 13+ and on Aurora
-- PostgreSQL, so a non-superuser database owner can create it. If it is
-- still unavailable (locked-down hosting), the schema remains valid and the
-- API remains correct -- only slower -- and a WARNING is raised so the
-- operator knows why.
DO $$
BEGIN
    BEGIN
        CREATE EXTENSION IF NOT EXISTS pg_trgm;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'hardmoney: pg_trgm unavailable (%), skipping trigram indexes; substring search will use sequential scans', SQLERRM;
        RETURN;
    END;

    CREATE INDEX IF NOT EXISTS schedule_a_name_trgm_idx
        ON schedule_a USING gin (name gin_trgm_ops);
    CREATE INDEX IF NOT EXISTS schedule_a_employer_trgm_idx
        ON schedule_a USING gin (employer gin_trgm_ops);
    CREATE INDEX IF NOT EXISTS disbursements_name_trgm_idx
        ON disbursements USING gin (name gin_trgm_ops);
    CREATE INDEX IF NOT EXISTS c2c_txn_name_trgm_idx
        ON committee_to_committee_transactions USING gin (name gin_trgm_ops);
    CREATE INDEX IF NOT EXISTS candidates_cand_name_trgm_idx
        ON candidates USING gin (cand_name gin_trgm_ops);
    CREATE INDEX IF NOT EXISTS committees_cmte_nm_trgm_idx
        ON committees USING gin (cmte_nm gin_trgm_ops);
END
$$;
