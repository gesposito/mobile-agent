//! Generates the emulator controller client from the proto that the installed
//! Android Emulator ships, `emulator/lib/emulator_controller.proto` in the SDK.
//! That copy describes the RPCs the running emulator implements; the AOSP
//! prebuilts copy is older (spike S7).
//!
//! Lookup order:
//!
//! 1. `MOBILE_AGENT_EMULATOR_PROTO`, the path of the proto file;
//! 2. `$ANDROID_HOME/emulator/lib/emulator_controller.proto`;
//! 3. the same under `$ANDROID_SDK_ROOT`;
//! 4. the same under the SDK's default location: `~/Library/Android/sdk` on
//!    macOS, `~/Android/Sdk` on Linux and `%LOCALAPPDATA%\Android\Sdk` on
//!    Windows;
//! 5. the vendored copy under `crates/android/proto/`, taken from emulator
//!    37.1.11, with a build warning, so hosts without the SDK (the Linux CI
//!    job) still build.
//!
//! Without any proto the build fails and says how to provide one. `protoc` and
//! the well-known types come from protoc-bin-vendored, so no system protoc is
//! needed.

use std::env;
use std::path::PathBuf;

const PROTO_VAR: &str = "MOBILE_AGENT_EMULATOR_PROTO";
const IN_SDK: &str = "emulator/lib/emulator_controller.proto";

/// Where the SDK's copy may be, in lookup order.
fn sdk_candidates() -> Vec<PathBuf> {
    let set = |var: &str| {
        env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let mut sdks: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .into_iter()
        .filter_map(set)
        .collect();
    let default_sdk = if cfg!(target_os = "macos") {
        set("HOME").map(|home| home.join("Library/Android/sdk"))
    } else if cfg!(windows) {
        set("LOCALAPPDATA").map(|local| local.join("Android").join("Sdk"))
    } else {
        set("HOME").map(|home| home.join("Android/Sdk"))
    };
    sdks.extend(default_sdk);
    sdks.into_iter().map(|sdk| sdk.join(IN_SDK)).collect()
}

fn locate_proto() -> PathBuf {
    if let Some(explicit) = env::var_os(PROTO_VAR).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(explicit);
        assert!(
            path.is_file(),
            "{PROTO_VAR} names {}, which is not a file",
            path.display()
        );
        return path;
    }
    let searched = sdk_candidates();
    if let Some(found) = searched.iter().find(|p| p.is_file()) {
        return found.clone();
    }
    let vendored =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("proto/emulator_controller.proto");
    if vendored.is_file() {
        println!(
            "cargo:warning=crates/android: no installed emulator_controller.proto found; \
             building the controller client from the vendored copy under crates/android/proto \
             (emulator 37.1.11). Install the SDK's emulator package to build from the installed one."
        );
        return vendored;
    }
    let listed: Vec<String> = searched
        .iter()
        .map(|p| format!("  {}", p.display()))
        .collect();
    panic!(
        "emulator_controller.proto not found. crates/android generates its emulator \
         controller client from the proto that the installed Android Emulator ships. \
         Searched:\n{}\nInstall the SDK's emulator package (`sdkmanager emulator`), point \
         ANDROID_HOME at an SDK that has it, or set {PROTO_VAR} to the proto file.",
        listed.join("\n")
    );
}

fn main() {
    for var in [PROTO_VAR, "ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let proto = locate_proto();
    println!("cargo:rerun-if-changed={}", proto.display());
    println!(
        "cargo:rustc-env=MOBILE_AGENT_EMULATOR_PROTO_SOURCE={}",
        proto.display()
    );
    let proto_dir = proto
        .parent()
        .expect("the proto file has a parent directory")
        .to_path_buf();

    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path().expect("vendored protoc"));
    let well_known = protoc_bin_vendored::include_path().expect("vendored protobuf includes");

    tonic_prost_build::configure()
        .build_server(false)
        // Frame payloads decode into `Bytes` instead of being copied into a Vec.
        .bytes(".android.emulation.control.Image.image")
        .compile_with_config(
            config,
            std::slice::from_ref(&proto),
            &[proto_dir, well_known],
        )
        .unwrap_or_else(|e| panic!("compiling {}: {e}", proto.display()));
}
