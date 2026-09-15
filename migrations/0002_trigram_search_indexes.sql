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
-- Extension placement matters for namespaces. hardmoney runs with
-- `search_path = <namespace>,public`, and `CREATE EXTENSION` installs into
-- the *first* schema on the search path. Installing pg_trgm into a namespace
-- would (a) hide its operator classes from every other namespace and (b)
-- drop the extension along with that namespace. So it is pinned to `public`
-- (shared, never dropped), and the index statements qualify the operator
-- class with whatever schema the extension actually lives in -- which also
-- copes with a database where an operator pre-installed pg_trgm elsewhere.
--
-- `pg_trgm` is a trusted extension on community Postgres 13+ and on Aurora
-- PostgreSQL, so a non-superuser database owner can create it. If it is
-- still unavailable (locked-down hosting), the schema remains valid and the
-- API remains correct -- only slower -- and a WARNING is raised so the
-- operator knows why.
DO $$
DECLARE
    ext_schema text;
BEGIN
    BEGIN
        CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'hardmoney: pg_trgm unavailable (%), skipping trigram indexes; substring search will use sequential scans', SQLERRM;
        RETURN;
    END;

    SELECT n.nspname INTO ext_schema
    FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
    WHERE e.extname = 'pg_trgm';

    IF ext_schema IS NULL THEN
        RAISE WARNING 'hardmoney: pg_trgm not found after CREATE EXTENSION; skipping trigram indexes';
        RETURN;
    END IF;

    EXECUTE format('CREATE INDEX IF NOT EXISTS schedule_a_name_trgm_idx ON schedule_a USING gin (name %I.gin_trgm_ops)', ext_schema);
    EXECUTE format('CREATE INDEX IF NOT EXISTS schedule_a_employer_trgm_idx ON schedule_a USING gin (employer %I.gin_trgm_ops)', ext_schema);
    EXECUTE format('CREATE INDEX IF NOT EXISTS disbursements_name_trgm_idx ON disbursements USING gin (name %I.gin_trgm_ops)', ext_schema);
    EXECUTE format('CREATE INDEX IF NOT EXISTS c2c_txn_name_trgm_idx ON committee_to_committee_transactions USING gin (name %I.gin_trgm_ops)', ext_schema);
    EXECUTE format('CREATE INDEX IF NOT EXISTS candidates_cand_name_trgm_idx ON candidates USING gin (cand_name %I.gin_trgm_ops)', ext_schema);
    EXECUTE format('CREATE INDEX IF NOT EXISTS committees_cmte_nm_trgm_idx ON committees USING gin (cmte_nm %I.gin_trgm_ops)', ext_schema);
END
$$;
