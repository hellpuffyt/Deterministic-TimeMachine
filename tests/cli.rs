//! End-to-end: the `tm` binary driving the example programs as child
//! processes, exactly as a user would.
#![cfg(feature = "cli")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

fn tm() -> &'static str {
    env!("CARGO_BIN_EXE_tm")
}

/// Path of an example binary, building the examples once if needed (they
/// are not built when only this test target is selected).
fn example(name: &str) -> PathBuf {
    static BUILT: OnceLock<()> = OnceLock::new();
    let dir = Path::new(tm()).parent().unwrap().join("examples");
    let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    BUILT.get_or_init(|| {
        if !path.exists() {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let status = Command::new(cargo)
                .args([
                    "build",
                    "--examples",
                    "--manifest-path",
                    concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
                ])
                .status()
                .unwrap();
            assert!(status.success(), "building examples failed");
        }
    });
    assert!(path.exists(), "missing example binary {}", path.display());
    path
}

fn run(args: &[&str], envs: &[(&str, &str)], cwd: &Path) -> Output {
    let mut c = Command::new(tm());
    c.args(args).current_dir(cwd);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn hunt_flaky_cache(dir: &Path) -> PathBuf {
    let ex = example("flaky_cache");
    let o = run(
        &["hunt", "--sources", "sim", "--", ex.to_str().unwrap()],
        &[],
        dir,
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains("found a failure on run"), "{}", text(&o));
    let saved: Vec<_> = std::fs::read_dir(dir.join("tm-out"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(saved.len(), 1);
    saved[0].clone()
}

#[test]
fn hunt_then_replay_reproduces_every_time() {
    let dir = tempfile::tempdir().unwrap();
    let trace = hunt_flaky_cache(dir.path());
    let ex = example("flaky_cache");
    let o = run(
        &[
            "replay",
            trace.to_str().unwrap(),
            "--times",
            "10",
            "--",
            ex.to_str().unwrap(),
        ],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("10 reproduced the recorded outcome"),
        "{}",
        text(&o)
    );
}

#[test]
fn simulated_hunt_is_deterministic() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let ta = std::fs::read_to_string(hunt_flaky_cache(a.path())).unwrap();
    let tb = std::fs::read_to_string(hunt_flaky_cache(b.path())).unwrap();
    assert_eq!(ta, tb);
}

#[test]
fn changed_program_diverges_with_exit_3() {
    let dir = tempfile::tempdir().unwrap();
    let trace = hunt_flaky_cache(dir.path());
    let ex = example("flaky_cache");
    let o = run(
        &[
            "replay",
            trace.to_str().unwrap(),
            "--quiet",
            "--",
            ex.to_str().unwrap(),
        ],
        &[("FLAKY_CACHE_VARIANT", "extra-draw")],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(3), "{}", text(&o));
    let t = text(&o);
    assert!(
        t.contains("DIVERGED")
            && t.contains("expected: #3 clock.now")
            && t.contains("actual:   #3 rng.u64"),
        "{t}"
    );
}

#[test]
fn fixed_program_passes_every_simulated_seed() {
    let dir = tempfile::tempdir().unwrap();
    let ex = example("flaky_cache");
    let o = run(
        &[
            "hunt",
            "--sources",
            "sim",
            "--runs",
            "150",
            "--",
            ex.to_str().unwrap(),
        ],
        &[("FLAKY_CACHE_VARIANT", "fixed")],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("no failure in 150 runs"));
}

#[test]
fn minimize_shrinks_the_race() {
    let dir = tempfile::tempdir().unwrap();
    let ex = example("lost_update");
    let o = run(&["hunt", "--", ex.to_str().unwrap()], &[], dir.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let trace = std::fs::read_dir(dir.path().join("tm-out"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let out = dir.path().join("min.tmtrace");
    let o = run(
        &[
            "minimize",
            trace.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--",
            ex.to_str().unwrap(),
        ],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(
        text(&o).contains("minimized decisions: [1]"),
        "{}",
        text(&o)
    );
    let o = run(
        &[
            "replay",
            out.to_str().unwrap(),
            "--times",
            "3",
            "--",
            ex.to_str().unwrap(),
        ],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
}

#[test]
fn validate_inspect_and_diff() {
    let dir = tempfile::tempdir().unwrap();
    let trace = hunt_flaky_cache(dir.path());
    let o = run(&["validate", trace.to_str().unwrap()], &[], dir.path());
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));

    let full = std::fs::read_to_string(&trace).unwrap();
    let truncated = dir.path().join("cut.tmtrace");
    let keep: Vec<&str> = full.lines().collect();
    std::fs::write(&truncated, keep[..keep.len() - 1].join("\n")).unwrap();
    let o = run(
        &[
            "validate",
            trace.to_str().unwrap(),
            truncated.to_str().unwrap(),
        ],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o).contains("INVALID") && text(&o).contains("truncated"),
        "{}",
        text(&o)
    );

    let o = run(
        &["inspect", trace.to_str().unwrap(), "--kind", "rng.u64"],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0));
    assert!(
        text(&o).contains("sources  sim+custom (seed"),
        "{}",
        text(&o)
    );
    assert!(text(&o).contains("rng.u64 ="), "{}", text(&o));

    let o = run(
        &["diff", trace.to_str().unwrap(), trace.to_str().unwrap()],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(0));
    let edited = dir.path().join("edited.tmtrace");
    let mut t = timemachine::Trace::load(&trace).unwrap();
    t.events.truncate(4);
    t.save(&edited).unwrap();
    let o = run(
        &["diff", trace.to_str().unwrap(), edited.to_str().unwrap()],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        text(&o).contains("first difference at event #4"),
        "{}",
        text(&o)
    );
}

#[test]
fn program_without_timemachine_is_explained() {
    let dir = tempfile::tempdir().unwrap();
    // `tm validate` on a missing file exits 1 and never writes a trace.
    let o = run(
        &[
            "hunt",
            "--runs",
            "2",
            "--",
            tm(),
            "validate",
            "/nonexistent.tmtrace",
        ],
        &[],
        dir.path(),
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("left no usable trace"), "{}", text(&o));
}

#[test]
fn invalid_tm_configuration_is_rejected_by_the_program() {
    let ex = example("lost_update");
    for (envs, needle) in [
        (vec![("TM_MODE", "rewind")], "invalid TM_MODE"),
        (vec![("TM_MODE", "replay")], "TM_TRACE must be set"),
        (
            vec![("TM_MODE", "record"), ("TM_TRACE", "x"), ("TM_SEED", "abc")],
            "invalid TM_SEED",
        ),
        (
            vec![("TM_MODE", "drive"), ("TM_DECISIONS", "/nonexistent.json")],
            "cannot load decisions",
        ),
    ] {
        let mut c = Command::new(&ex);
        for (k, v) in &envs {
            c.env(k, v);
        }
        let o = c.output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{envs:?}: {}", text(&o));
        assert!(text(&o).contains(needle), "{envs:?}: {}", text(&o));
    }
}

#[test]
fn test_filter_leaves_other_contexts_alone() {
    // TM_TEST names a different test: the program runs live and writes nothing.
    let dir = tempfile::tempdir().unwrap();
    let trace = dir.path().join("t.tmtrace");
    let o = Command::new(example("lost_update"))
        .env("TM_MODE", "record")
        .env("TM_TRACE", &trace)
        .env("TM_TEST", "some_other_test")
        .output()
        .unwrap();
    assert!(o.status.code().is_some());
    assert!(!trace.exists());
}

#[test]
fn timeouts_kill_the_child_and_are_reported() {
    // The flaky example really sleeps ~100 ms with real sources; a zero
    // timeout must kill it rather than wait.
    let dir = tempfile::tempdir().unwrap();
    let ex = example("flaky_cache");
    let start = std::time::Instant::now();
    let o = run(
        &[
            "hunt",
            "--sources",
            "real",
            "--runs",
            "2",
            "--timeout",
            "0",
            "--",
            ex.to_str().unwrap(),
        ],
        &[],
        dir.path(),
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("timed out"), "{}", text(&o));
}
