-- hardmoney migration 0003: amendment-chain resolution for ingested filings.
--
-- Committees re-file reports as amendments. An amendment's cover form type
-- ends in `A` (`F3XA`) and its HDR `report_id` is `FEC-<n>` where `n` is
-- the ORIGINAL filing's number (never a prior amendment's); `report_number`
-- is the sequential amendment number. `filings` already stores
-- `is_amendment` and `amends_filing_id`; this migration adds the columns
-- the resolver (`hardmoney::db::resolve_amendment_chain`) derives from
-- them, using openFEC's semantics so the API can expose openFEC's names:
--
--   amendment_version      position in the chain (original = 0)
--   amendment_chain        every filing id in the chain up to and
--                          including this one, ordered by version
--   most_recent            true on the highest-version member only
--   most_recent_filing_id  that member's id (openFEC: most_recent_file_number)
--   previous_filing_id     the immediately preceding member; the original
--                          points at itself (openFEC: previous_file_number)
--   chain_unresolved       true when an amendment's header lacks a usable
--                          `FEC-<n>` or names a filing that is not in this
--                          table: it then stands alone (chain = [self],
--                          most_recent) and this flag makes that visible
--
-- The four descriptive columns (`report_id`, `report_code`, `coverage_from`,
-- `coverage_through`) and `amendment_number` are written by the ingester
-- straight from the filing's header/cover line; the rest are NULL until the
-- resolver has run (`hardmoney bulk-load-filing` runs it after every
-- ingest; `resolve_all_amendment_chains` recomputes the whole table).
--
-- Rows ingested before this migration have NULLs in every new column until
-- they are re-ingested or `resolve_all_amendment_chains` is run.

ALTER TABLE filings
    ADD COLUMN IF NOT EXISTS report_id              TEXT,
    ADD COLUMN IF NOT EXISTS report_code            TEXT,
    ADD COLUMN IF NOT EXISTS coverage_from          DATE,
    ADD COLUMN IF NOT EXISTS coverage_through       DATE,
    ADD COLUMN IF NOT EXISTS amendment_number       INT,
    ADD COLUMN IF NOT EXISTS amendment_version      INT,
    ADD COLUMN IF NOT EXISTS amendment_chain        BIGINT[],
    ADD COLUMN IF NOT EXISTS most_recent            BOOLEAN,
    ADD COLUMN IF NOT EXISTS most_recent_filing_id  BIGINT,
    ADD COLUMN IF NOT EXISTS previous_filing_id     BIGINT,
    ADD COLUMN IF NOT EXISTS chain_unresolved       BOOLEAN NOT NULL DEFAULT FALSE;

-- The resolver's scope query (`filing_id = $1 OR amends_filing_id = $1`)
-- and the list route's `?committee_id=&most_recent=true` filter.
CREATE INDEX IF NOT EXISTS filings_amends_filing_id_idx ON filings (amends_filing_id);
CREATE INDEX IF NOT EXISTS filings_committee_most_recent_idx ON filings (committee_id, most_recent);

-- The latest version of every report: one row per chain. Unqualified, so
-- it binds to this namespace's `filings` (see src/db/mod.rs "Namespaces").
CREATE OR REPLACE VIEW filings_current AS
    SELECT * FROM filings WHERE most_recent;
