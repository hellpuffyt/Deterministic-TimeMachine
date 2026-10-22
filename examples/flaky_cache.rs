//! A genuinely flaky test: a token cache with a TTL, exercised by a client
//! that retries after a jittered backoff.
//!
//! The bug: the retry backoff (90 ms + up to 11 ms of jitter) can exceed the
//! cache TTL (100 ms). When it does, the cached token has expired and the
//! client fetches a second one, which the test treats as a failure. With the
//! real clock and real randomness this fails on roughly one run in six —
//! sometimes more, because real sleeps overshoot.
//!
//! ```text
//! cargo run --example flaky_cache -- --live-runs 30      # see it flake
//! tm hunt -- target/debug/examples/flaky_cache            # capture a failing run
//! tm replay tm-out/<trace> -- target/debug/examples/flaky_cache
//! ```
//!
//! `FLAKY_CACHE_VARIANT` simulates code changes for the replay demos:
//! `fixed` (backoff well inside the TTL) and `extra-draw` (one more random
//! draw, which changes the input sequence and makes replay diverge).

use std::process::ExitCode;
use std::time::Duration;

use timemachine::sources::{NetError, Request, Response, StubNet};
use timemachine::{Builder, Ctx, harness};

const TTL: Duration = Duration::from_millis(100);

struct TokenCache {
    token: Option<(String, u64)>,
    fetches: u32,
}

impl TokenCache {
    fn get(&mut self, ctx: &Ctx) -> Result<String, String> {
        let now = ctx.now_nanos();
        if let Some((token, expires_at)) = &self.token {
            if now < *expires_at {
                return Ok(token.clone());
            }
        }
        let resp = ctx
            .request(&Request::new("POST", "https://auth.example.test/token"))
            .map_err(|e| format!("token request failed: {e}"))?;
        self.fetches += 1;
        let token = String::from_utf8_lossy(&resp.body).into_owned();
        let ttl_nanos = u64::try_from(TTL.as_nanos()).unwrap_or(u64::MAX);
        self.token = Some((token.clone(), now.saturating_add(ttl_nanos)));
        Ok(token)
    }
}

/// The test: the retry must reuse the cached token.
fn token_is_reused_on_retry(ctx: &Ctx) -> Result<(), String> {
    let variant = std::env::var("FLAKY_CACHE_VARIANT").unwrap_or_default();
    let mut cache = TokenCache {
        token: None,
        fetches: 0,
    };
    let first = cache.get(ctx)?;

    // The first API call failed transiently; retry after a jittered backoff.
    let jitter = ctx.gen_range(0..12);
    if variant == "extra-draw" {
        let _ = ctx.gen_range(0..2);
    }
    let base = if variant == "fixed" { 40 } else { 90 };
    let backoff = Duration::from_millis(base + jitter);
    let before = ctx.now();
    ctx.sleep(backoff);
    let waited = ctx.now().saturating_sub(before);

    let second = cache.get(ctx)?;
    if cache.fetches != 1 {
        return Err(format!(
            "token refetched after the retry: {first} -> {second}, backoff {} ms, actually waited {:.1} ms, TTL {} ms",
            backoff.as_millis(),
            waited.as_secs_f64() * 1e3,
            TTL.as_millis()
        ));
    }
    Ok(())
}

/// A deterministic fake auth server: tokens tok-1, tok-2, …
fn auth_server() -> StubNet<impl FnMut(&Request) -> Result<Response, NetError> + Send> {
    let mut issued = 0u32;
    StubNet::new(move |_req: &Request| {
        issued += 1;
        Ok(Response::new(200, format!("tok-{issued}")))
    })
}

/// Real clock and randomness by default (that is what makes the test
/// flaky); the auth server is always the fake.
fn setup(b: Builder) -> Builder {
    b.real_sources().net(auth_server())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, n] = args.as_slice() {
        if flag == "--live-runs" {
            let Ok(runs) = n.parse::<u32>() else {
                eprintln!("usage: flaky_cache [--live-runs N]");
                return ExitCode::from(2);
            };
            let mut failures = 0;
            for i in 1..=runs {
                let ctx = setup(Builder::new("flaky_cache")).live();
                let out = harness::run(&ctx, token_is_reused_on_retry);
                if out.kind.is_failure() {
                    failures += 1;
                    println!("run {i:>3}: FAILED  {}", out.message.unwrap_or_default());
                }
            }
            println!(
                "{runs} live runs: {failures} failed, {} passed",
                runs - failures
            );
            return ExitCode::SUCCESS;
        }
    }
    timemachine::run_main("flaky_cache", setup, token_is_reused_on_retry)
}
