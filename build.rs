fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:warning=TARGET={target}");
    let path = std::env::current_dir().unwrap();
    println!("cargo:warning=HOLAAAA corri. {path:?}");
    if target.contains("wasm32") {
        println!("Compilando WASM");
        return;
    }

    let command = std::process::Command::new("zig")
        .args(["build"])
        .current_dir("./convert-songs/")
        .status()
        .expect("Problems creating zig process");
    if !command.success() {
        panic!("Problemas con la compilacion del modulo en zig")
    }

    use std::env;
    use std::fs;
    use std::path::PathBuf;

    // Get the output directory for the final executable
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:warning=El OUT_DIR es =  {out_dir:?}");
    // This will be something like target/debug/build/convert-main-c12345/out
    // We want to get to target/debug/
    let profile = env::var("PROFILE").unwrap();
    let target_dir = out_dir
        .ancestors()
        .nth(4)
        .unwrap()
        .join(profile)
        .join("deps");

    println!("cargo:warning=El nuevo target_dir es {target_dir:?}");
    // Define the source and destination for our .so file
    let lib_source = PathBuf::from("./convert-songs/zig-out/lib/libconvert-rs.so");
    let lib_dest = target_dir.join("libconvert-rs.so");

    println!("cargo:warning=Estoy en donde se compila la parte de zig corre. {path:?} {lib_source:?} {lib_dest :?} ");
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:warning=TARGET={target}");
    // Copy the file
    if lib_source.exists() {
        fs::copy(&lib_source, &lib_dest).expect("Failed to copy .so file");
        println!(
            "cargo:warning=Copied {} to {}",
            lib_source.display(),
            lib_dest.display()
        );
    }
    println!("cargo:rerun-if-changed=./convert-songs/build.zig");
    println!("cargo:rerun-if-changed=./convert-songs/src/root.zig");
    println!("cargo:rerun-if-changed=./convert-songs/src/main.zig");
    // Spotify creds are now read from runtime env vars (not embedded), so the
    // build no longer depends on a .env file.
    // Link against (and find at runtime) the .so we just copied into the
    // deps dir. Both paths are absolute so they resolve regardless of cwd.
    println!("cargo:rustc-link-search=native={}", target_dir.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath={}", target_dir.display());
    println!("cargo:rustc-link-lib=dylib=convert-rs");
    let ld_li = env::var("LD_LIBRARY_PATH").unwrap_or_default();
    println!("cargo:warning= LD_LIBRARY_PATH = {ld_li}");
}
