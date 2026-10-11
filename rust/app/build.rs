fn main() {
    // Native release bundles normalize libmpv and its dependency install names
    // to @rpath/SONAME. Keep lookup inside the package on both target families.
    #[cfg(target_os = "macos")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Resources/lib");
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/jellybridge/lib");
}
