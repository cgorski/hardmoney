//! Regenerates the golden fixture pack in `tests/fixtures/golden/`.
//!
//! The pack is a set of spec-8.5 `.fec` files built from `ParsedLine`s and
//! `Filing::from_parts`, with every cover page filled so that each Column
//! A line balances against its schedules and formulas (the fixed point of
//! `Filing::reconcile`), plus one sidecar per fixture with the expected
//! `hardmoney validate --json` and `hardmoney reconcile --json --all`
//! output, an `expected_column_a.json` for the Form 3X keyed the way
//! FECfile+'s `calculate_summary_column_a` keys its result, and a
//! `MANIFEST.json` with a SHA-256 per file.
//!
//! Everything is a pure function of the constants in this file (the
//! "seed", which also fills the `HDR` software name and version), not of
//! the commit or the crate version, so `tests/golden_fixtures.rs` can
//! regenerate the pack into memory and byte-compare it with the committed
//! files.
//!
//! Run with:
//!
//!     cargo run --example golden_fixtures -- [OUT_DIR]
//!
//! (defaults to `tests/fixtures/golden`). Requires the `serde` feature.

#[cfg(feature = "serde")]
pub mod pack {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::path::Path;

    use hardmoney::parser::reconcile::{Column, Source, rules_for};
    use hardmoney::parser::{Header, ParseOptions, Reconciliation, Validation};
    use hardmoney::{Filing, ParsedLine, SpecVersion, Table};
    use rust_decimal::Decimal;
    use serde_json::{Map, Value, json};

    type BoxError = Box<dyn Error + Send + Sync>;
    type Result<T> = std::result::Result<T, BoxError>;

    /// Bumped when a fixture's content changes on purpose.
    pub const PACK_VERSION: &str = "1";
    /// `HDR` software name; with [`PACK_VERSION`] this is the seed.
    pub const SOFT_NAME: &str = "hardmoney-golden";
    /// Where the pack lives in the repository.
    pub const DEFAULT_DIR: &str = "tests/fixtures/golden";

    /// Committee ids in the pack, by role. The FEC's WebCheck checks every
    /// committee id in a filing (filer, donor, beneficiary) against its
    /// committee registry, so a fictitious id such as `C00123456` fails
    /// it (`ID# NOT Correct FEC ID# Format`). These ids belong to
    /// committees that appear in the FEC's 1990 committee master and not
    /// in the 2024 one; they are used only so the files pass WebCheck.
    /// Nothing else in the pack (names, addresses, amounts, dates) relates
    /// to those committees, and the fixtures' committee names are
    /// invented on purpose so no file reads as a real filing.
    pub const COMMITTEE_IDS: &[(&str, &str, &str)] = &[
        (
            "pac",
            PAC,
            "NATIONAL COUNCIL OF SAVINGS INSTITUTIONS (THRIFTPAC)",
        ),
        (
            "affiliated_pac",
            AFFILIATED_PAC,
            "CALIFORNIA LEAGUE OF SAVINGS INSTITUTIONS FEDPAC",
        ),
        (
            "other_pac",
            OTHER_PAC,
            "CONNECTICUT MEDICAL POLITICAL ACTION COMMITTEE",
        ),
        (
            "party_committee",
            PARTY_COMMITTEE,
            "ALEXANDRIA DEMOCRATIC FEDERAL CAMPAIGN COMMITTEE",
        ),
        (
            "house_committee",
            HOUSE_COMMITTEE,
            "WAMPLER FOR CONGRESS COMMITTEE",
        ),
        (
            "presidential_committee",
            PRESIDENTIAL_COMMITTEE,
            "CRANE FOR PRESIDENT COMMITTEE",
        ),
        (
            "exploratory_committee",
            EXPLORATORY_COMMITTEE,
            "GRAYSON FOR PRESIDENT 1984",
        ),
    ];
    /// The filer of the F3X, F24, F1M, and F99 fixtures ("Example PAC").
    pub const PAC: &str = "C00002253";
    const AFFILIATED_PAC: &str = "C00003459";
    const OTHER_PAC: &str = "C00003020";
    const PARTY_COMMITTEE: &str = "C00078733";
    /// The filer of the F3 fixture ("Roe for Congress").
    pub const HOUSE_COMMITTEE: &str = "C00015669";
    /// The filer of the F3P fixture ("Roe for President").
    pub const PRESIDENTIAL_COMMITTEE: &str = "C00100834";
    const EXPLORATORY_COMMITTEE: &str = "C00110601";

    const V85: SpecVersion = SpecVersion::electronic(8, 5);

    /// The seed string recorded in the manifest.
    #[must_use]
    pub fn seed() -> String {
        format!("{SOFT_NAME}/{PACK_VERSION}")
    }

    // -----------------------------------------------------------------
    // Record builders
    // -----------------------------------------------------------------

    /// One record under construction: a table plus `(field, value)` pairs.
    /// Later pairs override earlier ones, so a template can be specialised.
    struct Rec {
        table: Table,
        fields: Vec<(&'static str, String)>,
    }

    /// A record filed by the PAC; [`with_filer`] re-stamps a whole filing
    /// for the other filers.
    fn rec(table: Table, token: &str) -> Rec {
        Rec {
            table,
            fields: vec![
                ("form_type", token.to_string()),
                ("filer_committee_id_number", PAC.to_string()),
            ],
        }
    }

    impl Rec {
        fn set(mut self, field: &'static str, value: impl Into<String>) -> Self {
            self.fields.push((field, value.into()));
            self
        }

        fn with(mut self, pairs: &[(&'static str, &str)]) -> Self {
            for (k, v) in pairs {
                self.fields.push((k, (*v).to_string()));
            }
            self
        }

        fn line(&self, line_no: u64) -> Result<ParsedLine> {
            let pairs = self.fields.iter().map(|(k, v)| (*k, v.as_str()));
            Ok(ParsedLine::from_pairs(self.table, V85, line_no, pairs)?)
        }
    }

    const INDIVIDUAL: &[(&str, &str)] = &[
        ("entity_type", "IND"),
        ("contributor_last_name", "Smith"),
        ("contributor_first_name", "Jane"),
        ("contributor_street_1", "1 Main St"),
        ("contributor_city", "Springfield"),
        ("contributor_state", "VA"),
        ("contributor_zip_code", "22150"),
        ("contributor_employer", "Acme Widgets"),
        ("contributor_occupation", "Engineer"),
    ];

    const VENDOR: &[(&str, &str)] = &[
        ("entity_type", "ORG"),
        ("payee_organization_name", "Acme Print"),
        ("payee_street_1", "2 Main St"),
        ("payee_city", "Springfield"),
        ("payee_state", "VA"),
        ("payee_zip_code", "22150"),
        ("expenditure_purpose_descrip", "Printing"),
        ("category_code", "001"),
    ];

    const CANDIDATE: &[(&str, &str)] = &[
        ("candidate_id_number", "H0VA01001"),
        ("candidate_last_name", "Roe"),
        ("candidate_first_name", "Rae"),
        ("candidate_office", "H"),
        ("candidate_state", "VA"),
        ("candidate_district", "01"),
    ];

    /// Schedule A receipt from an individual.
    fn receipt_ind(token: &str, tid: &str, date: &str, amount: &str) -> Rec {
        rec(Table::SchA, token)
            .with(INDIVIDUAL)
            .set("transaction_id", tid)
            .set("contribution_date", date)
            .set("contribution_amount", amount)
            .set("contribution_aggregate", amount)
    }

    /// Schedule A receipt from an organisation (bank, vendor refund).
    fn receipt_org(token: &str, tid: &str, date: &str, amount: &str, org: &str) -> Rec {
        rec(Table::SchA, token)
            .set("transaction_id", tid)
            .set("entity_type", "ORG")
            .set("contributor_organization_name", org)
            .set("contributor_street_1", "3 Bank St")
            .set("contributor_city", "Springfield")
            .set("contributor_state", "VA")
            .set("contributor_zip_code", "22150")
            .set("contribution_date", date)
            .set("contribution_amount", amount)
            .set("contribution_aggregate", amount)
    }

    /// Schedule A receipt from a political committee.
    fn receipt_com(token: &str, tid: &str, date: &str, amount: &str, name: &str, id: &str) -> Rec {
        receipt_org(token, tid, date, amount, name)
            .set("entity_type", "COM")
            .set("donor_committee_fec_id", id)
            .set("donor_committee_name", name)
    }

    /// Schedule B disbursement to a vendor.
    fn disbursement(token: &str, tid: &str, date: &str, amount: &str) -> Rec {
        rec(Table::SchB, token)
            .with(VENDOR)
            .set("transaction_id", tid)
            .set("expenditure_date", date)
            .set("expenditure_amount", amount)
    }

    /// Schedule B refund or contribution to a person or committee.
    fn disbursement_to(token: &str, tid: &str, date: &str, amount: &str, purpose: &str) -> Rec {
        disbursement(token, tid, date, amount)
            .set("expenditure_purpose_descrip", purpose)
            .set("category_code", "")
    }

    fn refund_to_individual(token: &str, tid: &str, date: &str, amount: &str) -> Rec {
        disbursement_to(token, tid, date, amount, "Contribution refund")
            .set("entity_type", "IND")
            .set("payee_organization_name", "")
            .set("payee_last_name", "Smith")
            .set("payee_first_name", "Jane")
    }

    fn payment_to_committee(
        token: &str,
        tid: &str,
        date: &str,
        amount: &str,
        purpose: &str,
        name: &str,
        id: &str,
    ) -> Rec {
        disbursement_to(token, tid, date, amount, purpose)
            .set("entity_type", "COM")
            .set("payee_organization_name", name)
            .set("beneficiary_committee_fec_id", id)
            .set("beneficiary_committee_name", name)
    }

    /// Schedule C loan from a bank. `receipt_line` is the cover line the
    /// loan is reported on: `13` on a Form 3X (both `SC/9` and `SC/10`,
    /// per the spec's value list and WebCheck), `13A`/`13B` on a Form 3.
    fn bank_loan(token: &str, tid: &str, receipt_line: &str, original: &str, balance: &str) -> Rec {
        rec(Table::SchC, token)
            .set("transaction_id", tid)
            .set("receipt_line_number", receipt_line)
            .set("entity_type", "ORG")
            .set("lender_organization_name", "First Bank")
            .set("lender_street_1", "3 Bank St")
            .set("lender_city", "Springfield")
            .set("lender_state", "VA")
            .set("lender_zip_code", "22150")
            .set("loan_amount_original", original)
            .set("loan_payment_to_date", "0.00")
            .set("loan_balance", balance)
            .set("loan_incurred_date_terms", "20260201")
            .set("loan_due_date_terms", "20261231")
            .set("loan_interest_rate_terms", "5.0%")
            .set("secured", "N")
            .set("personal_funds", "N")
    }

    /// Schedule D debt owed to a vendor.
    fn debt(token: &str, tid: &str, incurred: &str, balance: &str) -> Rec {
        rec(Table::SchD, token)
            .set("transaction_id", tid)
            .set("entity_type", "ORG")
            .set("creditor_organization_name", "Acme Print")
            .set("creditor_street_1", "2 Main St")
            .set("creditor_city", "Springfield")
            .set("creditor_state", "VA")
            .set("creditor_zip_code", "22150")
            .set("purpose_of_debt_or_obligation", "Printing")
            .set("beginning_balance_this_period", "0.00")
            .set("incurred_amount_this_period", incurred)
            .set("payment_amount_this_period", "0.00")
            .set("balance_at_close_this_period", balance)
    }

    /// Schedule E independent expenditure supporting a House candidate.
    fn independent_expenditure(
        tid: &str,
        date: &str,
        amount: &str,
        ytd: &str,
        signed: &str,
        memo: bool,
    ) -> Rec {
        rec(Table::SchE, "SE")
            .with(VENDOR)
            .with(CANDIDATE)
            .set("transaction_id", tid)
            .set("expenditure_purpose_descrip", "Digital advertising")
            .set("election_code", "G2026")
            .set("dissemination_date", date)
            .set("disbursement_date", date)
            .set("expenditure_amount", amount)
            .set("calendar_y_t_d_per_election_office", ytd)
            .set("support_oppose_code", "S")
            .set("completing_last_name", "Doe")
            .set("completing_first_name", "Pat")
            .set("date_signed", signed)
            .set("memo_code", if memo { "X" } else { "" })
    }

    /// Schedule F coordinated party expenditure.
    fn coordinated_expenditure(tid: &str, date: &str, amount: &str, aggregate: &str) -> Rec {
        rec(Table::SchF, "SF")
            .with(VENDOR)
            .set("transaction_id", tid)
            .set("coordinated_expenditures", "N")
            .set("expenditure_date", date)
            .set("expenditure_amount", amount)
            .set("aggregate_general_elec_expended", aggregate)
            .set("payee_candidate_id_number", "H0VA01001")
            .set("payee_candidate_last_name", "Roe")
            .set("payee_candidate_first_name", "Rae")
            .set("payee_candidate_office", "H")
            .set("payee_candidate_state", "VA")
            .set("payee_candidate_district", "01")
    }

    /// Schedule H3 transfer from the nonfederal account. One transfer is
    /// one `AD` record plus one record per other event type; every record
    /// of the group repeats the transfer total in `total_amount_transferred`
    /// and carries its own share in `transferred_amount`.
    fn h3(tid: &str, back: &str, event: &str, total: &str, share: &str) -> Rec {
        // The spec (and WebCheck) want an event name on direct fundraising
        // and direct candidate support records.
        let event_name = match event {
            "DF" | "DC" => "Spring fundraising dinner",
            _ => "",
        };
        rec(Table::H3, "H3")
            .set("transaction_id", tid)
            .set("back_reference_tran_id", back)
            .set("account_name", "STATE CHECKING")
            .set("event_type", event)
            .set("activity_event_name", event_name)
            .set("receipt_date", "20260310")
            .set("total_amount_transferred", total)
            .set("transferred_amount", share)
    }

    /// Schedule H4 allocated disbursement (or a memo breakdown of one).
    fn h4(tid: &str, back: &str, total: &str, fed: &str, nonfed: &str, memo: bool) -> Rec {
        rec(Table::H4, "H4")
            .with(VENDOR)
            .set("transaction_id", tid)
            .set("back_reference_tran_id", back)
            .set(
                "back_reference_sched_name",
                if back.is_empty() { "" } else { "H4" },
            )
            .set("account_identifier", "STATE CHECKING")
            .set("expenditure_date", "20260316")
            .set("total_amount", total)
            .set("federal_share", fed)
            .set("nonfederal_share", nonfed)
            .set("event_year_to_date", total)
            .set("expenditure_purpose_descrip", "Rent")
            .set("administrative_voter_drive_activity", "X")
            .set("memo_code", if memo { "X" } else { "" })
    }

    // -----------------------------------------------------------------
    // Cover pages and assembly
    // -----------------------------------------------------------------

    const TREASURER: &[(&str, &str)] = &[
        ("treasurer_last_name", "Doe"),
        ("treasurer_first_name", "Pat"),
    ];

    const Q1_2026: &[(&str, &str)] = &[
        ("report_code", "Q1"),
        ("coverage_from_date", "20260101"),
        ("coverage_through_date", "20260331"),
        ("date_signed", "20260415"),
    ];

    fn pac_address(cover: Rec) -> Rec {
        cover
            .set("committee_name", "Example PAC")
            .set("street_1", "PO BOX 1")
            .set("city", "ALEXANDRIA")
            .set("state", "VA")
            .set("zip_code", "22313")
            .with(TREASURER)
    }

    fn header() -> Result<Header> {
        Ok(Header::from_fields(&[
            "HDR",
            "FEC",
            "8.5",
            SOFT_NAME,
            PACK_VERSION,
            "",
            "",
            "",
        ])?)
    }

    /// Header + cover on line 2 + body from line 3.
    fn assemble(cover: &Rec, body: &[Rec]) -> Result<Filing> {
        let lines = body
            .iter()
            .enumerate()
            .map(|(i, r)| r.line(3 + i as u64))
            .collect::<Result<Vec<_>>>()?;
        Ok(Filing::from_parts(header()?, cover.line(2)?, lines)?)
    }

    /// Stamps `filer` on the cover and every body line.
    fn with_filer(mut filing: Filing, filer: &str) -> Result<Filing> {
        filing.summary.set("filer_committee_id_number", filer)?;
        for line in &mut filing.lines {
            line.set("filer_committee_id_number", filer)?;
        }
        Ok(filing)
    }

    fn two_places(d: Decimal) -> String {
        format!("{d:.2}")
    }

    /// Sets every line of `column` whose check is blank or disagrees to the
    /// value its rule implies, until the column balances. Each pass settles
    /// one more layer of formulas (11(a)(i) -> 11(a)(iii) -> 11(d) -> 19 ->
    /// ... -> 8), so a handful of passes reach the fixed point.
    fn fill_column(filing: &mut Filing, column: Column) -> Result<()> {
        for _ in 0..32 {
            let r = filing.reconcile()?;
            let pending: Vec<(&'static str, Decimal)> = r
                .column(column)
                .filter(|c| c.reported.is_none() || !c.matches())
                .map(|c| (c.field, c.expected))
                .collect();
            if pending.is_empty() {
                return Ok(());
            }
            for (field, expected) in pending {
                filing.summary.set(field, &two_places(expected))?;
            }
        }
        Err(format!("column {column} did not converge").into())
    }

    /// A first-quarter report is also the year (F3X) or cycle (F3, F3P) to
    /// date, so every Column B input equals its Column A line; F3X line
    /// 6(a) (cash on hand January 1) is Column A's 6(b).
    fn copy_column_a_into_b(filing: &mut Filing) -> Result<()> {
        let form = filing.summary.table();
        let rules = rules_for(form).ok_or("form has no rule table")?;
        let field_of = |column: Column, line: &str| {
            rules
                .iter()
                .find(|r| r.column == column && r.line == line)
                .map(|r| r.field)
        };
        let mut copies = Vec::new();
        for rule in rules
            .iter()
            .filter(|r| r.column == Column::B && matches!(r.source, Source::Input))
        {
            let a_line = if rule.line == "6(a)" {
                "6(b)"
            } else {
                rule.line
            };
            let Some(a_field) = field_of(Column::A, a_line) else {
                continue;
            };
            let value = filing.summary.get(a_field).unwrap_or("").to_string();
            copies.push((rule.field, value));
        }
        for (field, value) in copies {
            filing.summary.set(field, &value)?;
        }
        Ok(())
    }

    /// Real filings write `0.00` in every unused amount column.
    fn zero_blank_amounts(filing: &mut Filing) -> Result<()> {
        let blank: Vec<&'static str> = filing
            .summary
            .iter()
            .filter(|(name, value)| {
                (name.starts_with("col_a_") || name.starts_with("col_b_")) && value.is_empty()
            })
            .map(|(name, _)| name)
            .collect();
        for field in blank {
            filing.summary.set(field, "0.00")?;
        }
        Ok(())
    }

    /// Fills a periodic report's cover so every rule holds.
    fn balance(filing: &mut Filing) -> Result<()> {
        fill_column(filing, Column::A)?;
        copy_column_a_into_b(filing)?;
        fill_column(filing, Column::B)?;
        zero_blank_amounts(filing)
    }

    fn reported(r: &Reconciliation, column: Column, line: &str) -> Decimal {
        r.line(column, line)
            .and_then(|c| c.reported)
            .unwrap_or(Decimal::ZERO)
    }

    // -----------------------------------------------------------------
    // The fixtures
    // -----------------------------------------------------------------

    /// Form 3X from FECfile+'s `test_calculate_summary_column_a`
    /// transaction set (amounts, tokens, memo flags as in
    /// `web_services/summary/tests/utils.py`) plus Schedules H3 and H4.
    fn f3x() -> Result<Filing> {
        let cover = pac_address(rec(Table::F3X, "F3XN"))
            .with(Q1_2026)
            // FECfile+'s test sets cash on hand January 1 to 61 and files
            // unitemized individual receipts of 3.77 as an SA11AII
            // transaction; a .fec carries 11(a)(ii) only on the cover.
            .set("col_a_cash_on_hand_beginning_period", "61.00")
            .set("col_a_individuals_unitemized", "3.77")
            .set("col_b_year", "2026");
        let body = vec![
            receipt_ind("SA11AI", "SA11AI.1", "20260201", "10000.23"),
            receipt_com(
                "SA11B",
                "SA11B.2",
                "20260201",
                "444.44",
                "Commonwealth Party Committee",
                PARTY_COMMITTEE,
            ),
            receipt_com(
                "SA11C",
                "SA11C.3",
                "20260201",
                "555.55",
                "Friends PAC",
                OTHER_PAC,
            ),
            receipt_com(
                "SA12",
                "SA12.4",
                "20260201",
                "1212.12",
                "Example PAC Affiliate",
                AFFILIATED_PAC,
            ),
            receipt_org("SA13", "SA13.5", "20260201", "1313.13", "First Bank"),
            receipt_org("SA14", "SA14.6", "20260201", "1414.14", "Second Bank"),
            receipt_org("SA15", "SA15.7", "20260201", "1234.56", "Acme Print"),
            receipt_org("SA15", "SA15.8", "20260201", "891.23", "Acme Print"),
            receipt_org("SA15", "SA15.9", "20260201", "10000.23", "Acme Print")
                .set("memo_code", "X")
                .set("memo_text", "Memo entry: excluded from line 15"),
            receipt_com(
                "SA16",
                "SA16.10",
                "20260201",
                "16.00",
                "Roe for Congress",
                HOUSE_COMMITTEE,
            ),
            receipt_org("SA17", "SA17.11", "20260201", "200.50", "First Bank"),
            receipt_org("SA17", "SA17.12", "20260201", "-1.00", "First Bank"),
            receipt_org("SA17", "SA17.13", "20260201", "800.50", "First Bank"),
            disbursement("SB21B", "SB21B.14", "20260305", "150.00"),
            payment_to_committee(
                "SB22",
                "SB22.15",
                "20260305",
                "22.00",
                "Transfer to affiliate",
                "Example PAC Affiliate",
                AFFILIATED_PAC,
            ),
            // A contribution to a candidate carries the election it is for.
            payment_to_committee(
                "SB23",
                "SB23.16",
                "20260305",
                "14.00",
                "Contribution",
                "Roe for Congress",
                HOUSE_COMMITTEE,
            )
            .set("election_code", "P2026"),
            disbursement_to("SB26", "SB26.17", "20260305", "44.00", "Loan repayment")
                .set("payee_organization_name", "First Bank"),
            payment_to_committee(
                "SB27",
                "SB27.18",
                "20260305",
                "31.00",
                "Loan",
                "Roe for Congress",
                HOUSE_COMMITTEE,
            ),
            refund_to_individual("SB28A", "SB28A.19", "20260305", "101.50"),
            payment_to_committee(
                "SB28B",
                "SB28B.20",
                "20260305",
                "201.50",
                "Contribution refund",
                "Commonwealth Party Committee",
                PARTY_COMMITTEE,
            ),
            payment_to_committee(
                "SB28C",
                "SB28C.21",
                "20260305",
                "301.50",
                "Contribution refund",
                "Friends PAC",
                OTHER_PAC,
            ),
            disbursement_to("SB29", "SB29.22", "20260305", "201.50", "Bank fees")
                .set("payee_organization_name", "First Bank"),
            disbursement("SB30B", "SB30B.23", "20260305", "102.25")
                .set("expenditure_purpose_descrip", "Voter registration drive"),
            bank_loan("SC/9", "SC9.24", "13", "150.00", "150.00"),
            bank_loan("SC/10", "SC10.25", "13", "30.00", "30.00"),
            debt("SD9", "SD9.26", "100.00", "100.00"),
            debt("SD10", "SD10.27", "220.00", "220.00"),
            independent_expenditure("SE.28", "20260310", "65.00", "65.00", "20260415", false),
            independent_expenditure("SE.29", "20260311", "76.00", "141.00", "20260415", false),
            independent_expenditure("SE.30", "20260312", "10.00", "151.00", "20260415", false),
            independent_expenditure("SE.31", "20260312", "57.00", "151.00", "20260415", true)
                .set("memo_text", "Memo entry: excluded from line 24"),
            coordinated_expenditure("SF.32", "20260320", "65.00", "65.00"),
            coordinated_expenditure("SF.33", "20260321", "15.00", "80.00"),
            coordinated_expenditure("SF.34", "20260322", "53.00", "133.00"),
            // One 1000.00 transfer split 750 administrative + 250 direct
            // fundraising: 18(a) sums `transferred_amount` (1000.00), not
            // `total_amount_transferred` (which would give 2000.00).
            h3("H3.35", "H3.35", "AD", "1000.00", "750.00"),
            h3("H3.36", "H3.35", "DF", "1000.00", "250.00"),
            // One allocated disbursement and a memo breakdown of it:
            // 21(a)(i)/(ii) sum the non-memo shares only.
            h4("H4.37", "", "1000.00", "330.00", "670.00", false),
            h4("H4.38", "H4.37", "600.00", "198.00", "402.00", true),
        ];
        let mut filing = assemble(&cover, &body)?;
        balance(&mut filing)?;
        Ok(filing)
    }

    /// Form 3 (House candidate committee) with a candidate loan and a debt.
    fn f3() -> Result<Filing> {
        let cover = rec(Table::F3, "F3N")
            .set("committee_name", "Roe for Congress")
            .set("street_1", "PO BOX 2")
            .set("city", "RICHMOND")
            .set("state", "VA")
            .set("zip_code", "23219")
            .set("election_state", "VA")
            .set("election_district", "01")
            .with(TREASURER)
            .with(Q1_2026)
            .set("col_a_cash_on_hand_beginning_period", "5000.00")
            .set("col_a_individuals_unitemized", "150.00");
        let body = vec![
            receipt_ind("SA11AI", "SA11AI.1", "20260210", "2500.00").set("election_code", "P2026"),
            receipt_com(
                "SA11C",
                "SA11C.2",
                "20260212",
                "1000.00",
                "Friends PAC",
                OTHER_PAC,
            )
            .set("election_code", "P2026"),
            disbursement("SB17", "SB17.3", "20260301", "1200.00"),
            refund_to_individual("SB20A", "SB20A.4", "20260302", "250.00"),
            rec(Table::SchC, "SC/9")
                .set("transaction_id", "SC9.5")
                .set("receipt_line_number", "13A")
                .set("entity_type", "CAN")
                .set("lender_last_name", "Roe")
                .set("lender_first_name", "Rae")
                .set("lender_street_1", "4 Hill St")
                .set("lender_city", "Richmond")
                .set("lender_state", "VA")
                .set("lender_zip_code", "23219")
                .set("election_code", "P2026")
                .set("loan_amount_original", "10000.00")
                .set("loan_payment_to_date", "0.00")
                .set("loan_balance", "10000.00")
                .set("loan_incurred_date_terms", "20260201")
                .set("loan_due_date_terms", "20261231")
                .set("loan_interest_rate_terms", "0.0%")
                .set("secured", "N")
                .set("personal_funds", "Y")
                .set("lender_candidate_id_number", "H0VA01001")
                .set("lender_candidate_last_name", "Roe")
                .set("lender_candidate_first_name", "Rae")
                .set("lender_candidate_office", "H")
                .set("lender_candidate_state", "VA")
                .set("lender_candidate_district", "01"),
            debt("SD10", "SD10.6", "400.00", "400.00"),
        ];
        let mut filing = with_filer(assemble(&cover, &body)?, HOUSE_COMMITTEE)?;
        balance(&mut filing)?;
        Ok(filing)
    }

    /// Form 3P (presidential committee) with the four schedule lines the
    /// form's Column A rules sum.
    fn f3p() -> Result<Filing> {
        let cover = rec(Table::F3P, "F3PN")
            .set("committee_name", "Roe for President")
            .set("street_1", "PO BOX 3")
            .set("city", "DES MOINES")
            .set("state", "IA")
            .set("zip_code", "50309")
            .set("activity_primary", "X")
            .with(TREASURER)
            .with(Q1_2026)
            .set("col_a_cash_on_hand_beginning_period", "250.00")
            .set("col_a_individuals_unitemized", "75.00");
        let body = vec![
            receipt_ind("SA17A", "SA17A.1", "20260210", "3300.00").set("election_code", "P2028"),
            receipt_com(
                "SA18",
                "SA18.2",
                "20260212",
                "5000.00",
                "Roe Exploratory Committee",
                EXPLORATORY_COMMITTEE,
            ),
            disbursement("SB23", "SB23.3", "20260301", "800.00"),
            refund_to_individual("SB28A", "SB28A.4", "20260302", "300.00"),
        ];
        let mut filing = with_filer(assemble(&cover, &body)?, PRESIDENTIAL_COMMITTEE)?;
        balance(&mut filing)?;
        // The spec derives lines 14 and 15 from Column B: 17(e) - 28(d)
        // and 23 - 20(a). Neither is in the rule table, so set them here.
        let r = filing.reconcile()?;
        let net_contributions = reported(&r, Column::B, "17(e)") - reported(&r, Column::B, "28(d)");
        let net_operating = reported(&r, Column::B, "23") - reported(&r, Column::B, "20(a)");
        filing
            .summary
            .set("col_a_net_contributions", &two_places(net_contributions))?;
        filing.summary.set(
            "col_a_net_operating_expenditures",
            &two_places(net_operating),
        )?;
        Ok(filing)
    }

    /// Form 24 (48-hour report) with two Schedule E lines.
    fn f24() -> Result<Filing> {
        let cover = pac_address(rec(Table::F24, "F24N"))
            .set("report_type", "48")
            .set("date_signed", "20260317");
        let body = vec![
            independent_expenditure("SE.1", "20260316", "1500.00", "1500.00", "20260317", false),
            independent_expenditure("SE.2", "20260316", "2500.00", "4000.00", "20260317", false)
                .set("expenditure_purpose_descrip", "Direct mail"),
        ];
        assemble(&cover, &body)
    }

    /// Form 1M: notification of multicandidate status, five candidates.
    fn f1m() -> Result<Filing> {
        let candidates: [(&str, [(&str, &str); 7]); 5] = [
            (
                "first",
                [
                    ("id_number", "H0VA01001"),
                    ("last_name", "Roe"),
                    ("first_name", "Rae"),
                    ("office", "H"),
                    ("state", "VA"),
                    ("district", "01"),
                    ("contribution_date", "20260105"),
                ],
            ),
            (
                "second",
                [
                    ("id_number", "S0VA00001"),
                    ("last_name", "Poe"),
                    ("first_name", "Pia"),
                    ("office", "S"),
                    ("state", "VA"),
                    ("district", "00"),
                    ("contribution_date", "20260112"),
                ],
            ),
            (
                "third",
                [
                    ("id_number", "H0VA02002"),
                    ("last_name", "Loe"),
                    ("first_name", "Lee"),
                    ("office", "H"),
                    ("state", "VA"),
                    ("district", "02"),
                    ("contribution_date", "20260119"),
                ],
            ),
            (
                "fourth",
                [
                    ("id_number", "H0MD03003"),
                    ("last_name", "Moe"),
                    ("first_name", "Max"),
                    ("office", "H"),
                    ("state", "MD"),
                    ("district", "03"),
                    ("contribution_date", "20260126"),
                ],
            ),
            (
                "fifth",
                [
                    ("id_number", "H0MD04004"),
                    ("last_name", "Noe"),
                    ("first_name", "Nia"),
                    ("office", "H"),
                    ("state", "MD"),
                    ("district", "04"),
                    ("contribution_date", "20260202"),
                ],
            ),
        ];
        let mut cover = pac_address(rec(Table::F1M, "F1MN"))
            .set("committee_type", "N")
            .set("fifty_first_contributor_date", "20260210")
            .set("original_registration_date", "20250601")
            .set("requirements_met_date", "20260210")
            .set("date_signed", "20260215");
        let mut owned: Vec<(String, String)> = Vec::new();
        for (ordinal, fields) in &candidates {
            for (suffix, value) in fields {
                owned.push((
                    format!("{ordinal}_candidate_{suffix}"),
                    (*value).to_string(),
                ));
            }
        }
        // `Rec` stores `&'static str` field names; the layout owns them.
        let layout = Table::F1M.layout(V85).ok_or("no F1M layout at 8.5")?;
        for (name, value) in &owned {
            let field = layout
                .field(name)
                .ok_or_else(|| format!("F1M has no field {name}"))?;
            cover = cover.set(field.name, value.as_str());
        }
        assemble(&cover, &[])
    }

    /// Form 99 miscellaneous text with a `[BEGINTEXT]` block.
    fn f99() -> Result<Filing> {
        let cover = pac_address(rec(Table::F99, "F99"))
            .set("date_signed", "20260415")
            .set("text_code", "MST")
            .set(
                "text",
                "This filing is a synthetic fixture from the hardmoney golden pack.\n\nIt \
                 carries no real committee activity and exists so that software reading \
                 Form 99 text blocks can be tested against a known file.",
            );
        assemble(&cover, &[])
    }

    fn set_on_line(filing: &mut Filing, tid: &str, field: &str, value: &str) -> Result<()> {
        let line = filing
            .lines
            .iter_mut()
            .find(|l| l.get("transaction_id") == Some(tid))
            .ok_or_else(|| format!("no line with transaction id {tid}"))?;
        Ok(line.set(field, value)?)
    }

    /// One fixture: its files are `<name>.fec` and the sidecars.
    pub struct Fixture {
        pub name: &'static str,
        pub description: &'static str,
        pub provenance: &'static str,
        /// Emit `<name>.expected_column_a.json`.
        pub column_a: bool,
        pub filing: Filing,
    }

    const IDS_NOTE: &str = "Committee ids are those of committees terminated decades ago (see \
                            committee_ids), used only because WebCheck checks ids against the \
                            FEC registry; no other content relates to them.";

    const SYNTHETIC: &str = "Synthetic. Every name, address, amount, and date is invented, \
                             shaped after FEC-accepted spec-8.5 filings in tests/fixtures/. ";

    const FECFILE_PLUS: &str = "Synthetic. Transaction amounts, line tokens, and memo flags are \
                                FECfile+'s public-domain test set (fecgov/fecfile-web-api, \
                                django-backend/fecfiler/web_services/summary/tests/utils.py, \
                                asserted by reports/form_3x/tests/test_summary.py::\
                                test_calculate_summary_column_a); cash on hand January 1 (61) \
                                and unitemized individual receipts (3.77) come from the same \
                                test. Names, addresses, and dates are invented; the H3 and H4 \
                                records are hardmoney's addition. ";

    /// Builds every fixture, in manifest order.
    pub fn fixtures() -> Result<Vec<Fixture>> {
        let f3x = f3x()?;
        let mut off_by_one_cent = f3x.clone();
        off_by_one_cent
            .summary
            .set("col_a_individuals_itemized", "10000.24")?;
        let mut duplicate = f3x.clone();
        set_on_line(&mut duplicate, "SA11B.2", "transaction_id", "SA11AI.1")?;
        let mut missing = f3x.clone();
        set_on_line(&mut missing, "SA11AI.1", "contributor_last_name", "")?;

        Ok(vec![
            Fixture {
                name: "f3x",
                description: "Form 3X (PAC/party quarterly) with Schedules A, B, C, D, E, F, H3 \
                              and H4; cover filled so every Column A and Column B line balances. \
                              The Column A lines FECfile+ computes match its test's expected \
                              values; 18(a), 18(c), 21(a)(i), 21(a)(ii), and the totals that \
                              include them carry the H3/H4-derived values FECfile+ stubs to zero.",
                provenance: FECFILE_PLUS,
                column_a: true,
                filing: f3x,
            },
            Fixture {
                name: "f3",
                description: "Form 3 (House candidate quarterly) with SA11AI, SA11C, SB17, SB20A, \
                              a Schedule C candidate loan (SC/9) and a Schedule D debt (SD10); \
                              cover balanced.",
                provenance: SYNTHETIC,
                column_a: false,
                filing: f3()?,
            },
            Fixture {
                name: "f3p",
                description: "Form 3P (presidential quarterly) with SA17A, SA18, SB23, SB28A; \
                              cover balanced, lines 14 and 15 derived from Column B as the spec \
                              says.",
                provenance: SYNTHETIC,
                column_a: false,
                filing: f3p()?,
            },
            Fixture {
                name: "f24",
                description: "Form 24 (48-hour independent expenditure report) with two Schedule \
                              E lines.",
                provenance: SYNTHETIC,
                column_a: false,
                filing: f24()?,
            },
            Fixture {
                name: "f1m",
                description: "Form 1M (notification of multicandidate status) naming five \
                              candidates.",
                provenance: SYNTHETIC,
                column_a: false,
                filing: f1m()?,
            },
            Fixture {
                name: "f99",
                description: "Form 99 (miscellaneous text) with a two-paragraph [BEGINTEXT] block.",
                provenance: SYNTHETIC,
                column_a: false,
                filing: f99()?,
            },
            Fixture {
                name: "f3x_cover_off_by_one_cent",
                description: "f3x with Column A line 11(a)(i) reported as 10000.24 (the schedule \
                              sums to 10000.23). Validates with no findings, as the FEC's \
                              validator would accept it; reconcile flags 11(a)(i) (+0.01) and \
                              11(a)(iii) (-0.01, its formula evaluated over the reported lines).",
                provenance: FECFILE_PLUS,
                column_a: false,
                filing: off_by_one_cent,
            },
            Fixture {
                name: "f3x_duplicate_transaction_id",
                description: "f3x with line 4 (SA11B) reusing line 3's transaction id SA11AI.1. \
                              One duplicate_transaction_id error (FEC message #40) on line 4; the \
                              cover still balances.",
                provenance: FECFILE_PLUS,
                column_a: false,
                filing: duplicate,
            },
            Fixture {
                name: "f3x_missing_required_field",
                description: "f3x with contributor_last_name blank on line 3 (SA11AI, an \
                              individual). One required_field_empty error (FEC message #3) on \
                              line 3; the cover still balances.",
                provenance: FECFILE_PLUS,
                column_a: false,
                filing: missing,
            },
        ])
    }

    // -----------------------------------------------------------------
    // Sidecars and manifest
    // -----------------------------------------------------------------

    /// `11(a)(i)` -> `line_11ai`, `6(b)` -> `line_6b`: the keys of
    /// FECfile+'s `calculate_summary_column_a`.
    #[must_use]
    pub fn line_key(label: &str) -> String {
        let mut key = String::from("line_");
        key.extend(label.chars().filter(|c| *c != '(' && *c != ')'));
        key
    }

    fn pretty(value: &Value) -> Result<Vec<u8>> {
        let mut text = serde_json::to_string_pretty(value)?;
        text.push('\n');
        Ok(text.into_bytes())
    }

    /// The document `hardmoney validate <file> --json` prints.
    fn validation_json(file: &str, filing: &Filing, v: &Validation) -> Value {
        let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
        for f in v {
            *by_rule.entry(f.rule.to_string()).or_default() += 1;
        }
        json!({
            "file": file,
            "form_type": filing.raw_form_type,
            "version": filing.version,
            "line_count": filing.lines.len(),
            "acceptable": v.is_acceptable(),
            "errors": v.error_count(),
            "warnings": v.warning_count(),
            "findings_by_rule": by_rule,
            "findings": v.findings,
        })
    }

    /// The document `hardmoney reconcile <file> --json --all` prints.
    fn reconciliation_json(file: &str, r: &Reconciliation) -> Value {
        json!({
            "form": r.form,
            "file": file,
            "tolerance": Decimal::ZERO,
            "checks": r.checks.len(),
            "disagreeing": r.mismatches().count(),
            "lines": r.checks,
        })
    }

    /// Every Column A line, keyed like FECfile+'s summary dict, as strings
    /// with two decimals (exact; a JSON number would be read as a float).
    /// Lines with no rule of their own (11(a)(ii), 6(b)) are read from the
    /// cover, as `calculate_summary_column_a` reads them from its inputs.
    fn expected_column_a(filing: &Filing, r: &Reconciliation) -> Result<Value> {
        let rules = rules_for(r.form).ok_or("form has no rule table")?;
        let mut out = Map::new();
        for rule in rules.iter().filter(|r| r.column == Column::A) {
            let value = match rule.source {
                Source::Input => filing
                    .summary
                    .get_non_empty(rule.field)
                    .and_then(hardmoney::parse_money)
                    .unwrap_or(Decimal::ZERO),
                _ => r
                    .line(Column::A, rule.line)
                    .map(|c| c.expected)
                    .ok_or_else(|| format!("no check for line {}", rule.line))?,
            };
            out.insert(line_key(rule.line), Value::String(two_places(value)));
        }
        Ok(Value::Object(out))
    }

    /// Renders the fixtures to their files: `<name>.fec`, the sidecars,
    /// and `MANIFEST.json`, keyed by file name.
    pub fn render(fixtures: &[Fixture]) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut manifest_entries = Vec::new();
        for fx in fixtures {
            let file = format!("{}.fec", fx.name);
            let bytes = fx.filing.to_fec();
            // Re-parse the bytes so line numbers and findings are those of
            // the file on disk, not of the in-memory construction.
            let lenient = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
            let validation = lenient.validate();
            let filing = lenient.value();
            let mut sidecars = Vec::new();

            let validation_file = format!("{}.validation.json", fx.name);
            files.insert(
                validation_file.clone(),
                pretty(&validation_json(&file, filing, &validation))?,
            );
            sidecars.push(validation_file);

            let reconciliation = match filing.reconcile() {
                Ok(r) => {
                    let name = format!("{}.reconciliation.json", fx.name);
                    files.insert(name.clone(), pretty(&reconciliation_json(&file, &r))?);
                    sidecars.push(name);
                    if fx.column_a {
                        let name = format!("{}.expected_column_a.json", fx.name);
                        files.insert(name.clone(), pretty(&expected_column_a(filing, &r)?)?);
                        sidecars.push(name);
                    }
                    Some(r)
                }
                Err(_) => None,
            };

            let findings: Vec<Value> = validation
                .iter()
                .map(|f| {
                    json!({
                        "rule": f.rule.to_string(),
                        "severity": f.severity,
                        "line_no": f.line_no,
                        "form_type": f.form_type,
                        "field": f.field,
                    })
                })
                .collect();
            let reconciliation_expected = reconciliation.as_ref().map(|r| {
                let lines: Vec<Value> = r
                    .mismatches()
                    .map(|c| {
                        json!({
                            "column": c.column,
                            "line": c.line,
                            "reported": c.reported,
                            "expected": c.expected,
                            "delta": c.delta,
                        })
                    })
                    .collect();
                json!({
                    "checks": r.checks.len(),
                    "disagreeing": lines.len(),
                    "balances": r.balances(),
                    "lines": lines,
                })
            });

            manifest_entries.push(json!({
                "name": fx.name,
                "file": file,
                "form": filing.summary.table(),
                "form_type": filing.raw_form_type,
                "version": filing.version,
                "body_lines": filing.lines.len(),
                "bytes": bytes.len(),
                "sha256": sha256_hex(&bytes),
                "description": fx.description,
                "provenance": format!("{}{IDS_NOTE}", fx.provenance),
                "sidecars": sidecars,
                "expected": {
                    "acceptable": validation.is_acceptable(),
                    "errors": validation.error_count(),
                    "warnings": validation.warning_count(),
                    "findings": findings,
                    "reconciliation": reconciliation_expected,
                },
            }));
            files.insert(file, bytes);
        }
        let manifest = json!({
            "pack": SOFT_NAME,
            "pack_version": PACK_VERSION,
            "spec_version": V85,
            "seed": seed(),
            "committee_ids": COMMITTEE_IDS
                .iter()
                .map(|(role, id, registry_name)| {
                    json!({ "role": role, "id": id, "registry_name": registry_name })
                })
                .collect::<Vec<_>>(),
            "committee_ids_note": "WebCheck validates every committee id in a filing against the \
                                   FEC's committee registry, so no fictitious id can pass it. These \
                                   ids appear in the FEC's 1990 committee master and not in the 2024 \
                                   one. The fixtures' committee names, addresses, transactions, and \
                                   dates are invented and do not describe these committees.",
            "generator": "examples/golden_fixtures.rs",
            "regenerate": format!("cargo run --example golden_fixtures -- {DEFAULT_DIR}"),
            "verify": "cargo test --all-features --test golden_fixtures",
            "license": "Apache-2.0 OR BSD-3-Clause; the FECfile+ transaction amounts are public domain (17 U.S.C. 105)",
            "fixtures": manifest_entries,
        });
        files.insert("MANIFEST.json".to_string(), pretty(&manifest)?);
        Ok(files)
    }

    /// Builds and renders the whole pack.
    pub fn generate() -> Result<BTreeMap<String, Vec<u8>>> {
        render(&fixtures()?)
    }

    /// Generates the pack and writes it under `dir` (created if needed).
    /// Files the generator does not produce (`README.md`) are left alone.
    pub fn write_to(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
        let files = generate()?;
        std::fs::create_dir_all(dir)?;
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes)?;
        }
        Ok(files)
    }

    // -----------------------------------------------------------------
    // SHA-256 (FIPS 180-4), so the manifest needs no extra dependency
    // -----------------------------------------------------------------

    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    /// Lower-case hex SHA-256 of `data`.
    #[must_use]
    pub fn sha256_hex(data: &[u8]) -> String {
        let mut h: [u32; 8] = [
            0x6a09_e667,
            0xbb67_ae85,
            0x3c6e_f372,
            0xa54f_f53a,
            0x510e_527f,
            0x9b05_688c,
            0x1f83_d9ab,
            0x5be0_cd19,
        ];
        let mut msg = data.to_vec();
        let bit_len = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks_exact(64) {
            let mut w = [0u32; 64];
            for (slot, word) in w.iter_mut().zip(chunk.chunks_exact(4)) {
                *slot = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
            for (k, wi) in K.iter().zip(w.iter()) {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(*k)
                    .wrapping_add(*wi);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
                *slot = slot.wrapping_add(v);
            }
        }
        h.iter().map(|v| format!("{v:08x}")).collect()
    }
}

#[cfg(feature = "serde")]
fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(pack::DEFAULT_DIR)
        });
    match pack::write_to(&dir) {
        Ok(files) => {
            println!("wrote {} file(s) to {}", files.len(), dir.display());
            for (name, bytes) in &files {
                println!("  {name:<45} {:>7} bytes", bytes.len());
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "serde"))]
fn main() {
    eprintln!("golden_fixtures needs the `serde` feature (on by default)");
    std::process::exit(2);
}
