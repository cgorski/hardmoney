//! `sqlx::migrate!` embeds `migrations/*.sql` at compile time but Cargo does
//! not know about those files, so an edited migration would not trigger a
//! rebuild (and CI could run stale SQL). This is sqlx's documented remedy.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
