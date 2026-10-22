//! Shared test programs.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use timemachine::sources::{MapEnv, MemFs, NetError, Request, Response, StubNet};
use timemachine::{Builder, Ctx};

/// Sources with something in every slot, so the probe exercises all of them.
pub fn rich_setup(b: Builder) -> Builder {
    let mut calls = 0u32;
    b.fs(MemFs::new()
        .with_file("config/limits.toml", "max = 3\n")
        .with_file("data/a.txt", "alpha"))
        .env(MapEnv::new().with("REGION", "eu-1"))
        .net(StubNet::new(move |req: &Request| {
            calls += 1;
            if req.target.ends_with("/flaky") && calls % 2 == 0 {
                Err(NetError::new("timeout", "upstream did not answer"))
            } else {
                Ok(Response::new(200, format!("{} #{calls}", req.target)))
            }
        }))
}

/// A program that reads every kind of input and returns a transcript of what
/// it observed. Two runs with the same inputs must yield the same transcript.
pub fn probe(ctx: &Ctx) -> Result<String, String> {
    let mut log = Vec::new();
    ctx.mark("start");
    let t0 = ctx.now_nanos();
    log.push(format!("region={:?}", ctx.env_var("REGION")));
    log.push(format!("missing={:?}", ctx.env_var("NOPE")));
    log.push(format!(
        "limits={:?}",
        ctx.read_to_string("config/limits.toml")
            .map_err(|e| e.to_string())
    ));
    log.push(format!(
        "absent={:?}",
        ctx.read("nope.txt").map_err(|e| e.kind())
    ));
    log.push(format!(
        "escape={:?}",
        ctx.read("../etc/passwd").map_err(|e| e.kind())
    ));
    ctx.write("out/result.txt", b"\x00\xffbinary")
        .map_err(|e| e.to_string())?;
    log.push(format!(
        "roundtrip={:?}",
        ctx.read("out/result.txt").map_err(|e| e.to_string())
    ));
    log.push(format!("exists={}", ctx.exists("out")));
    log.push(format!(
        "list={:?}",
        ctx.list("data").map_err(|e| e.to_string())
    ));
    ctx.remove("data/a.txt").map_err(|e| e.to_string())?;
    log.push(format!("exists_after_remove={}", ctx.exists("data/a.txt")));
    let mut order = vec!["a", "b", "c", "d", "e"];
    ctx.shuffle(&mut order);
    log.push(format!("order={order:?}"));
    log.push(format!("roll={}", ctx.gen_range(1..7)));
    log.push(format!("coin={}", ctx.gen_bool(0.5)));
    for _ in 0..3 {
        let r = ctx.request(&Request::new("GET", "https://svc.test/flaky").with_body("q"));
        log.push(format!(
            "net={:?}",
            r.map(|r| (r.status, String::from_utf8_lossy(&r.body).into_owned()))
        ));
        ctx.sleep(Duration::from_millis(ctx.gen_range(1..50)));
    }
    let elapsed = ctx.now_nanos() - t0;
    log.push(format!("elapsed_ns={elapsed}"));
    ctx.mark("end");
    Ok(log.join("\n"))
}

/// The token-cache bug from `examples/flaky_cache.rs`, in library form.
pub fn token_cache_test(ctx: &Ctx) -> Result<(), String> {
    const TTL: u64 = 100_000_000;
    let mut cached: Option<(String, u64)> = None;
    let mut fetches = 0;
    let mut get = |ctx: &Ctx| -> Result<String, String> {
        let now = ctx.now_nanos();
        if let Some((t, exp)) = &cached {
            if now < *exp {
                return Ok(t.clone());
            }
        }
        let r = ctx
            .request(&Request::new("POST", "https://auth.test/token"))
            .map_err(|e| e.to_string())?;
        fetches += 1;
        let t = String::from_utf8_lossy(&r.body).into_owned();
        cached = Some((t.clone(), now + TTL));
        Ok(t)
    };
    get(ctx)?;
    ctx.sleep(Duration::from_millis(90 + ctx.gen_range(0..12)));
    get(ctx)?;
    if fetches != 1 {
        return Err(format!("token refetched: {fetches} fetches"));
    }
    Ok(())
}

pub fn token_setup(b: Builder) -> Builder {
    let mut n = 0;
    b.net(StubNet::new(move |_r: &Request| {
        n += 1;
        Ok(Response::new(200, format!("tok-{n}")))
    }))
}
