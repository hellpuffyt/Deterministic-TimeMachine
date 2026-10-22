# Sources of nondeterminism that are NOT controlled

Deterministic-TimeMachine controls exactly the inputs your code reads through a `Ctx`. Everything below is outside that boundary. If a test's outcome depends on one of these, recording and replay will not make it reproducible — replay will either report "did not reproduce" or, if the uncontrolled input changes which `Ctx` calls are made, a divergence.

| source | why it is not controlled | what to do instead |
|---|---|---|
| OS thread scheduling | The library does not replace or patch the OS scheduler. | Model the concurrent steps with `sched::run` (each step order becomes a recorded `choose`), or serialize the test. |
| `std::time::Instant` / `SystemTime::now()` called directly | Only `ctx.now()` is intercepted. | Read time through the `Ctx`; pass it (or a `Clock`) to the code under test. |
| Time read inside third-party crates (timeouts in HTTP clients, `tokio::time`) | No hook into other crates' clocks. | Wrap the dependency behind your own trait and inject a `Ctx`-backed implementation. An async adapter is planned. |
| `HashMap`/`HashSet` iteration order with the default `RandomState` | The hasher is seeded per process. | Use `BTreeMap`, sort before iterating, or use a fixed hasher. |
| `rand::thread_rng()` and other global RNGs | Not routed through the `Ctx`. | Use `ctx.next_u64()` / `gen_range` / `shuffle`, or implement your RNG on top of them. |
| Syscalls outside the adapters: sockets opened directly, `std::fs` used directly, `std::env::var`, process spawning, signals | No syscall interception (by design, on every platform). | Route them through `Ctx` (`read`, `write`, `env_var`, `request`) or your own trait backed by it. |
| Memory addresses, pointer hashing, allocation order | Not observable through the `Ctx`. | Avoid making decisions based on addresses. |
| Floating-point differences across CPUs/compilers | Recorded values are replayed exactly, but code that *computes* floats from replayed inputs may still differ across platforms. | Keep recorded decisions integer; compare floats with tolerances. |
| Data races / undefined behavior | Not detectable or controllable here. | Use Miri, loom, sanitizers. |
| Global mutable state shared between test runs (statics, caches) | Not part of the trace. `minimize` refuses to start when the original decisions no longer reproduce the failure, which is the typical symptom. | Reset global state per run, or move it into values created inside the test. |
| Symlink changes inside a `RealFs` root during recording | Paths are confined syntactically (`..` and absolute paths are refused); existing symlinks are followed by the OS and not re-checked. | Record with `MemFs` when possible; replay never touches the real filesystem. |

## Platform notes

- The library and CLI are plain Rust with no platform-specific code paths except process exit codes; CI runs Linux, macOS and Windows (see `docs/PRODUCTION_READINESS.md` for which have actually been observed passing).
- Call sites are normalized to `/` separators so traces recorded on one OS replay on another.
