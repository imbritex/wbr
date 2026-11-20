use std::collections::HashMap;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"),
    );

    let material_entry = manifest_dir.join("ui/vendor/material-components/material.slint");
    println!("cargo:rerun-if-changed=ui");

    let config = slint_build::CompilerConfiguration::new().with_library_paths(HashMap::from([
        ("material".to_string(), material_entry),
    ]));

    slint_build::compile_with_config("ui/main.slint", config)
        .expect("Failed to compile Slint UI");
}
