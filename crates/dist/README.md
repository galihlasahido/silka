# silka-dist

The application half of shipping software: **which update applies to this
install, is the file we downloaded the file we asked for, and what did the
process say on its way down.**

The other half — signing, notarizing, bundling, uploading — is not code. It is
`.github/workflows/release.yml`, the scripts next to it, and
[`docs/RELEASE.md`](../../docs/RELEASE.md), which explains a release from zero.
This crate exists because four of those steps have a counterpart that runs
inside the shipped binary and therefore has to be testable:

| Module | The question it answers |
| --- | --- |
| `version` | Is `1.4.0-rc.2` newer than `1.4.0-rc.10`? (no, and that is the whole point) |
| `feed` | What did the release pipeline publish? |
| `update` | Which of those releases applies to *this* install, on *this* OS, in *this* rollout bucket? |
| `sha256` | Is the file we downloaded byte-for-byte the file the feed described? |
| `pending` | What has to happen at the next restart, and what if the swap fails? |
| `crash` | What is written down before the process dies, and where is it read back? |

## One deliberate refusal, one optional dependency

**1. This crate does not verify signatures.** It computes the digest, it hands
you the exact bytes that were signed, and it takes a `SignatureVerifier` you
implement with a real cryptography crate. Hand-rolling Ed25519 field arithmetic
in a UI framework — with no compiler and no test vectors from a third party —
would produce a verification routine that looks like security and is not. The
digest check it *does* perform is integrity, not authenticity, and the type
names say so.

**2. Minidumps come from one optional dependency.** `crash::write_minidump`
writes a real dump of the current process on macOS and Windows through
`minidump-writer`, behind the default-on `minidump` feature (opt out with
`default-features = false` for the zero-dependency build). Elsewhere it returns
`MinidumpError::Unsupported` saying why, the convention the platform crate uses
for every backend it does not have. The dump is **in-process**: sound for a Rust
panic, unreliable for heap corruption or a stack overflow, which need a
Crashpad-style handler process (a second signed executable, and a distribution
decision rather than a function). The JSON report beside the dump — application,
version, build id, platform, panic label, message and location — is what makes
it symbolicatable six months later.

## Almost no dependencies, on purpose

Apart from that one opt-out-able crash backend, nothing here pulls a
dependency tree. An updater is the one component that cannot be fixed by an update. Every byte of
its logic is arithmetic over bytes here — SHA-256, a JSON reader, a version
ordering — so that the code path which decides whether to replace the
application is a code path you can read in an afternoon.
