//! Reading an attached picture is something the agent does, not something the
//! kernel does on its own.
//!
//! Until 2026-09-21 the chat path called the Vision tool as `Caller::System`,
//! and the capability gate answers `Ok(())` for System before it looks at
//! anything — so no agent could be allowed or refused image reading, and there
//! was no switch anywhere. The avatar path had always called it as the agent.
//! These pin the corrected shape: which caller the call is made as, why a
//! refusal happened, and that an operator's switch is opt-out.

use std::collections::HashMap;

use cloto_core::handlers::system::{vision_opted_out, vision_refusal, VISION_AUTO_ANALYZE_KEY};

const OFF: &str = "this agent is set not to read images";
const NO_SERVER: &str = "no server that can read images is installed";
const NOT_ALLOWED: &str = "this agent is not allowed to read images";

/// Every combination, because each one is a different sentence the person reads
/// and a different thing for them to go and change.
#[test]
fn each_reason_images_go_unread_is_reported_as_itself() {
    let cases = [
        // opted_out, server,        allowed, expected
        (false, Some("vision"), true, None),
        (false, Some("vision"), false, Some(NOT_ALLOWED)),
        (false, None, false, Some(NO_SERVER)),
        (true, Some("vision"), true, Some(OFF)),
        (true, Some("vision"), false, Some(OFF)),
        (true, None, false, Some(OFF)),
    ];
    for (opted_out, server, allowed, want) in cases {
        let got = vision_refusal(opted_out, server, allowed);
        assert_eq!(
            got, want,
            "opted_out={opted_out} server={server:?} allowed={allowed}"
        );
    }
}

/// The only combination that reads an image is the one where all three hold.
/// Written separately from the table so that a table that stopped covering the
/// admitting case could not pass by covering only refusals.
#[test]
fn images_are_read_only_when_nothing_objects() {
    assert_eq!(vision_refusal(false, Some("vision"), true), None);
    assert!(vision_refusal(true, Some("vision"), true).is_some());
    assert!(vision_refusal(false, None, true).is_some());
    assert!(vision_refusal(false, Some("vision"), false).is_some());
}

/// Being switched off is said even when a server is missing: otherwise turning
/// the switch on is met by a second refusal nobody mentioned.
#[test]
fn the_switch_is_reported_before_the_missing_server() {
    assert_eq!(vision_refusal(true, None, false), Some(OFF));
}

#[test]
fn an_agent_nobody_configured_reads_images() {
    assert!(!vision_opted_out(&HashMap::new()));
}

#[test]
fn only_off_turns_it_off_and_case_and_padding_do_not_matter() {
    for value in ["off", "OFF", "Off", "  off  "] {
        let mut m = HashMap::new();
        m.insert(VISION_AUTO_ANALYZE_KEY.to_string(), value.to_string());
        assert!(vision_opted_out(&m), "{value:?} should turn it off");
    }
}

/// A value nobody recognises leaves it on. A typo in metadata must not silently
/// disable something the settings panel still shows as enabled — the failure
/// would look like the feature being broken rather than being switched off.
#[test]
fn an_unrecognised_value_leaves_it_on() {
    for value in ["on", "", "false", "0", "no", "disabled", "ofF ff"] {
        let mut m = HashMap::new();
        m.insert(VISION_AUTO_ANALYZE_KEY.to_string(), value.to_string());
        assert!(!vision_opted_out(&m), "{value:?} should leave it on");
    }
}

/// The call is made as the agent.
///
/// There is no way to ask the running kernel which caller a private async
/// function passed, so this reads the source the way the route-registration test
/// does. It is scoped to this one function: `Caller::System` is correct
/// elsewhere in the file, including in the audio path next to it.
#[test]
fn the_image_call_is_made_as_the_agent_not_as_the_kernel() {
    let src = include_str!("../../src/handlers/system.rs");
    let start = src
        .find("async fn maybe_analyze_images")
        .expect("maybe_analyze_images is gone; this test no longer guards anything");
    let end = src[start..]
        .find("async fn maybe_transcribe_audio")
        .expect("maybe_transcribe_audio is gone; the slice below would run to the end of the file")
        + start;
    let body = &src[start..end];

    // Validate the slice before trusting what is or is not in it: an empty or
    // suspiciously small body would make both assertions below pass for free.
    assert!(
        body.len() > 1000,
        "the extracted body is {} bytes, too small to be the function",
        body.len()
    );
    assert!(
        body.contains("Caller::Agent("),
        "the image call does not name the agent as its caller"
    );
    assert!(
        !body.contains("Caller::System"),
        "the image call still runs as the kernel, which skips the capability gate"
    );
}
