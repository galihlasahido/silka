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

## Two deliberate refusals

**1. This crate does not verify signatures.** It computes the digest, it hands
you the exact bytes that were signed, and it takes a `SignatureVerifier` you
implement with a real cryptography crate. Hand-rolling Ed25519 field arithmetic
in a UI framework — with no compiler and no test vectors from a third party —
would produce a verification routine that looks like security and is not. The
digest check it *does* perform is integrity, not authenticity, and the type
names say so.

**2. This crate does not write minidumps.** `crash::write_minidump` returns
`MinidumpError::Unsupported` naming the API it is waiting for
(`minidump-writer`, or Crashpad's handler process), the same convention the
platform crate uses for every backend it does not have yet. What it does write
is the metadata *around* the dump — application, version, build id, platform,
panic label, message and location — because that file is what makes a dump
symbolicatable six months later, and because it is useful on its own.

## Zero dependencies, on purpose

An updater is the one component that cannot be fixed by an update. Every byte of
its logic is arithmetic over bytes here — SHA-256, a JSON reader, a version
ordering — so that the code path which decides whether to replace the
application is a code path you can read in an afternoon.
