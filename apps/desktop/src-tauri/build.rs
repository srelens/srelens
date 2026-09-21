fn main() {
    tauri_build::build();
    embed_common_controls_manifest_in_test_binaries();
}

/// Windows targets only: give this crate's test binaries the common-controls
/// v6 manifest that `tauri_build` embeds in the app binary alone.
///
/// The crate links `TaskDialogIndirect` (through `rfd`, under
/// `tauri-plugin-dialog`), and `C:\Windows\System32\comctl32.dll` is v5.82,
/// which does not export it. v6 is reached only through a side-by-side
/// manifest, and comctl32 is a KnownDLL, so dropping a v6 copy beside the
/// binary is ignored. Without a manifest the process therefore fails to load:
/// every `cargo test -p srelens-desktop` on Windows died with exit code
/// `0xc0000139` (`STATUS_ENTRYPOINT_NOT_FOUND`) before a single test ran,
/// naming no test and no symbol. `srelens` itself always worked, and CI is
/// Linux, so nothing caught it.
///
/// Reads `CARGO_CFG_TARGET_OS` rather than `cfg!(target_os)`: a build script
/// is compiled for the HOST, so `cfg!` here answers for the machine doing the
/// build, not the machine the binary will run on.
fn embed_common_controls_manifest_in_test_binaries() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df"
        language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
"#;
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR"))
        .join("common-controls.manifest");
    std::fs::write(&out, MANIFEST).expect("write the test manifest");

    // Not `rustc-link-arg-tests`: Cargo matches that on `TargetKind::Test`,
    // and a lib compiled in test mode is still `TargetKind::Lib`, so it
    // reaches `tests/*.rs` and never the unit-test harness — which is the
    // binary that was failing. Checked rather than assumed: with `-tests` the
    // manifest is absent from `srelens_desktop_lib-*.exe` and the harness
    // still dies at startup.
    //
    // So the flags go to every target and the app binary opts back out. It
    // already carries `tauri_build`'s manifest inside `resource.lib`, and a
    // second one is not merged but rejected — `CVT1100: duplicate resource`,
    // which fails the build of the shipping binary outright. `/MANIFEST:NO`
    // lands after these on the bin's command line and wins, so `srelens`
    // links exactly as it did before this function existed. The cost is one
    // `LNK4075: ignoring '/MANIFESTINPUT' due to '/MANIFEST:NO'` per link of
    // the app on Windows, which is this trade-off announcing itself.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", out.display());
    println!("cargo:rustc-link-arg-bin=srelens=/MANIFEST:NO");
}
