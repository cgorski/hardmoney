//! `sqlx::migrate!` embeds `migrations/*.sql` at compile time but Cargo does
//! not know about those files, so an edited migration would not trigger a
//! rebuild (and CI could run stale SQL). This is sqlx's documented remedy.
//!
//! NB: once a build script emits *any* `rerun-if-changed`, Cargo stops
//! watching the package's other files by default, so `src` and `data` are
//! listed explicitly too.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=data/fec-csv-sources");
    println!("cargo:rerun-if-changed=build.rs");
}
