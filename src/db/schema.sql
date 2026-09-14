-- hardmoney bulk-data schema.
--
-- Column names mirror the FEC's own bulk-data header files (fetched live
-- from https://www.fec.gov/files/bulk-downloads/data_dictionaries/, see
-- README "Sources" section) rather than the openFEC/openfec API's naming,
-- so a column here maps 1:1 onto the FEC's own file-description pages.
--
-- Amount/id columns that are NUMERIC/BIGINT in the FEC's own spec are kept
-- as such; loosely-formatted fields (raw MMDDYYYY dates, codes) are kept as
-- TEXT so a single malformed real-world row never fails an entire batch --
-- see README "Design notes" for why (this crate found more than one
-- upstream data-quality quirk this way; loud failure on one bad row would
-- make the whole ETL brittle against production government data).
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
    transaction_amt   NUMERIC,
    other_id          TEXT,
    tran_id           TEXT,
    file_num          BIGINT,
    memo_cd           TEXT,
    memo_text         TEXT,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS schedule_a_cmte_id_idx ON schedule_a (cmte_id);
CREATE INDEX IF NOT EXISTS schedule_a_name_idx ON schedule_a (name);

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
    transaction_amt   NUMERIC,
    other_id          TEXT,
    tran_id           TEXT,
    file_num          BIGINT,
    memo_cd           TEXT,
    memo_text         TEXT,
    PRIMARY KEY (sub_id)
);
CREATE INDEX IF NOT EXISTS c2c_txn_cmte_id_idx ON committee_to_committee_transactions (cmte_id);

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

-- Schedule B: itemized operating expenditures (bulk source: oppexp.txt).
-- NB: real oppexp.txt rows have one MORE pipe-delimited field than the
-- FEC's own 25-column header file documents (a trailing empty field) --
-- a known, publicly tracked FEC data-quality quirk, not a parsing bug in
-- this crate. See README "Known upstream data-quality issues" for the
-- citation and how the loader handles it.
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
CREATE INDEX IF NOT EXISTS disbursements_name_idx ON disbursements (name);

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
    PRIMARY KEY (cmte_id, cycle)
);

-- Raw filings ingested directly via the `parser` module (not from bulk
-- CSVs). This is the flagship "why we built our own parser" path: exact
-- Schedule E rows extracted from the filing itself, not derived/filtered
-- from an aggregate bulk file.
CREATE TABLE IF NOT EXISTS filings (
    filing_id          BIGINT PRIMARY KEY,
    form_type          TEXT NOT NULL,
    fec_version        TEXT,
    committee_id       TEXT,
    is_amendment       BOOLEAN NOT NULL DEFAULT FALSE,
    amends_filing_id   BIGINT,
    header             JSONB,
    summary            JSONB,
    ingested_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

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

-- If the FEC's own `fec_fitem_sched_e.dump` (a real, weekly-updated
-- pg_dump covering independent expenditures back to 1975) has been
-- restored into the `disclosure` schema -- see `hardmoney bulk load-sched-e-dump`
-- and README "The pgdump question" -- expose it under a friendly,
-- hardmoney-schema-consistent view name. This is the authoritative,
-- full-history independent-expenditures source; `committee_to_candidate_transactions`
-- (filtered to transaction types 24A/24E) and `schedule_e_lines` (from
-- directly-ingested filings) are the two complementary/approximate paths
-- when the dump hasn't been loaded.
DO $$
BEGIN
    IF to_regclass('disclosure.fec_fitem_sched_e') IS NOT NULL THEN
        -- DROP + CREATE (not CREATE OR REPLACE) because Postgres refuses to
        -- REPLACE a view when a column's output type changes (e.g. the
        -- numeric(19,0) -> bigint cast added for sub_id/file_num below), and
        -- this view has no dependents of its own to worry about losing.
        EXECUTE 'DROP VIEW IF EXISTS independent_expenditures';
        EXECUTE '
            CREATE VIEW independent_expenditures AS
            SELECT
                sub_id::bigint     AS sub_id,
                file_num::bigint   AS file_num,
                cmte_id,
                cmte_nm            AS committee_name,
                pye_nm             AS payee_name,
                s_o_cand_id        AS candidate_id,
                s_o_cand_nm        AS candidate_name,
                s_o_cand_office    AS candidate_office,
                s_o_cand_office_st AS candidate_office_state,
                s_o_ind            AS support_oppose_code,
                s_o_ind_desc       AS support_oppose_desc,
                exp_amt            AS expenditure_amt,
                exp_dt             AS expenditure_date,
                exp_desc           AS expenditure_description,
                catg_cd_desc       AS category_desc,
                election_tp,
                rpt_yr,
                election_cycle,
                filing_form,
                image_num,
                dissem_dt          AS disseminated_at
            FROM disclosure.fec_fitem_sched_e
        ';
    END IF;
END
$$;
