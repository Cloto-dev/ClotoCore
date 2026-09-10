//! The admin key's *source* decides whether a rotation survives a restart.
//!
//! `dotenvy` never overwrites a variable the process already has, so a
//! deployment that receives `CLOTO_API_KEY` from its environment (a systemd
//! `EnvironmentFile`, a container env) keeps loading that value at every
//! boot, whatever the kernel wrote into an `.env`. The only moment this can
//! be observed is before the `.env` is loaded, which is why the entry points
//! sample it — and why the sampling has to be pinned here: a handler test
//! passes just as well when nobody calls the sampler at all.

/// The sampler runs before any `.env` load and answers for the whole process.
///
/// This lives in its own test binary: the snapshot is a `OnceLock`, so a
/// process can only answer this question once.
#[test]
fn env_supplied_key_is_visible_to_a_later_rotation() {
    std::env::set_var("CLOTO_API_KEY", "key-from-the-environment");
    cloto_core::apikey::note_env_key_before_dotenv();

    assert!(
        cloto_core::apikey::env_key_at_boot(),
        "a key already in the environment must be recorded as such"
    );
    assert!(
        cloto_core::apikey::rotation_persistence_note(
            cloto_core::apikey::env_key_at_boot(),
            std::path::Path::new("/var/lib/clotocore/.env"),
        )
        .is_some(),
        "a rotation on such a deployment must carry a note"
    );
}

/// The headless entry point samples the environment *before* it loads any
/// `.env`. Order is the whole property: after the load the two sources are
/// indistinguishable, so a sampler placed one line later always answers
/// "not from the environment" and the warning silently never fires.
#[test]
fn headless_entry_point_samples_before_it_loads_dotenv() {
    let main_rs = include_str!("../src/main.rs");
    let sample = main_rs
        .find("apikey::note_env_key_before_dotenv()")
        .expect("crates/core/src/main.rs must sample the environment at boot");
    let load = main_rs
        .find("dotenvy::dotenv()")
        .expect("crates/core/src/main.rs must load .env");
    assert!(
        sample < load,
        "the sample must come before the .env load, or it can no longer tell the two apart"
    );
}

/// The desktop entry point has the same requirement and its own `.env` load.
#[test]
fn desktop_entry_point_samples_before_it_loads_dotenv() {
    let lib_rs = include_str!("../../../dashboard/src-tauri/src/lib.rs");
    let sample = lib_rs
        .find("apikey::note_env_key_before_dotenv()")
        .expect("the desktop boot must sample the environment before loading .env");
    let load = lib_rs
        .find("dotenvy::dotenv()")
        .expect("the desktop boot must load .env");
    assert!(sample < load, "the sample must come before the .env load");
}

/// The kernel's boot carries the sample into the state the rotation handler
/// reads. Without this line every rotation reports itself as durable — and
/// no handler test can tell, because they set the field themselves.
#[test]
fn kernel_boot_carries_the_sample_into_app_state() {
    let lib_rs = include_str!("../src/lib.rs");
    assert!(
        lib_rs.contains("admin_key_from_env: AtomicBool::new(crate::apikey::env_key_at_boot())"),
        "run_kernel must seed AppState.admin_key_from_env from the boot sample"
    );
}

/// The headless boot reads the file the rotation writes.
///
/// `apikey::resolve_env_target` falls back to `data_dir/.env` when no other
/// `.env` exists, so without a matching read a rotation on a headless install
/// persists into a file the next boot never opens — the same silent loss as
/// the environment case, reached by a different route. This is a structural
/// assertion: it pins that the read side exists, which no runtime test in
/// this crate can observe (the load happens in the binary's entry point,
/// before anything testable is constructed).
#[test]
fn headless_entry_point_reads_the_file_a_rotation_writes() {
    let main_rs = include_str!("../src/main.rs");
    assert!(
        main_rs.contains("dotenvy::from_path(cloto_core::config::data_dir().join(\".env\"))"),
        "the headless boot must also load data_dir/.env, which is where \
         apikey::resolve_env_target writes when no other .env exists"
    );
}
