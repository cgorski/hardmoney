//! Descriptions of the FEC's pipe-delimited bulk-data files.
//!
//! Column lists here were read directly from the FEC's own data
//! dictionaries at `https://www.fec.gov/files/bulk-downloads/data_dictionaries/{name}_header_file.csv`
//! (`weball`/`webk`/`webl` don't have a published dictionary CSV, so those
//! three come from their file-description pages instead -- see the path on
//! each [`BulkSource`]) and cross-checked against real downloaded files for
//! the current cycle. See the crate README "Sources and provenance" for the
//! full citation list.
//!
//! Download and documentation URLs are built from
//! [`Endpoints::www_base`] ([`download_url_with`], [`BulkSource::doc_url`]),
//! so a mirror set with `HARDMONEY_FEC_WWW_BASE` serves every file.

use crate::Cycle;
use crate::fec::Endpoints;

/// One pipe-delimited FEC bulk file: where to download it, which table it
/// loads into, and the column order of its un-headered rows.
#[derive(Debug)]
pub struct BulkSource {
    /// Short identifying name, used on the CLI (`hardmoney bulk-load <name> ...`).
    pub name: &'static str,
    /// Destination table (created by the migrations in `migrations/`).
    pub table: &'static str,
    /// Column names in the exact order fields appear in each source row
    /// (NOT including the `cycle` column, which the loader appends).
    pub columns: &'static [&'static str],
    /// `{www_base}/files/bulk-downloads/{cycle}/{stem}{yy}.zip` stem, e.g.
    /// `"cn"` for `cn26.zip`. [`download_url_with`] fills in the 4-digit
    /// cycle for the URL path and the 2-digit year for the filename.
    pub url_stem: &'static str,
    /// The FEC page documenting this file's columns, as a path under
    /// [`Endpoints::www_base`] (`files/bulk-downloads/data_dictionaries/cn_header_file.csv`);
    /// [`BulkSource::doc_url`] makes it a URL. Cited in the README.
    pub doc_path: &'static str,
    /// If the real data has strictly more pipe-delimited fields per row
    /// than `columns.len()` (a known upstream quirk, currently only true
    /// for `oppexp` -- fecgov/FEC#11052), extra trailing fields are
    /// silently dropped rather than treated as a parse error.
    pub allow_extra_trailing_fields: bool,
    /// Raw date columns (`*_dt`, kept as TEXT exactly as shipped) that the
    /// loader also parses into a `DATE` twin column (`*_date`). The FEC
    /// ships **two** date formats across its bulk files -- `MMDDYYYY` in
    /// `indiv`/`oth`/`pas2` and `MM/DD/YYYY` in `oppexp`/`weball`/`webk`
    /// -- and the loader accepts both; anything unparseable becomes NULL
    /// and is counted in `LoadReport::dates_nulled`.
    pub date_columns: &'static [DateColumn],
}

impl BulkSource {
    /// The FEC page documenting this file's columns, under
    /// `endpoints.www_base`.
    #[must_use]
    pub fn doc_url(&self, endpoints: &Endpoints) -> String {
        endpoints.bulk_doc(self.doc_path)
    }
}

/// A raw text date column and the parsed `DATE` column derived from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateColumn {
    pub raw: &'static str,
    pub parsed: &'static str,
}

const TRANSACTION_DT: &[DateColumn] = &[DateColumn {
    raw: "transaction_dt",
    parsed: "transaction_date",
}];
const CVG_END_DT: &[DateColumn] = &[DateColumn {
    raw: "cvg_end_dt",
    parsed: "cvg_end_date",
}];
const NO_DATES: &[DateColumn] = &[];

pub const CANDIDATES: BulkSource = BulkSource {
    name: "candidates",
    table: "candidates",
    columns: &[
        "cand_id",
        "cand_name",
        "cand_pty_affiliation",
        "cand_election_yr",
        "cand_office_st",
        "cand_office",
        "cand_office_district",
        "cand_ici",
        "cand_status",
        "cand_pcc",
        "cand_st1",
        "cand_st2",
        "cand_city",
        "cand_st",
        "cand_zip",
    ],
    url_stem: "cn",
    doc_path: "files/bulk-downloads/data_dictionaries/cn_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: NO_DATES,
};

pub const COMMITTEES: BulkSource = BulkSource {
    name: "committees",
    table: "committees",
    columns: &[
        "cmte_id",
        "cmte_nm",
        "tres_nm",
        "cmte_st1",
        "cmte_st2",
        "cmte_city",
        "cmte_st",
        "cmte_zip",
        "cmte_dsgn",
        "cmte_tp",
        "cmte_pty_affiliation",
        "cmte_filing_freq",
        "org_tp",
        "connected_org_nm",
        "cand_id",
    ],
    url_stem: "cm",
    doc_path: "files/bulk-downloads/data_dictionaries/cm_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: NO_DATES,
};

pub const CANDIDATE_COMMITTEE_LINKS: BulkSource = BulkSource {
    name: "candidate_committee_links",
    table: "candidate_committee_links",
    columns: &[
        "cand_id",
        "cand_election_yr",
        "fec_election_yr",
        "cmte_id",
        "cmte_tp",
        "cmte_dsgn",
        "linkage_id",
    ],
    url_stem: "ccl",
    doc_path: "files/bulk-downloads/data_dictionaries/ccl_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: NO_DATES,
};

pub const SCHEDULE_A: BulkSource = BulkSource {
    name: "schedule_a",
    table: "schedule_a",
    columns: &[
        "cmte_id",
        "amndt_ind",
        "rpt_tp",
        "transaction_pgi",
        "image_num",
        "transaction_tp",
        "entity_tp",
        "name",
        "city",
        "state",
        "zip_code",
        "employer",
        "occupation",
        "transaction_dt",
        "transaction_amt",
        "other_id",
        "tran_id",
        "file_num",
        "memo_cd",
        "memo_text",
        "sub_id",
    ],
    url_stem: "indiv",
    doc_path: "files/bulk-downloads/data_dictionaries/indiv_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: TRANSACTION_DT,
};

pub const COMMITTEE_TO_COMMITTEE_TRANSACTIONS: BulkSource = BulkSource {
    name: "committee_to_committee_transactions",
    table: "committee_to_committee_transactions",
    columns: SCHEDULE_A.columns,
    url_stem: "oth",
    doc_path: "files/bulk-downloads/data_dictionaries/oth_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: TRANSACTION_DT,
};

pub const COMMITTEE_TO_CANDIDATE_TRANSACTIONS: BulkSource = BulkSource {
    name: "committee_to_candidate_transactions",
    table: "committee_to_candidate_transactions",
    columns: &[
        "cmte_id",
        "amndt_ind",
        "rpt_tp",
        "transaction_pgi",
        "image_num",
        "transaction_tp",
        "entity_tp",
        "name",
        "city",
        "state",
        "zip_code",
        "employer",
        "occupation",
        "transaction_dt",
        "transaction_amt",
        "other_id",
        "cand_id",
        "tran_id",
        "file_num",
        "memo_cd",
        "memo_text",
        "sub_id",
    ],
    url_stem: "pas2",
    doc_path: "files/bulk-downloads/data_dictionaries/pas2_header_file.csv",
    allow_extra_trailing_fields: false,
    date_columns: TRANSACTION_DT,
};

pub const DISBURSEMENTS: BulkSource = BulkSource {
    name: "disbursements",
    table: "disbursements",
    columns: &[
        "cmte_id",
        "amndt_ind",
        "rpt_yr",
        "rpt_tp",
        "image_num",
        "line_num",
        "form_tp_cd",
        "sched_tp_cd",
        "name",
        "city",
        "state",
        "zip_code",
        "transaction_dt",
        "transaction_amt",
        "transaction_pgi",
        "purpose",
        "category",
        "category_desc",
        "memo_cd",
        "memo_text",
        "entity_tp",
        "sub_id",
        "file_num",
        "tran_id",
        "back_ref_tran_id",
    ],
    url_stem: "oppexp",
    doc_path: "files/bulk-downloads/data_dictionaries/oppexp_header_file.csv",
    // Real oppexp.txt rows carry one extra trailing empty field beyond the
    // FEC's own 25-column header file (https://github.com/fecgov/FEC/issues/11052).
    allow_extra_trailing_fields: true,
    // NB: oppexp dates are `MM/DD/YYYY` (with slashes), unlike indiv/pas2.
    date_columns: TRANSACTION_DT,
};

pub const CANDIDATE_SUMMARY: BulkSource = BulkSource {
    name: "candidate_summary",
    table: "candidate_summary",
    columns: WEBALL_COLUMNS,
    url_stem: "weball",
    doc_path: "campaign-finance-data/all-candidates-file-description/",
    allow_extra_trailing_fields: false,
    date_columns: CVG_END_DT,
};

pub const HOUSE_SENATE_SUMMARY: BulkSource = BulkSource {
    name: "house_senate_summary",
    table: "house_senate_summary",
    columns: WEBALL_COLUMNS,
    url_stem: "webl",
    doc_path: "campaign-finance-data/current-campaigns-house-and-senate-file-description/",
    allow_extra_trailing_fields: false,
    date_columns: CVG_END_DT,
};

const WEBALL_COLUMNS: &[&str] = &[
    "cand_id",
    "cand_name",
    "cand_ici",
    "pty_cd",
    "cand_pty_affiliation",
    "ttl_receipts",
    "trans_from_auth",
    "ttl_disb",
    "trans_to_auth",
    "coh_bop",
    "coh_cop",
    "cand_contrib",
    "cand_loans",
    "other_loans",
    "cand_loan_repay",
    "other_loan_repay",
    "debts_owed_by",
    "ttl_indiv_contrib",
    "cand_office_st",
    "cand_office_district",
    "spec_election",
    "prim_election",
    "run_election",
    "gen_election",
    "gen_election_percent",
    "other_pol_cmte_contrib",
    "pol_pty_contrib",
    "cvg_end_dt",
    "indiv_refunds",
    "cmte_refunds",
];

pub const PAC_PARTY_SUMMARY: BulkSource = BulkSource {
    name: "pac_party_summary",
    table: "pac_party_summary",
    columns: &[
        "cmte_id",
        "cmte_nm",
        "cmte_tp",
        "cmte_dsgn",
        "cmte_filing_freq",
        "ttl_receipts",
        "trans_from_aff",
        "indv_contrib",
        "other_pol_cmte_contrib",
        "cand_contrib",
        "cand_loans",
        "ttl_loans_received",
        "ttl_disb",
        "tranf_to_aff",
        "indv_refunds",
        "other_pol_cmte_refunds",
        "cand_loan_repay",
        "loan_repay",
        "coh_bop",
        "coh_cop",
        "debts_owed_by",
        "nonfed_trans_received",
        "contrib_to_other_cmte",
        "ind_exp",
        "pty_coord_exp",
        "nonfed_share_exp",
        "cvg_end_dt",
    ],
    url_stem: "webk",
    doc_path: "campaign-finance-data/pac-and-party-summary-file-description/",
    allow_extra_trailing_fields: false,
    date_columns: CVG_END_DT,
};

/// Every registered [`BulkSource`], for CLI listing/validation and for
/// `hardmoney bulk-load-all`.
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

/// The production bulk-download URL for `source` at `cycle`:
/// `https://www.fec.gov/files/bulk-downloads/{cycle}/{stem}{yy}.zip`.
/// [`download_url_with`] at the default [`Endpoints`]; use that to honour
/// a `HARDMONEY_FEC_WWW_BASE` override.
#[must_use]
pub fn download_url(source: &BulkSource, cycle: Cycle) -> String {
    download_url_with(source, cycle, &Endpoints::default())
}

/// Builds the bulk-download URL for `source` at `cycle` under
/// `endpoints.www_base`, matching the FEC's own directory layout:
/// `{www_base}/files/bulk-downloads/{cycle}/{stem}{yy}.zip`
/// ([`Endpoints::bulk_zip`]).
#[must_use]
pub fn download_url_with(source: &BulkSource, cycle: Cycle, endpoints: &Endpoints) -> String {
    endpoints.bulk_zip(cycle, source.url_stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_registered_source_by_its_cli_name() {
        // If a source were ever added to `ALL` without matching its own
        // `name` field (e.g. copy-paste error), this catches it --
        // `find` is exactly what the CLI's `bulk-load <name>` argument
        // relies on to resolve user input.
        for source in ALL {
            let found = find(source.name).unwrap_or_else(|| {
                panic!(
                    "BulkSource {:?} is in ALL but find() can't locate it by name",
                    source.name
                )
            });
            assert_eq!(found.name, source.name);
        }
    }

    #[test]
    fn find_returns_none_for_an_unregistered_name() {
        assert!(find("not_a_real_source").is_none());
    }

    #[test]
    fn download_url_pads_the_two_digit_year_and_uses_the_full_cycle_in_the_path() {
        // The FEC's real layout mixes a full 4-digit cycle in the path
        // with a zero-padded 2-digit year in the filename (e.g.
        // `.../2026/cn26.zip`) -- a source cycle like 2008 has to come
        // out as `08`, not `8`, or the resulting URL 404s against the
        // FEC's real bulk-download server.
        assert_eq!(
            download_url(&CANDIDATES, Cycle::new(2026).unwrap()),
            "https://www.fec.gov/files/bulk-downloads/2026/cn26.zip"
        );
        assert_eq!(
            download_url(&CANDIDATES, Cycle::new(2008).unwrap()),
            "https://www.fec.gov/files/bulk-downloads/2008/cn08.zip"
        );
        assert_eq!(
            download_url(&DISBURSEMENTS, Cycle::new(2026).unwrap()),
            "https://www.fec.gov/files/bulk-downloads/2026/oppexp26.zip"
        );
    }

    #[test]
    fn urls_follow_a_configured_www_base() {
        let mirror = Endpoints::default()
            .with_www_base(crate::fec::Url::parse("https://mirror.example.gov/fec/").unwrap());
        assert_eq!(
            download_url_with(&CANDIDATES, Cycle::new(2026).unwrap(), &mirror),
            "https://mirror.example.gov/fec/files/bulk-downloads/2026/cn26.zip"
        );
        assert_eq!(
            CANDIDATES.doc_url(&mirror),
            "https://mirror.example.gov/fec/files/bulk-downloads/data_dictionaries/cn_header_file.csv"
        );
        assert_eq!(
            CANDIDATE_SUMMARY.doc_url(&Endpoints::default()),
            "https://www.fec.gov/campaign-finance-data/all-candidates-file-description/"
        );
        for source in ALL {
            assert!(
                !source.doc_path.starts_with('/') && !source.doc_path.contains("://"),
                "{}: doc_path must be relative to www_base",
                source.name
            );
        }
    }

    #[test]
    fn every_date_column_exists_in_its_source() {
        for source in ALL {
            for dc in source.date_columns {
                assert!(
                    source.columns.contains(&dc.raw),
                    "{}: date column {} not in columns",
                    source.name,
                    dc.raw
                );
                assert!(dc.parsed.ends_with("_date"), "{}", dc.parsed);
            }
        }
    }

    #[test]
    fn schedule_a_derived_sources_reuse_its_column_layout_exactly() {
        // `COMMITTEE_TO_COMMITTEE_TRANSACTIONS` (`oth`) shares Schedule
        // A's column layout by construction (`columns: SCHEDULE_A.columns`)
        // -- this pins that relationship so it can't silently drift if
        // `SCHEDULE_A.columns` is ever edited without noticing the alias.
        assert_eq!(
            COMMITTEE_TO_COMMITTEE_TRANSACTIONS.columns,
            SCHEDULE_A.columns
        );
    }
}
