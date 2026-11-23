fn main() {
    // sqlx::migrate! embeds files; new migrations must invalidate the build too.
    println!("cargo:rerun-if-changed=../../migrations");
}
