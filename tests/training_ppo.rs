//! Training a network, on the processor with burn's ndarray backend, so it
//! runs anywhere the suite does.
//!
//! The game: fire scores a point, and while fire is held a solid bar lies
//! across the top of the screen; holding fire scores nothing more, and the
//! bar goes when fire is let go. The best play alternates — fire, then
//! anything else — for half a point a step. Random play gets about a seventh.
//!
//! What makes this a test of seeing: play that ignores the screen can only
//! press fire with some probability p, earning p(1 - p), which is at most a
//! quarter of a point a step. Beating that needs the bar.
//!
//! The first version of this game had only an 8-pixel mark drawn in a new
//! place for each point, and the network learnt to press fire half the time —
//! 0.234 a step, just under the quarter — and never used the screen: a mark
//! that small, averaged down fourfold and never in the same place, was not
//! something it found.
//!
//! The bar alone was not enough at first either: with the default discount of
//! 0.99 the network reached only 0.23 in 61 updates. Probing the fire
//! probability for a screen with the bar and one without showed it did learn to
//! tell them apart — 0.08 against 0.88 — but not until about the 140th update.
//! Each advantage at 0.99 carries the next twenty-odd steps of other random
//! choices, which drowns a consequence that here lasts a single step. At 0.9 it
//! reaches half a point a step by about the 25th update, on two seeds. Raising
//! the learning rate did nothing, and turning the value loss down tenfold only
//! a little. The default stays at 0.99, which is right for a game whose rewards
//! come long after the move that earned them.

use zx_rustrum::joystick::Kind;
use zx_rustrum::machine::Spectrum;
use zx_rustrum::training::env::Config;
use zx_rustrum::training::inputs::InputSet;
use zx_rustrum::training::judge::{Judge, Number};
use zx_rustrum::training::ppo::{PpoConfig, Trainer};
use zx_rustrum::training::setup::Setup;
use zx_rustrum::training::sight::{Area, Sight};
use zx_rustrum::training::worker::{Processor, Shared, Worker, NETWORK, OPTIMISER, SETUP, START};

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use zx_rustrum::machine::FRAME_T;
use zx_rustrum::training::env::Env;
use zx_rustrum::training::player::Player;
use zx_rustrum::training::worker::keep;
use zx_rustrum::z80::Bus;

type B = burn::backend::Autodiff<burn::backend::NdArray>;

fn tiny_game() -> Spectrum {
    let mut spec = Spectrum::new();
    let code: &[(u16, &[u8])] = &[
        (0x8000, &[0xF3]),             // DI
        (0x8001, &[0xDB, 0x1F]),       // loop: IN A,($1F)
        (0x8003, &[0xE6, 0x10]),       // AND $10
        (0x8005, &[0x28, 0xFA]),       // JR Z,loop
        (0x8007, &[0x21, 0x00, 0x90]), // LD HL,$9000
        (0x800A, &[0x34]),             // INC (HL)
        (0x800B, &[0x3E, 0xFF]),       // LD A,$FF
        (0x800D, &[0xCD, 0x20, 0x80]), // CALL bar
        (0x8010, &[0xDB, 0x1F]),       // wait: IN A,($1F)
        (0x8012, &[0xE6, 0x10]),       // AND $10
        (0x8014, &[0x20, 0xFA]),       // JR NZ,wait
        (0x8016, &[0xAF]),             // XOR A
        (0x8017, &[0xCD, 0x20, 0x80]), // CALL bar
        (0x801A, &[0x18, 0xE5]),       // JR loop
        // bar: the top character row, all eight pixel rows, filled with A.
        (0x8020, &[0x21, 0x00, 0x40]), // LD HL,$4000
        (0x8023, &[0x0E, 0x08]),       // LD C,8
        (0x8025, &[0x06, 0x20]),       // row: LD B,32
        (0x8027, &[0xE5]),             // PUSH HL
        (0x8028, &[0x77]),             // cell: LD (HL),A
        (0x8029, &[0x23]),             // INC HL
        (0x802A, &[0x10, 0xFC]),       // DJNZ cell
        (0x802C, &[0xE1]),             // POP HL
        (0x802D, &[0x24]),             // INC H
        (0x802E, &[0x0D]),             // DEC C
        (0x802F, &[0x20, 0xF4]),       // JR NZ,row
        (0x8031, &[0xC9]),             // RET
    ];
    for (at, bytes) in code {
        for (i, b) in bytes.iter().enumerate() {
            spec.bus.poke(at + i as u16, *b);
        }
    }
    for addr in 0x4000..0x5B00u16 {
        spec.bus.poke(addr, if addr >= 0x5800 { 0x38 } else { 0 });
    }
    spec.bus.poke(0x9000, 0);
    spec.cpu.pc = 0x8000;
    spec.cpu.sp = 0x7F00;
    spec
}

fn env_config() -> Config {
    Config {
        inputs: InputSet::joystick(Kind::Kempston, false, false),
        sight: Sight {
            area: Area::Display,
            shrink: 4,
            colour: false,
            frames: 2,
        },
        judge: Judge {
            score: Some(Number::Byte(0x9000)),
            max_steps: 32,
            ..Judge::default()
        },
        frames_per_step: 2,
        random_wait: 3,
    }
}

fn ppo_config() -> PpoConfig {
    PpoConfig {
        games: 8,
        steps: 32,
        learning_rate: 1e-3,
        gamma: 0.9,
        epochs: 4,
        minibatch: 64,
        ..PpoConfig::default()
    }
}

/// What training the tiny game once came to, for the tests that need a
/// trained network: training takes seconds, and doing it twice would prove
/// nothing more.
struct Learnt {
    first: f32,
    last: f32,
    updates: usize,
    steps: u64,
    dir: std::path::PathBuf,
    /// A screen with the bar on it, and what the trainer made of it.
    bar: Vec<u8>,
    bar_probabilities: Vec<f32>,
}

fn learnt() -> &'static Learnt {
    static LEARNT: OnceLock<Learnt> = OnceLock::new();
    LEARNT.get_or_init(|| {
        let mut trainer =
            Trainer::<B>::new(&tiny_game(), env_config(), ppo_config(), Default::default())
                .unwrap();
        let first = trainer.update().step_reward;
        let mut last = first;
        for _ in 0..60 {
            last = trainer.update().step_reward;
            if last >= 0.45 {
                break;
            }
        }
        let dir = std::env::temp_dir().join(format!("zxrs-learnt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let setup = Setup {
            env: env_config(),
            ppo: ppo_config(),
        };
        keep(&trainer, &setup, &tiny_game(), &dir).unwrap();
        let mut env = Env::new(Arc::new(tiny_game()), Arc::new(env_config()), 1);
        let bar = env.step(FIRE).observation;
        let bar_probabilities = trainer.probabilities(&bar);
        let p = trainer.progress();
        Learnt {
            first,
            last,
            updates: p.updates,
            steps: p.steps,
            dir,
            bar,
            bar_probabilities,
        }
    })
}

/// Fire, in the Kempston set with no diagonals and no fire while moving.
const FIRE: usize = 5;

/// Trained from nothing but the screen and the judge's reward, the network
/// learns to alternate: 0.45 a step, which play blind to the screen cannot
/// reach, and at least twice where it began.
#[test]
fn a_network_learns_the_tiny_game_from_the_screen() {
    let l = learnt();
    eprintln!(
        "started at {:.3} a step, reached {:.3} after {} updates, {} steps",
        l.first, l.last, l.updates, l.steps
    );
    assert!(
        l.last >= 0.45 && l.last >= 2.0 * l.first,
        "from {:.3} to {:.3} a step: the network did not learn to alternate",
        l.first,
        l.last
    );
}

/// A kept network loaded to play rather than to train prefers what the
/// trainer preferred, plays a machine it has never seen far better than
/// play blind to the screen could, and lets go when told.
#[test]
fn a_kept_network_plays_a_machine_of_its_own() {
    let l = learnt();
    let mut player = Player::load(&l.dir).unwrap();
    let probs = player.probabilities(&l.bar);
    assert!(
        probs
            .iter()
            .zip(&l.bar_probabilities)
            .all(|(a, b)| (a - b).abs() < 1e-5),
        "the player's {probs:?} against the trainer's {:?}",
        l.bar_probabilities
    );

    let mut spec = tiny_game();
    player.prepare(&mut spec);
    let steps = 64;
    for _ in 0..steps * env_config().frames_per_step {
        player.play(&mut spec);
        spec.run(FRAME_T);
    }
    let score = spec.bus.peek_raw(0x9000);
    let per_step = score as f32 / steps as f32;
    eprintln!("played {score} points in {steps} steps, {per_step:.3} a step");
    assert!(
        per_step >= 0.35,
        "{score} points in {steps} steps, {per_step:.3} a step: blind play gets at most \
         0.25, and training reached {:.3}",
        l.last
    );
    player.release(&mut spec);
    assert_eq!(spec.bus.io_read(0x001F), 0, "and let go");
}

/// A network without the set-up it was trained with cannot be rebuilt, and
/// is refused with the name of what is missing.
#[test]
fn a_network_without_its_set_up_is_refused() {
    let dir = std::env::temp_dir().join(format!("zxrs-bare-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = Player::load(&dir).err().expect("refused");
    assert!(err.contains("setup.txt"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A network kept in a file and put back chooses exactly as it did.
#[test]
fn a_saved_network_comes_back_the_same() {
    let mut trainer =
        Trainer::<B>::new(&tiny_game(), env_config(), ppo_config(), Default::default()).unwrap();
    trainer.update();
    let observation = trainer.pool().game(0).observation();
    let before = trainer.probabilities(&observation);

    let dir = std::env::temp_dir().join(format!("zxrs-net-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("net");
    trainer.save(&path).unwrap();

    let mut fresh =
        Trainer::<B>::new(&tiny_game(), env_config(), ppo_config(), Default::default()).unwrap();
    assert_ne!(
        fresh.probabilities(&observation),
        before,
        "a fresh network differs"
    );
    fresh.load(&path).unwrap();
    assert_eq!(
        fresh.probabilities(&observation),
        before,
        "and the loaded one does not"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A picture too small for the network is refused with the reason.
#[test]
fn a_picture_too_small_is_refused() {
    let mut env = env_config();
    env.sight.shrink = 8;
    let result = Trainer::<B>::new(&tiny_game(), env, ppo_config(), Default::default());
    let err = result.err().expect("refused");
    assert!(err.contains("too small"), "{err}");
}

fn worker_start(
    env: Config,
    ppo: PpoConfig,
    keep_in: Option<std::path::PathBuf>,
) -> zx_rustrum::training::worker::Start {
    zx_rustrum::training::worker::Start {
        machine: tiny_game(),
        setup: Setup { env, ppo },
        processor: Processor::Cpu,
        resume: None,
        keep_in,
        updates: None,
    }
}

/// Wait for something the worker writes down, or fail saying what never came.
fn wait_for(worker: &Worker, what: &str, done: impl Fn(&Shared) -> bool) {
    let start = Instant::now();
    while !done(&worker.shared()) {
        if let Some(e) = &worker.shared().error {
            panic!("waiting for {what}, the worker stopped: {e}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(300),
            "{what} never came"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Training on a thread of its own writes down every update and a picture of
/// a game as it goes, and when stopped keeps the network beside the set-up
/// it was trained with — which is what it takes to load it again.
#[test]
fn a_worker_trains_shows_a_game_and_keeps_the_network() {
    let dir = std::env::temp_dir().join(format!("zxrs-worker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut worker = Worker::start(worker_start(env_config(), ppo_config(), Some(dir.clone())));
    wait_for(&worker, "two updates and a picture", |s| {
        s.history.len() >= 2 && s.preview.is_some()
    });
    worker.stop();
    assert!(!worker.is_running());

    let shared = worker.shared();
    let preview = shared.preview.as_ref().unwrap();
    assert_eq!(
        preview.picture.len(),
        preview.width * preview.height * 4,
        "an RGBA picture"
    );
    assert_eq!(preview.probabilities.len(), 6, "one for each action");
    // And what the network was shown for that step, so the window can draw
    // the frames it stacked.
    assert_eq!(preview.sight, env_config().sight);
    assert_eq!(
        preview.seen.len(),
        preview.sight.len(),
        "the frames stacked as the network sees them"
    );
    let sum: f32 = preview.probabilities.iter().sum();
    assert!((sum - 1.0).abs() < 1e-4, "probabilities sum to {sum}");
    assert!(
        shared
            .history
            .windows(2)
            .all(|w| w[1].updates == w[0].updates + 1),
        "every update written down, in order"
    );
    assert_eq!(shared.saved.as_deref(), Some(dir.as_path()));

    let text = std::fs::read_to_string(dir.join(SETUP)).expect("the set-up is kept");
    let setup = Setup::from_text(&text).unwrap();
    assert_eq!(
        setup,
        Setup {
            env: env_config(),
            ppo: ppo_config()
        }
    );
    let mut again =
        Trainer::<B>::new(&tiny_game(), setup.env, setup.ppo, Default::default()).unwrap();
    again
        .load(&dir.join(NETWORK))
        .expect("the kept network loads");

    // And the machine every game started from, so the next run can start
    // there too.
    let mut back = Spectrum::new();
    let start = std::fs::read(dir.join(START)).expect("the start is kept");
    zx_rustrum::szx::load(&mut back, &start).unwrap();
    let game = tiny_game();
    assert_eq!(back.cpu.pc, game.cpu.pc, "the start's registers");
    assert!(
        (0x8000..0x8040u16).all(|a| back.bus.peek_raw(a) == game.bus.peek_raw(a)),
        "and its program"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Stop is answered within a step, not at the end of an update — at the
/// default size an update on the processor takes most of a minute — and an
/// update cut short is neither counted nor kept.
#[test]
fn stopping_cuts_an_update_short() {
    let ppo = PpoConfig {
        steps: 1_000_000,
        ..ppo_config()
    };
    let dir = std::env::temp_dir().join(format!("zxrs-stop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut worker = Worker::start(worker_start(env_config(), ppo, Some(dir.clone())));
    wait_for(&worker, "the first picture", |s| s.preview.is_some());
    // Stopped from another thread, so a stop that is never answered fails
    // the test rather than hanging it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        worker.stop();
        let _ = tx.send(worker);
    });
    let worker = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("stopped within two seconds");
    assert!(worker.shared().history.is_empty(), "nothing counted");
    assert!(!dir.exists(), "and nothing kept");
}

/// A set-up the network cannot be built for is said in the window, and the
/// worker stops, rather than the emulator falling over.
#[test]
fn a_worker_that_cannot_start_says_why() {
    let mut env = env_config();
    env.sight.shrink = 8;
    let worker = Worker::start(worker_start(env, ppo_config(), None));
    let start = Instant::now();
    while worker.is_running() && start.elapsed() < Duration::from_secs(30) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let error = worker.shared().error.clone().expect("an error");
    assert!(error.contains("too small"), "{error}");
}

/// A reset or a snapshot puts the machine's frame count back, and the player
/// chooses again at once rather than holding its last choice until the count
/// catches up with where it was.
#[test]
fn a_player_chooses_again_when_the_clock_goes_back() {
    let mut player = Player::load(&learnt().dir).unwrap();
    let mut spec = tiny_game();
    player.prepare(&mut spec);
    for _ in 0..3 {
        spec.run(FRAME_T);
    }
    assert!(player.play(&mut spec).is_some(), "a first choice");
    spec.run(FRAME_T);
    assert!(
        player.play(&mut spec).is_none(),
        "a step is two frames, and only one has gone"
    );
    spec.bus.frame = 0;
    assert!(
        player.play(&mut spec).is_some(),
        "the clock went back and it chose again"
    );
}

/// Going on from a kept network carries the optimiser's state with it. Three
/// copies of the same kept network each take one update: two without the
/// state come out the same to within rounding — so a difference is not
/// noise — and the one with it comes out different by far more, because
/// Adam's steps depend on what it had worked out before.
#[test]
fn going_on_carries_the_optimisers_state() {
    let setup = Setup {
        env: env_config(),
        ppo: ppo_config(),
    };
    let mut first = Trainer::<B>::new(
        &tiny_game(),
        setup.env.clone(),
        setup.ppo.clone(),
        Default::default(),
    )
    .unwrap();
    first.update();
    first.update();
    let dir = std::env::temp_dir().join(format!("zxrs-optimiser-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    keep(&first, &setup, &tiny_game(), &dir).unwrap();

    let again = || {
        let mut t = Trainer::<B>::new(
            &tiny_game(),
            setup.env.clone(),
            setup.ppo.clone(),
            Default::default(),
        )
        .unwrap();
        t.load(&dir.join(NETWORK)).unwrap();
        t
    };
    let (mut without, mut also_without, mut with) = (again(), again(), again());
    with.load_optimiser(&dir.join(OPTIMISER)).unwrap();
    for t in [&mut without, &mut also_without, &mut with] {
        t.update();
    }
    let mut env = Env::new(Arc::new(tiny_game()), Arc::new(env_config()), 1);
    let seen = env.step(FIRE).observation;
    let (a, b, c) = (
        without.probabilities(&seen),
        also_without.probabilities(&seen),
        with.probabilities(&seen),
    );
    // Not bit for bit: threads add things up in an order that depends on the
    // processor, and GitHub's x86 runners came out 1e-7 apart where this Mac
    // matched exactly. The state's own effect measured 1e-2 to 2.5e-2, so
    // there is room between the two for each bound to mean something.
    let gap = |x: &[f32], y: &[f32]| {
        x.iter()
            .zip(y)
            .map(|(p, q)| (p - q).abs())
            .fold(0f32, f32::max)
    };
    let (noise, effect) = (gap(&a, &b), gap(&a, &c));
    assert!(
        noise < 1e-5,
        "two runs going on without it agree to within rounding: {noise:e} apart, {a:?} and {b:?}"
    );
    assert!(
        effect > 1e-3,
        "and the one with the optimiser's state went its own way: only {effect:e} apart, \
         against {noise:e} between the two without"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A worker told to stop after two updates does so by itself, and each update
/// says how long it took — which is how Time it measures one.
#[test]
fn a_worker_stops_itself_after_the_updates_it_was_given() {
    let mut start = worker_start(env_config(), ppo_config(), None);
    start.updates = Some(2);
    let worker = Worker::start(start);
    let began = Instant::now();
    while worker.is_running() {
        assert!(
            began.elapsed() < Duration::from_secs(60),
            "still running after {:?}, with {} updates done",
            began.elapsed(),
            worker.shared().history.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let shared = worker.shared();
    assert_eq!(shared.history.len(), 2, "two updates, no more");
    assert!(
        shared.history.iter().all(|p| p.seconds > 0.0),
        "each timed: {:?}",
        shared.history.iter().map(|p| p.seconds).collect::<Vec<_>>()
    );
}
