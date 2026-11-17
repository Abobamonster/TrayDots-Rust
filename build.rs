use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bin=rust-dots=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=rust-dots=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
