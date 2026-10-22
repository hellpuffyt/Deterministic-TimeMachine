# Security policy

## Supported versions

Only the latest commit on `main` is supported while the project is at 0.x.

## Reporting a vulnerability

Please report vulnerabilities **privately** through GitHub's private vulnerability reporting:
<https://github.com/hellpuffyt/Deterministic-TimeMachine/security/advisories/new>

Do not open a public issue for a vulnerability. Include the affected version or commit, a description, and a reproducer if possible (a `.tmtrace` file is fine — but please make sure it contains no secrets of your own). You should receive an acknowledgement within a week. This is a volunteer-maintained project; there is no bug bounty.

## Scope and threat model

What this project considers in scope:

- **Trace parsing** (`Trace::parse`, `Trace::load`, `tm validate|inspect|diff`) must reject malformed input with an error, never panic, and never execute anything.
- **`RealFs` confinement**: paths with `..` or absolute paths must be refused.
- **The `tm` CLI** must only run the command the user passed after `--`, and must not leak `TM_*` variables of its own environment into children in unexpected ways.

Out of scope, by design (documented in the README and `docs/LIMITATIONS.md`):

- A trace is *input* to the program you replay it into. Replaying a malicious trace can make your program observe malicious values, exactly as a malicious test fixture could.
- The trace checksum detects accidents, not deliberate tampering.
- Recorded traces can contain sensitive data your program read; nothing is redacted.
- Symlinks already present inside a `RealFs` root are followed.

No external security audit has been performed.
