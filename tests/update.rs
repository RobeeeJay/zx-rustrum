//! Telling somebody a newer release is out: the reply GitHub's releases API
//! gives, read the way PurpleCAD reads it, and the notice it puts up. Nothing
//! here reaches the network; the launch check is only started by `main`.

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use zx_rustrum::machine::Spectrum;
use zx_rustrum::ui::{App, Roms};
use zx_rustrum::update::{is_newer, parse_version, read_release, Update};

fn release(tag: &str, draft: bool, prerelease: bool) -> String {
    format!(
        r#"{{"tag_name": "{tag}", "draft": {draft}, "prerelease": {prerelease},
            "html_url": "https://github.com/RobeeeJay/zx-rustrum/releases/tag/{tag}",
            "assets": [{{"name": "x"}}]}}"#
    )
}

// Versions are compared part by part as numbers: compared as text, 1.0.10
// would sort before 1.0.9 and a real update would be missed.
#[test]
fn versions_are_compared_as_numbers_part_by_part() {
    assert!(is_newer("v1.0.10", "1.0.9"), "1.0.10 against 1.0.9");
    assert!(is_newer("v1.5.0", "1.4.0"));
    assert!(is_newer("2", "1.9.9"), "a version with fewer parts");
    assert!(
        !is_newer("v1.4.0", "1.4.0"),
        "the same version is not newer"
    );
    assert!(!is_newer("v1.3.0", "1.4.0"), "an older one is not newer");
    assert!(!is_newer("nightly", "1.4.0"), "a tag that isn't a version");
    assert_eq!(parse_version("v1.4.0"), Some(vec![1, 4, 0]));
    assert_eq!(parse_version("1.4.0-beta"), None);
}

// /releases/latest already leaves drafts and pre-releases out, but a reply is
// read as if it might not have: only a published release newer than the one
// running is offered, with the page it was published on.
#[test]
fn only_a_newer_published_release_is_offered() {
    assert_eq!(
        read_release(&release("v1.5.0", false, false), "1.4.0"),
        Some(Update {
            version: "1.5.0".into(),
            url: "https://github.com/RobeeeJay/zx-rustrum/releases/tag/v1.5.0".into(),
        })
    );
    assert_eq!(
        read_release(&release("v1.5.0", true, false), "1.4.0"),
        None,
        "a draft"
    );
    assert_eq!(
        read_release(&release("v1.5.0", false, true), "1.4.0"),
        None,
        "a pre-release"
    );
    assert_eq!(
        read_release(&release("v1.4.0", false, false), "1.4.0"),
        None,
        "the same"
    );
    assert_eq!(
        read_release("<html>rate limited</html>", "1.4.0"),
        None,
        "not JSON"
    );
    assert_eq!(
        read_release(r#"{"tag_name": "v1.5.0"}"#, "1.4.0"),
        None,
        "no page to send anybody to"
    );
}

fn harness<'a>() -> Harness<'a, App> {
    let roms = Roms {
        rom48: Some(vec![0x00; 0x4000]),
        rom128: Some(vec![0x00; 0x8000]),
        rom_plus3: Some(vec![0x00; 0x10000]),
        rom_zx81: Some(vec![0x00; 0x2000]),
    };
    let mut app = App::with_roms(Spectrum::new(), String::new(), roms, None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    Harness::builder()
        .with_size([1200.0, 900.0])
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app)
}

fn found(version: &str) -> Option<Update> {
    Some(Update {
        version: version.into(),
        url: format!("https://github.com/RobeeeJay/zx-rustrum/releases/tag/v{version}"),
    })
}

// What the check finds is said under the toolbars, with a way to it, a way
// to stop hearing about that version, and a way to close it.
#[test]
fn a_newer_release_is_offered_until_it_is_skipped() {
    let mut h = harness();
    h.state_mut().offer_update(found("9.9.0"));
    h.run_steps(2);
    assert!(
        h.query_by_label("ZX-Rustrum 9.9.0 is available").is_some(),
        "the notice is up"
    );
    h.get_by_label("Skip this version").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("ZX-Rustrum 9.9.0 is available").is_none(),
        "skipping takes the notice down"
    );
    assert_eq!(
        h.state().prefs.get("skipped_update"),
        Some("9.9.0"),
        "and the version skipped is remembered"
    );
    // The same version found again at the next launch says nothing; a later
    // one does.
    h.state_mut().offer_update(found("9.9.0"));
    assert_eq!(h.state().update, None, "the skipped version again");
    h.state_mut().offer_update(found("9.9.1"));
    assert_eq!(h.state().update, found("9.9.1"), "a later one");
}

// Closing it is for now only: nothing is remembered.
#[test]
fn closing_the_notice_remembers_nothing() {
    let mut h = harness();
    h.state_mut().offer_update(found("9.9.0"));
    h.run_steps(2);
    h.get_by_label("×").click();
    h.run_steps(2);
    assert_eq!(h.state().update, None, "closed");
    assert_eq!(
        h.state().prefs.get("skipped_update"),
        None,
        "nothing skipped"
    );
}

// The check is on unless it has been turned off, as in PurpleCAD.
#[test]
fn the_check_is_on_until_it_is_turned_off() {
    let mut h = harness();
    assert!(h.state().checks_for_updates(), "on to begin with");
    h.state_mut().prefs.set("check_for_updates", false);
    assert!(!h.state().checks_for_updates(), "off once turned off");
}
