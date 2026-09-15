//! Generated format tables: the [`Table`] enum, and for each table a module
//! with a marker type, `Field` constants, version [`Layout`]s, and
//! [`FieldSpec`] rows.
//!
//! This file is a thin shell; the contents are produced by `build.rs` from
//! `data/fec-csv-sources/*.csv` and `data/fec-spec/spec-*.json` on every
//! build, so they can never be stale. See [`crate::parser::schema`] for the
//! types and the design.
//!
//! # Provenance
//!
//! The column layouts originate from the community-maintained fech-sources
//! project (<https://github.com/dwillis/fech-sources>, derived from the
//! NYT's Fech Ruby gem), current through FEC spec 8.5, with the corrections
//! listed in `NOTICE`; `F2S.csv` is authored locally. The field
//! specifications are distilled from the FEC's own *Electronic Filing
//! Specification Requirements, Part II* workbook (public domain).

#![allow(clippy::too_many_lines, clippy::large_const_arrays)]

use crate::parser::schema::{
    Field, FieldDef, FieldKind, FieldSpec, Layout, Requirement, SpecVersion, TableMarker,
};

include!(concat!(env!("OUT_DIR"), "/tables.rs"));
