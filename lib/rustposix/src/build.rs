use std::path::Path;
use std::process::Command;

fn main() {
    // The name of the shared library file without the 'lib' prefix or '.so' extension
    let lib_name = "rustposix";
    // Assuming the .so file is in the current directory, same as Cargo.toml and build.rs
    let so_file_path = format!("./lib{}.so", lib_name);

    // Only attempt to create a symlink if the .so file exists
    if Path::new(&so_file_path).exists() {
        // Example command to create a symlink in a directory (adjust as necessary)
        // This is a placeholder and likely needs adjustment
        let link_path = format!("./target/debug/lib{}.so", lib_name);

        // Remove existing symlink if it exists to avoid errors
        let _ = std::fs::remove_file(&link_path);

        match Command::new("ln")
            .args(&["-s", &so_file_path, &link_path])
            .status()
        {
            Ok(status) if status.success() => {
                println!("Successfully created symlink for {}", lib_name);
            }
            _ => {
                eprintln!("Failed to create symlink for {}", lib_name);
            }
        }
    } else {
        eprintln!("Shared library file {} does not exist.", so_file_path);
    }

    println!("cargo:rustc-link-search=native=./target/debug");
    println!("cargo:rustc-link-lib=dylib={}", lib_name);
}
