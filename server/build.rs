// Rebuild when a migration is added: sqlx::migrate! embeds the files at compile time.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
