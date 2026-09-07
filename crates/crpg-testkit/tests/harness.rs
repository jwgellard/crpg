//! Wiring tests for the T008b hash harness: determinism, the 10,000-tick
//! golden round trip, tamper detection with exact-tick reports, and two
//! script shapes through the one fixed interleaving. All deterministic;
//! no proptest needed — these pin wiring, not distributions.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use crpg_core::EntityId;
use crpg_sim::{state_hash, tick, InitiativeKey, Transform, World};
use crpg_testkit::{
    run_hash_sequence, verify_golden, write_golden, HarnessError, Mismatch, ScriptStep,
};

/// Spawns up to `cap` entities, then rewrites every live transform per tick.
struct SpawnHeavy {
    cap: usize,
    live: Vec<EntityId>,
}

impl SpawnHeavy {
    fn step(&mut self, world: &mut World) {
        if self.live.len() < self.cap {
            for _ in 0..3 {
                if self.live.len() >= self.cap {
                    break;
                }
                self.live.push(world.spawn(crpg_sim::EntityMeta {}));
            }
        }
        for (i, id) in self.live.iter().enumerate() {
            if world.contains(*id) {
                let v = i as f32;
                world.transforms_mut().insert(
                    *id,
                    Transform {
                        position: [v, -v, v / 2.0],
                        velocity: [0.0, 1.0, 0.0],
                    },
                );
            }
        }
    }
}

/// Holds ~10 live entities: spawns, rotating timeline keys, RNG draws, and a
/// despawn every fifth tick.
struct TimelineRng {
    live: Vec<EntityId>,
    tick: usize,
}

impl TimelineRng {
    fn step(&mut self, world: &mut World) {
        self.tick += 1;
        if self.live.len() < 10 {
            let id = world.spawn(crpg_sim::EntityMeta {});
            self.live.push(id);
        }
        for (i, id) in self.live.iter().enumerate() {
            if world.contains(*id) {
                world
                    .timeline_mut()
                    .insert(InitiativeKey((i as i32 + self.tick as i32) % 5), *id);
            }
        }
        let _ = world.rng_mut().stream("script").next_u32();
        if self.tick.is_multiple_of(5) {
            if let Some(victim) = self.live.iter().find(|id| world.contains(**id)).copied() {
                world.despawn(victim);
            }
        }
        self.live.retain(|id| world.contains(*id));
    }
}

fn spawn_heavy_script(cap: usize) -> ScriptStep {
    let mut script = SpawnHeavy {
        cap,
        live: Vec::new(),
    };
    Box::new(move |world: &mut World| script.step(world))
}

fn timeline_rng_script() -> ScriptStep {
    let mut script = TimelineRng {
        live: Vec::new(),
        tick: 0,
    };
    Box::new(move |world: &mut World| script.step(world))
}

fn temp_golden(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("crpg-testkit-{}-{name}.golden", std::process::id()));
    path
}

fn remove_silently(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

#[test]
fn same_seed_and_script_replay_identically() {
    let first = run_hash_sequence(0xC0FFEE, 500, spawn_heavy_script(60));
    let second = run_hash_sequence(0xC0FFEE, 500, spawn_heavy_script(60));
    assert_eq!(first, second);
    let third = run_hash_sequence(0xC0FFEE, 500, timeline_rng_script());
    let fourth = run_hash_sequence(0xC0FFEE, 500, timeline_rng_script());
    assert_eq!(third, fourth);
}

#[test]
fn golden_round_trip_over_10k_ticks() {
    let path = temp_golden("roundtrip");
    remove_silently(&path);
    let hashes = run_hash_sequence(0xBEEF, 10_000, spawn_heavy_script(60));
    write_golden(&path, &hashes).unwrap();
    verify_golden(&path, &hashes).unwrap();
    remove_silently(&path);
}

fn hex_of(hash: &[u8; 32]) -> String {
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn tamper_reports_the_exact_tick() {
    let path = temp_golden("tamper");
    remove_silently(&path);
    let hashes = run_hash_sequence(0xFACE, 200, timeline_rng_script());
    write_golden(&path, &hashes).unwrap();

    // Flip one hex char in the hash at index 40.
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let target = 2 + 40; // two header lines, then hash lines.
    let mut chars: Vec<char> = lines[target].chars().collect();
    chars[0] = if chars[0] == '0' { '1' } else { '0' };
    lines[target] = chars.into_iter().collect();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    match verify_golden(&path, &hashes) {
        Err(HarnessError::Mismatch(Mismatch::Diverged {
            tick,
            expected,
            actual,
        })) => {
            assert_eq!(tick, 40);
            assert_eq!(actual, hashes[40]);
            assert_ne!(expected, actual);
            assert_eq!(
                Mismatch::Diverged {
                    tick,
                    expected,
                    actual
                }
                .to_string(),
                format!(
                    "diverged at tick 40: expected {}, got {}",
                    hex_of(&expected),
                    hex_of(&hashes[40]),
                )
            );
        }
        other => panic!("expected a tick-40 divergence, got {other:?}"),
    }

    // Truncate a line: malformed content reports the exact tick with the
    // produced hash and the raw line, not a zeroed expected hash.
    write_golden(&path, &hashes).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let target = 2 + 40;
    lines[target].truncate(10);
    let bad_line = lines[target].clone();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    match verify_golden(&path, &hashes) {
        Err(HarnessError::Mismatch(Mismatch::Malformed { tick, actual, line })) => {
            assert_eq!(tick, 40);
            assert_eq!(actual, Some(hashes[40]));
            assert_eq!(line, bad_line);
        }
        other => panic!("expected a tick-40 malformed line, got {other:?}"),
    }

    // Run outlived the file: the produced hash is reported, no expected side.
    let hashes_plus = run_hash_sequence(0xFACE, 200, timeline_rng_script());
    write_golden(&path, &hashes_plus).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    std::fs::write(&path, lines[..lines.len() - 5].join("\n") + "\n").unwrap();
    match verify_golden(&path, &hashes_plus) {
        Err(HarnessError::Mismatch(Mismatch::GoldenShort { tick, actual })) => {
            assert_eq!(tick, 195);
            assert_eq!(actual, hashes_plus[195]);
        }
        other => panic!("expected a tick-195 golden-short, got {other:?}"),
    }

    // File outlived the run: the approved hash is reported, no produced side.
    write_golden(&path, &hashes_plus).unwrap();
    match verify_golden(&path, &hashes_plus[..195]) {
        Err(HarnessError::Mismatch(Mismatch::RunShort { tick, expected })) => {
            assert_eq!(tick, 195);
            assert_eq!(expected, hashes_plus[195]);
        }
        other => panic!("expected a tick-195 run-short, got {other:?}"),
    }

    // Reworded headers still verify: headers never compare.
    write_golden(&path, &hashes_plus).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let reworded = text.replace("# scope:", "# whatever:");
    std::fs::write(&path, reworded).unwrap();
    verify_golden(&path, &hashes_plus).unwrap();

    // Readable non-UTF-8 is content divergence, not an I/O failure.
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0x2a]).unwrap();
    match verify_golden(&path, &hashes_plus) {
        Err(HarnessError::Mismatch(Mismatch::InvalidUtf8)) => {}
        other => panic!("expected invalid-UTF-8 mismatch, got {other:?}"),
    }

    // A missing file is an I/O failure, not a mismatch.
    remove_silently(&path);
    match verify_golden(&path, &hashes_plus) {
        Err(HarnessError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
        other => panic!("expected a NotFound io error, got {other:?}"),
    }
    remove_silently(&path);
}

#[test]
fn golden_format_is_exact_and_headers_never_compare() {
    let path = temp_golden("format");
    remove_silently(&path);
    let hashes = run_hash_sequence(0xA11CE, 8, spawn_heavy_script(6));
    write_golden(&path, &hashes).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.ends_with('\n'));
    let mut lines = text.lines();
    let scope = lines.next().unwrap();
    assert!(scope.starts_with("# scope: "));
    assert!(scope.contains(std::env::consts::OS));
    assert!(scope.contains(std::env::consts::ARCH));
    assert_eq!(lines.next().unwrap(), "# generator: crpg-testkit");
    let hash_lines: Vec<&str> = lines.collect();
    assert_eq!(hash_lines.len(), hashes.len());
    for (line, hash) in hash_lines.iter().zip(hashes.iter()) {
        assert_eq!(line.len(), 64);
        assert_eq!(*line, hex_of(hash));
    }

    // Nested parents are created by the writer.
    let nested = std::env::temp_dir().join(format!(
        "crpg-testkit-{}-nested/a/b/c.golden",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&nested);
    write_golden(&nested, &hashes).unwrap();
    verify_golden(&nested, &hashes).unwrap();
    let _ = std::fs::remove_file(&nested);
    remove_silently(&path);
}

#[test]
fn harness_pins_script_tick_hash_order() {
    // A spawning script observes different event ticks depending on whether
    // the script runs before `tick`: script-first stamps spawns at the prior
    // tick, tick-first at the new tick. The harness must match script-first.
    fn script_first(seed: u64, ticks: usize) -> Vec<[u8; 32]> {
        let mut world = World::new(seed);
        let mut out = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            world.spawn(crpg_sim::EntityMeta {});
            tick(&mut world);
            out.push(state_hash(&world));
        }
        out
    }
    fn tick_first(seed: u64, ticks: usize) -> Vec<[u8; 32]> {
        let mut world = World::new(seed);
        let mut out = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            tick(&mut world);
            world.spawn(crpg_sim::EntityMeta {});
            out.push(state_hash(&world));
        }
        out
    }
    let seed = 0x0BDE;
    let ticks = 32;
    let harness = run_hash_sequence(
        seed,
        ticks,
        Box::new(|world: &mut World| {
            world.spawn(crpg_sim::EntityMeta {});
        }),
    );
    assert_eq!(harness, script_first(seed, ticks));
    assert_ne!(harness, tick_first(seed, ticks));
}

#[test]
fn both_script_shapes_verify_end_to_end() {
    for (name, script) in [
        ("spawn", spawn_heavy_script(25)),
        ("timeline", timeline_rng_script()),
    ] {
        let path = temp_golden(name);
        remove_silently(&path);
        let hashes = run_hash_sequence(0x5EED, 1_000, script);
        write_golden(&path, &hashes).unwrap();
        verify_golden(&path, &hashes).unwrap();
        // Cross-check: each shape rejects the other's golden.
        let other = run_hash_sequence(0x5EED, 1_000, spawn_heavy_script(3));
        assert!(matches!(
            verify_golden(&path, &other),
            Err(HarnessError::Mismatch(_))
        ));
        remove_silently(&path);
    }
}
