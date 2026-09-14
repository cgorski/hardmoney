//! Descriptions of the FEC's pipe-delimited bulk-data files.
//!
//! Column lists here were read directly from the FEC's own data
//! dictionaries at `https://www.fec.gov/files/bulk-downloads/data_dictionaries/{name}_header_file.csv`
//! (`weball`/`webk`/`webl` don't have a published dictionary CSV, so those
//! three come from their file-description pages instead -- see the URL on
//! each [`BulkSource`]) and cross-checked against real downloaded files for
//! the current cycle. See the crate README "Sources" table for the full
//! citation list.

/// One pipe-delimited FEC bulk file: where to download it, which table it
/// loads into, and the column order of its un-headered rows.
pub struct BulkSource {
    /// Short identifying name, used on the CLI (`hardmoney bulk load <name> ...`).
    pub name: &'static str,
    /// Destination table (must already exist per `src/db/schema.sql`).
    pub table: &'static str,
    /// Column names in the exact order fields appear in each source row
    /// (NOT including the `cycle` column, which the loader appends).
    pub columns: &'static [&'static str],
    /// `https://www.fec.gov/files/bulk-downloads/{{cycle}}/{{stem}}{{yy}}.zip`
    /// stem, e.g. `"cn"` for `cn26.zip`. The loader fills in the 4-digit
    /// cycle for the URL path and the 2-digit year for the filename.
    pub url_stem: &'static str,
    /// The FEC page documenting this file's columns, cited in the README.
    pub doc_url: &'static str,
    /// If the real data has strictly more pipe-delimited fields per row
    /// than `columns.len()` (a known upstream quirk, currently only true
    /// for `oppexp` -- see README "Known upstream data-quality issues"),
    /// extra trailing fields are silently dropped rather than treated as
    /// a parse error.
    pub allow_extra_trailing_fields: bool,
}

pub const CANDIDATES: BulkSource = BulkSource {
    name: "candidates",
    table: "candidates",
    columns: &[
        "cand_id", "cand_name", "cand_pty_affiliation", "cand_election_yr", "cand_office_st",
        "cand_office", "cand_office_district", "cand_ici", "cand_status", "cand_pcc", "cand_st1",
        "cand_st2", "cand_city", "cand_st", "cand_zip",
    ],
    url_stem: "cn",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/cn_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const COMMITTEES: BulkSource = BulkSource {
    name: "committees",
    table: "committees",
    columns: &[
        "cmte_id", "cmte_nm", "tres_nm", "cmte_st1", "cmte_st2", "cmte_city", "cmte_st",
        "cmte_zip", "cmte_dsgn", "cmte_tp", "cmte_pty_affiliation", "cmte_filing_freq", "org_tp",
        "connected_org_nm", "cand_id",
    ],
    url_stem: "cm",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/cm_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const CANDIDATE_COMMITTEE_LINKS: BulkSource = BulkSource {
    name: "candidate_committee_links",
    table: "candidate_committee_links",
    columns: &["cand_id", "cand_election_yr", "fec_election_yr", "cmte_id", "cmte_tp", "cmte_dsgn", "linkage_id"],
    url_stem: "ccl",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/ccl_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const SCHEDULE_A: BulkSource = BulkSource {
    name: "schedule_a",
    table: "schedule_a",
    columns: &[
        "cmte_id", "amndt_ind", "rpt_tp", "transaction_pgi", "image_num", "transaction_tp",
        "entity_tp", "name", "city", "state", "zip_code", "employer", "occupation",
        "transaction_dt", "transaction_amt", "other_id", "tran_id", "file_num", "memo_cd",
        "memo_text", "sub_id",
    ],
    url_stem: "indiv",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/indiv_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const COMMITTEE_TO_COMMITTEE_TRANSACTIONS: BulkSource = BulkSource {
    name: "committee_to_committee_transactions",
    table: "committee_to_committee_transactions",
    columns: SCHEDULE_A.columns,
    url_stem: "oth",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/oth_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const COMMITTEE_TO_CANDIDATE_TRANSACTIONS: BulkSource = BulkSource {
    name: "committee_to_candidate_transactions",
    table: "committee_to_candidate_transactions",
    columns: &[
        "cmte_id", "amndt_ind", "rpt_tp", "transaction_pgi", "image_num", "transaction_tp",
        "entity_tp", "name", "city", "state", "zip_code", "employer", "occupation",
        "transaction_dt", "transaction_amt", "other_id", "cand_id", "tran_id", "file_num",
        "memo_cd", "memo_text", "sub_id",
    ],
    url_stem: "pas2",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/pas2_header_file.csv",
    allow_extra_trailing_fields: false,
};

pub const DISBURSEMENTS: BulkSource = BulkSource {
    name: "disbursements",
    table: "disbursements",
    columns: &[
        "cmte_id", "amndt_ind", "rpt_yr", "rpt_tp", "image_num", "line_num", "form_tp_cd",
        "sched_tp_cd", "name", "city", "state", "zip_code", "transaction_dt", "transaction_amt",
        "transaction_pgi", "purpose", "category", "category_desc", "memo_cd", "memo_text",
        "entity_tp", "sub_id", "file_num", "tran_id", "back_ref_tran_id",
    ],
    url_stem: "oppexp",
    doc_url: "https://www.fec.gov/files/bulk-downloads/data_dictionaries/oppexp_header_file.csv",
    // Real oppexp.txt rows carry one extra trailing empty field beyond the
    // FEC's own 25-column header file -- see README "Known upstream
    // data-quality issues" (cites https://github.com/fecgov/FEC/issues/11052).
    allow_extra_trailing_fields: true,
};

pub const CANDIDATE_SUMMARY: BulkSource = BulkSource {
    name: "candidate_summary",
    table: "candidate_summary",
    columns: WEBALL_COLUMNS,
    url_stem: "weball",
    doc_url: "https://www.fec.gov/campaign-finance-data/all-candidates-file-description/",
    allow_extra_trailing_fields: false,
};

pub const HOUSE_SENATE_SUMMARY: BulkSource = BulkSource {
    name: "house_senate_summary",
    table: "house_senate_summary",
    columns: WEBALL_COLUMNS,
    url_stem: "webl",
    doc_url: "https://www.fec.gov/campaign-finance-data/current-campaigns-house-and-senate-file-description/",
    allow_extra_trailing_fields: false,
};

const WEBALL_COLUMNS: &[&str] = &[
    "cand_id", "cand_name", "cand_ici", "pty_cd", "cand_pty_affiliation", "ttl_receipts",
    "trans_from_auth", "ttl_disb", "trans_to_auth", "coh_bop", "coh_cop", "cand_contrib",
    "cand_loans", "other_loans", "cand_loan_repay", "other_loan_repay", "debts_owed_by",
    "ttl_indiv_contrib", "cand_office_st", "cand_office_district", "spec_election",
    "prim_election", "run_election", "gen_election", "gen_election_percent",
    "other_pol_cmte_contrib", "pol_pty_contrib", "cvg_end_dt", "indiv_refunds", "cmte_refunds",
];

pub const PAC_PARTY_SUMMARY: BulkSource = BulkSource {
    name: "pac_party_summary",
    table: "pac_party_summary",
    columns: &[
        "cmte_id", "cmte_nm", "cmte_tp", "cmte_dsgn", "cmte_filing_freq", "ttl_receipts",
        "trans_from_aff", "indv_contrib", "other_pol_cmte_contrib", "cand_contrib", "cand_loans",
        "ttl_loans_received", "ttl_disb", "tranf_to_aff", "indv_refunds", "other_pol_cmte_refunds",
        "cand_loan_repay", "loan_repay", "coh_bop", "coh_cop", "debts_owed_by",
        "nonfed_trans_received", "contrib_to_other_cmte", "ind_exp", "pty_coord_exp",
        "nonfed_share_exp", "cvg_end_dt",
    ],
    url_stem: "webk",
    doc_url: "https://www.fec.gov/campaign-finance-data/pac-and-party-summary-file-description/",
    allow_extra_trailing_fields: false,
};

/// Every registered [`BulkSource`], for CLI listing/validation and for
/// `hardmoney bulk load-all`.
pub const ALL: &[&BulkSource] = &[
    &CANDIDATES,
    &COMMITTEES,
    &CANDIDATE_COMMITTEE_LINKS,
    &SCHEDULE_A,
    &COMMITTEE_TO_COMMITTEE_TRANSACTIONS,
    &COMMITTEE_TO_CANDIDATE_TRANSACTIONS,
    &DISBURSEMENTS,
    &CANDIDATE_SUMMARY,
    &HOUSE_SENATE_SUMMARY,
    &PAC_PARTY_SUMMARY,
];

/// Looks up a registered source by its CLI `name`.
pub fn find(name: &str) -> Option<&'static BulkSource> {
    ALL.iter().copied().find(|s| s.name == name)
}

/// Builds the bulk-download URL for `source` at the given 4-digit `cycle`
/// (e.g. `2026`), matching the FEC's own directory layout:
/// `https://www.fec.gov/files/bulk-downloads/{cycle}/{stem}{yy}.zip`.
pub fn download_url(source: &BulkSource, cycle: u16) -> String {
    let yy = cycle % 100;
    format!(
        "https://www.fec.gov/files/bulk-downloads/{cycle}/{stem}{yy:02}.zip",
        cycle = cycle,
        stem = source.url_stem,
        yy = yy
    )
}
