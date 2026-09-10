//! The mirror of `api_key_env_precedence_test`: when the environment carries
//! no key, a rotation is durable and must say nothing. Separate binary
//! because the boot sample is a `OnceLock` — one answer per process.

#[test]
fn a_key_that_did_not_come_from_the_environment_raises_no_note() {
    std::env::remove_var("CLOTO_API_KEY");
    cloto_core::apikey::note_env_key_before_dotenv();

    assert!(
        !cloto_core::apikey::env_key_at_boot(),
        "no key in the environment must record as not-from-the-environment"
    );
    assert!(
        cloto_core::apikey::rotation_persistence_note(
            cloto_core::apikey::env_key_at_boot(),
            std::path::Path::new("/var/lib/clotocore/.env"),
        )
        .is_none(),
        "a durable rotation must not warn"
    );
}
