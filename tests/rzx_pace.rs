//! How fast a recording plays, and how far into a frame it can be stopped.

use std::path::PathBuf;
use zx_rustrum::machine::Spectrum;
use zx_rustrum::rzx::{Frame, Recording};
use zx_rustrum::ui::{App, Roms, RzxPlayback, RACE_SPEED};

/// An app playing a made-up recording of NOPs: nothing to see, but every frame
/// is a known number of fetches, which is what the pacing is counted in.
fn playing(speed: f32) -> App {
    playing_frames_of(speed, 10_000)
}

fn playing_frames_of(speed: f32, fetches: u16) -> App {
    let mut spec = Spectrum::new();
    spec.bus.rom.iter_mut().for_each(|b| *b = 0x00); // NOPs
    spec.cpu.pc = 0;

    let mut app = App::with_roms(spec, String::new(), Roms::default(), None);
    app.show_ram_map = false;
    app.show_debugger = false;
    app.show_back_buffer = false;
    app.show_tape = false;
    app.speed = speed;
    app.running = true;
    app.rzx = Some(RzxPlayback {
        recording: Recording {
            creator: "test".into(),
            snapshot: None,
            later: Vec::new(),
            // Sixty frames of ten thousand fetches each: a NOP is four
            // T-states, so ten thousand of them is about half a frame's worth
            // of work, and the length hardly matters — what is being measured
            // is how much of one frame a host frame buys.
            frames: (0..60)
                .map(|_| Frame {
                    fetches,
                    inputs: Vec::new(),
                })
                .collect(),
            start_t: 0,
        },
        path: PathBuf::from("test.rzx"),
        frame: 0,
        remaining: 0,
        carry: 0,
        owed: 0.0,
        visited: Default::default(),
        max_speed: false,
    });
    app.spec.bus.playback = Some(zx_rustrum::machine::Playback::default());
    app
}

/// A recording's frame is the video frame, however long it runs.
///
/// A recorded frame can hold more instructions than fit in a frame of the
/// machine's own time — Space Harrier's recording holds about half as many
/// again — and the T-state clock would then end a frame of its own before the
/// recording's boundary did. The screen was painted twice inside one recorded
/// frame, which at five seconds a frame reads as the picture flickering.
#[test]
fn a_recorded_frame_paints_the_screen_once_however_long_it_is() {
    // Twenty thousand NOPs is 80,000 T-states, which is longer than the
    // 69,888 a 48K frame takes.
    let mut app = playing_frames_of(1.0, 20_000);
    assert!(
        20_000 * 4 > app.spec.bus.frame_t(),
        "the point of the test is a recorded frame longer than a video frame"
    );

    for _ in 0..6 {
        let before = app.spec.bus.frame;
        let recorded = app.rzx.as_ref().unwrap().frame;
        while app.rzx.as_ref().map_or(recorded + 1, |r| r.frame) == recorded {
            app.advance(1.0 / 50.0);
        }
        assert_eq!(
            app.spec.bus.frame - before,
            1,
            "one recorded frame should finish one picture, not {}",
            app.spec.bus.frame - before
        );
    }
}

/// Race the Beam runs at five seconds a frame, and a recording has to be able
/// to run at that pace too: playing whole frames at a time means the picture
/// stands still for five seconds and then jumps a frame, which is the one
/// thing the mode exists not to do.
#[test]
fn a_recording_can_be_played_part_of_a_frame_at_a_time() {
    let mut app = playing(RACE_SPEED);

    // A tenth of a second of host time, in sixtieths.
    for _ in 0..6 {
        app.advance(1.0 / 60.0);
    }

    let rzx = app.rzx.as_ref().expect("still playing");
    assert_eq!(rzx.frame, 0, "a tenth of a second is nowhere near a frame");
    assert!(
        rzx.remaining > 0 && rzx.remaining < 10_000,
        "the recording should be part way through its first frame, not at \
         {} fetches of 10000",
        rzx.remaining
    );
    assert!(
        app.spec.bus.tstates > 0,
        "the machine has not run, so there is no beam to watch"
    );
}

/// And the pace is right: five seconds of host time to one frame.
#[test]
fn a_recorded_frame_takes_five_seconds_to_play_while_racing() {
    let mut app = playing(RACE_SPEED);

    // Five seconds of host time.
    for _ in 0..60 * 5 {
        app.advance(1.0 / 60.0);
    }

    let rzx = app.rzx.as_ref().expect("still playing");
    let done = rzx.frame as f32 + (10_000 - rzx.remaining) as f32 / 10_000.0;
    assert!(
        (0.8..1.3).contains(&done),
        "five seconds of racing played {done} frames of the recording"
    );
}

/// Full speed is unchanged: a recording made at fifty frames a second plays at
/// fifty frames a second.
#[test]
fn a_recording_still_plays_at_speed() {
    let mut app = playing(1.0);

    for _ in 0..60 {
        app.advance(1.0 / 60.0);
    }

    let rzx = app.rzx.as_ref().expect("still playing");
    assert!(
        (45..=55).contains(&rzx.frame),
        "a second at full speed played {} frames, not about fifty",
        rzx.frame
    );
}

/// And at full speed a frame is played whole. Splitting one costs nothing in
/// itself, but it leaves the machine part way through a frame between calls,
/// where anything reading the recording's input sees a frame half consumed.
#[test]
fn at_full_speed_a_frame_is_played_whole() {
    let mut app = playing(1.0);

    // Fewer calls than the recording has frames, so it is still playing at
    // the end of them.
    for call in 0..30 {
        app.advance(1.0 / 60.0);
        let rzx = app.rzx.as_ref().expect("still playing");
        assert_eq!(
            rzx.remaining,
            0,
            "call {call} left the recording {} fetches into a frame",
            10_000 - rzx.remaining
        );
    }
}
