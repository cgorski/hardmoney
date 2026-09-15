-- hardmoney migration 0001: initial bulk-data schema.
--
-- Applied by `sqlx::migrate!` (see src/db/mod.rs). Never edit an applied
-- migration; add a new numbered file instead.
--
-- Column names mirror the FEC's own bulk-data header files (fetched live
-- from https://www.fec.gov/files/bulk-downloads/data_dictionaries/, see
-- README "Sources and provenance") rather than the openFEC API's naming,
-- so a column here maps 1:1 onto the FEC's own file-description pages.
--
-- Amount/id columns that are NUMERIC/BIGINT in the FEC's own spec are kept
-- as such. Raw date strings are kept as TEXT (`*_dt`) exactly as the FEC
-- ships them, and a parsed DATE twin (`*_date`) is added alongside by the
-- loader, so a malformed date on one real-world row never fails a batch
-- and never gets silently invented -- see README "Dates".
--
-- All bulk tables are keyed by (natural key, cycle) so multiple two-year
-- election cycles can coexist in one database.

CREATE TABLE IF NOT EXISTS candidates (
    cand_id               TEXT NOT NULL,
    cycle                 INT NOT NULL,
    cand_name             TEXT,
    cand_pty_affiliation  TEXT,
    cand_election_yr      TEXT,
    cand_office_st        TEXT,
    cand_office           TEXT,
    cand_office_district  TEXT,
    cand_ici              TEXT,
    cand_status           TEXT,
    cand_pcc              TEXT,
    cand_st1              TEXT,
    cand_st2              TEXT,
    cand_city             TEXT,
    cand_st               TEXT,
    cand_zip              TEXT,
    PRIMARY KEY (cand_id, cycle)
);

CREATE TABLE IF NOT EXISTS committees (
    cmte_id               TEXT NOT NULL,
    cycle                 INT NOT NULL,
    cmte_nm               TEXT,
    tres_nm               TEXT,
    cmte_st1              TEXT,
    cmte_st2              TEXT,
    cmte_city             TEXT,
    cmte_st               TEXT,
    cmte_zip              TEXT,
    cmte_dsgn             TEXT,
    cmte_tp               TEXT,
    cmte_pty_affiliation  TEXT,
    cmte_filing_freq      TEXT,
    org_tp                TEXT,
    connected_org_nm      TEXT,
    cand_id               TEXT,
    PRIMARY KEY (cmte_id, cycle)
);
CREATE INDEX IF NOT EXISTS committees_cand_id_idx ON committees (cand_id);

CREATE TABLE IF NOT EXISTS candidate_committee_links (
    linkage_id        BIGINT NOT NULL,
    cycle             INT NOT NULL,
    cand_id           TEXT,
    cand_election_yr  TEXT,
    fec_election_yr   TEXT,
    cmte_id           TEXT,
    cmte_tp           TEXT,
    cmte_dsgn         TEXT,
    PRIMARY KEY (linkage_id, cycle)
);
CREATE INDEX IF NOT EXISTS ccl_cand_id_idx ON candidate_committee_links (cand_id);
CREATE INDEX IF NOT EXISTS ccl_cmte_id_idx ON candidate_committee_links (cmte_id);

-- Schedule A: itemized contributions to a committee (bulk source: indiv.txt).
CREATE TABLE IF NOT EXISTS schedule_a (
    sub_id            BIGINT NOT NULL,
    cycle             INT NOT NULL,
    cmte_id           TEXT,
    amndt_ind         TEXT,
    rpt_tp            TEXT,
    transaction_pgi   TEXT,
    image_num         TEXT,
    transaction_tp    TEXT,
    entity_tp         TEXT,
    name              TEXT,
    city              TEXT,
    state             TEXT,
    zip_code          TEXT,
    employer          TEXT,
    occupation        TEXT,
    transaction_dt    TEXT,
    transaction_date  DATE,
    transaction_amt   NUMERIC,
    other_id          TEXT,
    tran_id           TEXT,
    file_num          BIGINT,
    memo_cd           TEXT,
    memo_text         TEXT,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS schedule_a_cmte_id_idx ON schedule_a (cmte_id);
CREATE INDEX IF NOT EXISTS schedule_a_transaction_date_idx ON schedule_a (transaction_date DESC NULLS LAST);

-- "Any transaction from one committee to another" (bulk source: itoth.txt / oth{YY}.zip).
CREATE TABLE IF NOT EXISTS committee_to_committee_transactions (
    sub_id            BIGINT NOT NULL,
    cycle             INT NOT NULL,
    cmte_id           TEXT,
    amndt_ind         TEXT,
    rpt_tp            TEXT,
    transaction_pgi   TEXT,
    image_num         TEXT,
    transaction_tp    TEXT,
    entity_tp         TEXT,
    name              TEXT,
    city              TEXT,
    state             TEXT,
    zip_code          TEXT,
    employer          TEXT,
    occupation        TEXT,
    transaction_dt    TEXT,
    transaction_date  DATE,
    transaction_amt   NUMERIC,
    other_id          TEXT,
    tran_id           TEXT,
    file_num          BIGINT,
    memo_cd           TEXT,
    memo_text         TEXT,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS c2c_txn_cmte_id_idx ON committee_to_committee_transactions (cmte_id);
CREATE INDEX IF NOT EXISTS c2c_txn_transaction_date_idx ON committee_to_committee_transactions (transaction_date DESC NULLS LAST);

-- Contributions from committees to candidates, incl. independent
-- expenditures/coordinated & communication-cost transaction types
-- (bulk source: itpas2.txt, downloaded as pas2{YY}.zip).
CREATE TABLE IF NOT EXISTS committee_to_candidate_transactions (
    sub_id                       BIGINT NOT NULL,
    cycle                        INT NOT NULL,
    cmte_id                      TEXT,
    amndt_ind                    TEXT,
    rpt_tp                       TEXT,
    transaction_pgi              TEXT,
    image_num                    TEXT,
    transaction_tp               TEXT,
    entity_tp                    TEXT,
    name                         TEXT,
    city                         TEXT,
    state                        TEXT,
    zip_code                     TEXT,
    employer                     TEXT,
    occupation                   TEXT,
    transaction_dt               TEXT,
    transaction_date             DATE,
    transaction_amt              NUMERIC,
    other_id                     TEXT,
    cand_id                      TEXT,
    tran_id                      TEXT,
    file_num                     BIGINT,
    memo_cd                      TEXT,
    memo_text                    TEXT,
    is_independent_expenditure   BOOLEAN GENERATED ALWAYS AS (transaction_tp IN ('24A', '24E')) STORED,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS c2cand_txn_cand_id_idx ON committee_to_candidate_transactions (cand_id);
CREATE INDEX IF NOT EXISTS c2cand_txn_cmte_id_idx ON committee_to_candidate_transactions (cmte_id);
CREATE INDEX IF NOT EXISTS c2cand_txn_ie_idx ON committee_to_candidate_transactions (is_independent_expenditure) WHERE is_independent_expenditure;
CREATE INDEX IF NOT EXISTS c2cand_txn_transaction_date_idx ON committee_to_candidate_transactions (transaction_date DESC NULLS LAST);

-- Schedule B: itemized operating expenditures (bulk source: oppexp.txt).
-- NB: real oppexp.txt rows have one MORE pipe-delimited field than the
-- FEC's own 25-column header file documents (a trailing empty field) --
-- a known, publicly tracked FEC data-quality quirk (fecgov/FEC#11052), not
-- a parsing bug in this crate; the loader tolerates the extra field.
CREATE TABLE IF NOT EXISTS disbursements (
    sub_id             BIGINT NOT NULL,
    cycle              INT NOT NULL,
    cmte_id            TEXT,
    amndt_ind          TEXT,
    rpt_yr             TEXT,
    rpt_tp             TEXT,
    image_num          TEXT,
    line_num           TEXT,
    form_tp_cd         TEXT,
    sched_tp_cd        TEXT,
    name               TEXT,
    city               TEXT,
    state              TEXT,
    zip_code           TEXT,
    transaction_dt     TEXT,
    transaction_date   DATE,
    transaction_amt    NUMERIC,
    transaction_pgi    TEXT,
    purpose            TEXT,
    category           TEXT,
    category_desc      TEXT,
    memo_cd            TEXT,
    memo_text          TEXT,
    entity_tp          TEXT,
    file_num           BIGINT,
    tran_id            TEXT,
    back_ref_tran_id   TEXT,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS disbursements_cmte_id_idx ON disbursements (cmte_id);
CREATE INDEX IF NOT EXISTS disbursements_transaction_date_idx ON disbursements (transaction_date DESC NULLS LAST);

-- Shared 30-column layout for both the all-candidates summary file
-- (weball{YY}.zip) and the current House/Senate campaigns file
-- (webl{YY}.zip) -- same file description/column order, different scope
-- (all candidates vs. only current-cycle House/Senate).
CREATE TABLE IF NOT EXISTS candidate_summary (
    cand_id                 TEXT NOT NULL,
    cycle                   INT NOT NULL,
    cand_name               TEXT,
    cand_ici                TEXT,
    pty_cd                  TEXT,
    cand_pty_affiliation    TEXT,
    ttl_receipts            NUMERIC,
    trans_from_auth         NUMERIC,
    ttl_disb                NUMERIC,
    trans_to_auth           NUMERIC,
    coh_bop                 NUMERIC,
    coh_cop                 NUMERIC,
    cand_contrib            NUMERIC,
    cand_loans              NUMERIC,
    other_loans             NUMERIC,
    cand_loan_repay         NUMERIC,
    other_loan_repay        NUMERIC,
    debts_owed_by           NUMERIC,
    ttl_indiv_contrib       NUMERIC,
    cand_office_st          TEXT,
    cand_office_district    TEXT,
    spec_election           TEXT,
    prim_election           TEXT,
    run_election            TEXT,
    gen_election            TEXT,
    gen_election_percent    TEXT,
    other_pol_cmte_contrib  NUMERIC,
    pol_pty_contrib         NUMERIC,
    cvg_end_dt              TEXT,
    cvg_end_date            DATE,
    indiv_refunds           NUMERIC,
    cmte_refunds            NUMERIC,
    PRIMARY KEY (cand_id, cycle)
);

CREATE TABLE IF NOT EXISTS house_senate_summary (
    LIKE candidate_summary INCLUDING ALL
);

CREATE TABLE IF NOT EXISTS pac_party_summary (
    cmte_id                    TEXT NOT NULL,
    cycle                      INT NOT NULL,
    cmte_nm                    TEXT,
    cmte_tp                    TEXT,
    cmte_dsgn                  TEXT,
    cmte_filing_freq           TEXT,
    ttl_receipts               NUMERIC,
    trans_from_aff             NUMERIC,
    indv_contrib               NUMERIC,
    other_pol_cmte_contrib     NUMERIC,
    cand_contrib               NUMERIC,
    cand_loans                 NUMERIC,
    ttl_loans_received         NUMERIC,
    ttl_disb                   NUMERIC,
    tranf_to_aff                NUMERIC,
    indv_refunds               NUMERIC,
    other_pol_cmte_refunds     NUMERIC,
    cand_loan_repay            NUMERIC,
    loan_repay                 NUMERIC,
    coh_bop                    NUMERIC,
    coh_cop                    NUMERIC,
    debts_owed_by              NUMERIC,
    nonfed_trans_received      NUMERIC,
    contrib_to_other_cmte      NUMERIC,
    ind_exp                    NUMERIC,
    pty_coord_exp               NUMERIC,
    nonfed_share_exp           NUMERIC,
    cvg_end_dt                 TEXT,
    cvg_end_date               DATE,
    PRIMARY KEY (cmte_id, cycle)
);

-- Raw filings ingested directly via the `parser` module (not from bulk
-- CSVs): exact Schedule E rows extracted from the filing itself, not
-- derived/filtered from an aggregate bulk file.
CREATE TABLE IF NOT EXISTS filings (
    filing_id          BIGINT PRIMARY KEY,
    form_type          TEXT NOT NULL,
    fec_version        TEXT,
    committee_id       TEXT,
    is_amendment       BOOLEAN NOT NULL DEFAULT FALSE,
    amends_filing_id   BIGINT,
    header             JSONB,
    summary            JSONB,
    -- Body lines the (lenient) parser could not parse and skipped. 0 for a
    -- fully-parsed filing; never silently hidden.
    skipped_lines      INT NOT NULL DEFAULT 0,
    ingested_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS filings_committee_id_idx ON filings (committee_id);

CREATE TABLE IF NOT EXISTS schedule_e_lines (
    id                       BIGSERIAL PRIMARY KEY,
    filing_id                BIGINT NOT NULL REFERENCES filings (filing_id) ON DELETE CASCADE,
    line_index               INT NOT NULL,
    payee_name               TEXT,
    expenditure_amt          NUMERIC,
    expenditure_date         DATE,
    support_oppose_code      TEXT,
    candidate_id             TEXT,
    candidate_name           TEXT,
    candidate_office_state   TEXT,
    raw                      JSONB,
    UNIQUE (filing_id, line_index)
);
CREATE INDEX IF NOT EXISTS schedule_e_lines_candidate_id_idx ON schedule_e_lines (candidate_id);

-- One row per bulk load, for provenance and for `--if-changed` (S3 serves
-- ETag/Last-Modified for every bulk file). `row_limit IS NULL` marks a full
-- load; sampled dev loads (`--limit N`) are recorded but never "current".
CREATE TABLE IF NOT EXISTS loads (
    load_id               BIGSERIAL PRIMARY KEY,
    source                TEXT NOT NULL,
    cycle                 INT,
    loaded_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    mode                  TEXT NOT NULL,
    row_count             BIGINT NOT NULL,
    row_limit             BIGINT,
    dates_nulled          BIGINT NOT NULL DEFAULT 0,
    source_url            TEXT,
    source_etag           TEXT,
    source_last_modified  TIMESTAMPTZ,
    hardmoney_version     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS loads_source_cycle_idx ON loads (source, cycle, loaded_at DESC);
