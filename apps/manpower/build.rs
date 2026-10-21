fn main() {
    // Rebuild when migrations change, so `renox::migrations!()` sees new files.
    println!("cargo:rerun-if-changed=migrations");
}
