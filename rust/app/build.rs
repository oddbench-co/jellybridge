fn main() {
    // Native release bundles normalize libmpv and its dependency install names
    // to @rpath/SONAME. Keep lookup inside the package on both target families.
    // The target's OS, not the host's: a build script runs on the host, so
    // `cfg!(target_os)` here would describe the machine doing a cross build.
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Resources/lib");
        }
        Ok("linux") => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/jellybridge/lib"),
        // Windows finds libmpv-2.dll beside the executable.
        _ => {}
    }
}
