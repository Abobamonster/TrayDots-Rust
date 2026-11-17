use std::{env, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("app.manifest");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rustc-link-arg-bin=tray-dots-rust=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bin=tray-dots-rust=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
