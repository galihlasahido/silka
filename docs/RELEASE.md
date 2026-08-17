# Releasing a silka application

How to take a silka application from a green `cargo test` to something a stranger
can download, install, trust, and be updated from — starting from nothing.

Read it once end to end before the first release. Every section after **Cutting a
release** is reference material you will come back to.

---

## 1. What "released" actually means

Five things have to be true, and only the first one is about code:

| | Without it |
|---|---|
| **It builds** for each OS | — |
| **It is bundled** — `.app`/`.dmg`, `.msi`/NSIS, AppImage/`.deb`/`.rpm` | Users get a bare executable and no icon, no Start-menu entry, no uninstaller |
| **It is signed** | macOS: Gatekeeper blocks it **and system notifications silently do nothing**. Windows: SmartScreen hides the Run button |
| **It is notarized** (macOS) | Gatekeeper blocks it even though it is signed |
| **It is described by a feed** | Nobody is ever offered version two |

The pipeline that does all five is `.github/workflows/release.yml`. The half that
runs inside the shipped binary — deciding which update applies, verifying the
download, applying it at restart, writing a crash report — is the `silka-dist`
crate, and it is documented in `crates/dist/README.md`.

> **The rule from INTEGRASI-NATIVE §9:** signing and the updater go into CI from
> the first flagship application, not from the first release. Notarization that
> fails on release day is a bad tradition that can be avoided. Run the workflow
> by hand with `dry-run: true` long before you need it to work.

---

## 2. One-time setup

You do this once per product. Budget half a day, most of it waiting for Apple.

### 2.1 macOS — a Developer ID certificate

1. Join the **Apple Developer Program** ($99/year). An individual account works;
   an organisation account needs a D-U-N-S number and takes longer.
2. In Xcode → Settings → Accounts, or on developer.apple.com, create a
   **Developer ID Application** certificate. This is the one for software
   distributed *outside* the App Store. "Apple Development" and "Apple
   Distribution" certificates cannot sign a Developer ID build.
3. Export it from Keychain Access as a `.p12` **with the private key**, and give
   it a password. Verify you exported the right thing:

   ```sh
   security find-identity -v -p codesigning
   # 1) A1B2… "Developer ID Application: Acme Ltd (AB12CD34EF)"
   ```

   The quoted string is `MACOS_SIGN_IDENTITY`. The ten characters in brackets
   are your team id.

4. Turn the `.p12` into a secret:

   ```sh
   base64 -i DeveloperID.p12 | pbcopy
   ```

### 2.2 macOS — a notarization credential

Prefer an **App Store Connect API key** over an Apple ID: it is scoped, it is
revocable, and it does not stop working when a person leaves.

1. App Store Connect → Users and Access → Integrations → App Store Connect API.
2. Create a key with the **Developer** role. Download `AuthKey_XXXXXXXX.p8` —
   you can only download it once.
3. Note the **Key ID** (the `XXXXXXXX`) and the **Issuer ID** (a UUID on the same
   page).
4. `base64 -i AuthKey_XXXXXXXX.p8 | pbcopy`

<details>
<summary>Fallback: an Apple ID with an app-specific password</summary>

appleid.apple.com → Sign-In and Security → App-Specific Passwords. Use it as
`NOTARY_PASSWORD` together with `NOTARY_APPLE_ID` and `NOTARY_TEAM_ID`. Never the
account password — notarytool would then hold an unrestricted credential.

</details>

### 2.3 Windows — a code-signing certificate

Since June 2023 every publicly trusted code-signing key must live on hardware.
That leaves two workable options:

- **Azure Trusted Signing** (recommended). Pay per month, the key never exists as
  a file, and CI authenticates with a service principal. Set
  `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE`,
  and the three `AZURE_*` credentials.
- **A certificate from a CA** (DigiCert, Sectigo, SSL.com) delivered on a token
  or in their cloud HSM. If your provider gives you an exportable `.pfx`, use
  `WINDOWS_CERT_PFX` and `WINDOWS_CERT_PASSWORD`.

An **EV** certificate additionally gives the build SmartScreen reputation from
day one; a standard OV certificate has to earn reputation over a few thousand
installs, during which some users still see a warning.

### 2.4 The update signing key

This is yours, not a vendor's. Generate it once, on a machine you trust, and back
it up somewhere that is not this repository:

```sh
openssl genpkey -algorithm ed25519 -out update-signing.pem
openssl pkey -in update-signing.pem -pubout -out update-signing.pub
```

- The **private** half becomes the `UPDATE_SIGNING_KEY` secret.
- The **public** half is compiled into the application, as the key its
  `SignatureVerifier` trusts. A public key fetched at runtime is a public key an
  attacker can replace, which is the entire point of compiling it in.

Losing the private key means you can never ship another update to existing
installs. Rotating it is a two-release job: ship a build that trusts both keys,
wait for it to reach everyone, then sign with the new key only.

### 2.5 Somewhere to serve the feed

Any static HTTPS host — S3 + CloudFront, R2, GitHub Pages, a plain nginx. Two
requirements:

- **HTTPS.** The signature protects the artifacts, but a feed served over HTTP
  can be replaced with an older one to hold a user back on a version with a
  known bug.
- A **stable** URL per channel, e.g.
  `https://downloads.example.com/stable/feed.json`. Set the base in
  `.github/workflows/release.yml` (`BASE_URL`).

### 2.6 The secrets, in one table

Repository → Settings → Secrets and variables → Actions.

| Secret | Used by | Notes |
|---|---|---|
| `MACOS_CERT_P12` | `macos-sign.sh` | base64 of the `.p12` |
| `MACOS_CERT_PASSWORD` | `macos-sign.sh` | the `.p12` export password |
| `MACOS_KEYCHAIN_PASSWORD` | `macos-sign.sh` | any string; the temporary keychain's own |
| `MACOS_SIGN_IDENTITY` | `macos-sign.sh` | `Developer ID Application: … (TEAMID)` |
| `NOTARY_API_KEY_P8` | `macos-notarize.sh` | base64 of the `.p8` |
| `NOTARY_API_KEY_ID` | `macos-notarize.sh` | the key id |
| `NOTARY_API_ISSUER` | `macos-notarize.sh` | the issuer UUID |
| `WINDOWS_CERT_PFX` | `windows-sign.ps1` | base64 of the `.pfx` (or use the `AZURE_*` set) |
| `WINDOWS_CERT_PASSWORD` | `windows-sign.ps1` | |
| `UPDATE_SIGNING_KEY` | `make-update-feed.sh` | the Ed25519 private key, PEM |

Nothing in this repository contains any of them. Every script reads them from
the environment and deletes what it wrote to disk in a `trap` or a `finally`.

---

## 3. Cutting a release

### 3.1 Rehearse first

```
Actions → Release → Run workflow
  version:  1.4.0
  channel:  stable
  rollout:  25
  dry-run:  true
```

This builds, bundles, signs, notarizes and generates the feed — and publishes
nothing. Do this the first time, after any certificate change, and any time it
has been more than a couple of months.

### 3.2 Bump the version in the three places that must agree

```
Cargo.toml                  workspace.package.version
packaging/Packager.toml     version
git tag                     v<version>
```

The `settings` job checks all three and fails the release if they disagree. When
they drift, the bundle says one thing and the feed says another, and the updater
offers an install the user already has.

### 3.3 Tag and push

```sh
git tag -a v1.4.0 -m "1.4.0"
git push origin v1.4.0
```

A tag matching `v*` starts the workflow. A pre-release version (`1.4.0-rc.1`)
goes to the `beta` channel automatically — see §5.2 for why that ordering is not
the string ordering.

### 3.4 Smoke-test before the feed goes up

The release is created as a **draft** and the feed is **not** uploaded
automatically. That gap is on purpose: it is the last point where a human looks
at the thing before anyone is offered it.

On a machine that has never seen this build:

- **macOS** — download the `.dmg` *through a browser*, so it carries the
  quarantine attribute a real user's download has. Drag to Applications, launch.
  No Gatekeeper dialog should appear at all.
- **Windows** — run the `.msi` and the NSIS `.exe`. A SmartScreen warning means
  the file was not signed, or was signed without a timestamp.
- **Linux** — `chmod +x` the AppImage and run it; `sudo dpkg -i` the `.deb`.

Then check that the application starts, that a notification appears, and that
the About box shows the version you think you built.

### 3.5 Publish

1. Upload `feed.json` to `$BASE_URL/<channel>/feed.json`.
2. Publish the draft GitHub release.
3. Keep `symbols-<version>.tar.gz` somewhere permanent — see §7.

Nobody is offered the update until step 1. Until then the artifacts exist and
are inert, which is the right state for a build nobody has smoke-tested.

---

## 4. What the pipeline does

| Job | Runner | What it produces |
|---|---|---|
| `settings` | ubuntu | version, channel, rollout; fails on a version mismatch |
| `macos` | macos-14 | universal `.dmg`, signed, notarized, stapled + symbols |
| `windows` | windows-latest | `.msi` and NSIS `.exe`, both Authenticode-signed + symbols |
| `linux` | ubuntu-22.04 | AppImage, `.deb`, `.rpm` + symbols |
| `publish` | ubuntu | `feed.json`, symbol archive, draft GitHub release |

Four details worth knowing before you edit any of it.

**The macOS build is universal.** Both architectures are built and fused with
`lipo`. Two separate downloads would work and would double every step below —
including notarization, which is the slow one.

**`--deep` is never used when signing.** Apple documents it as unsuitable for
shipping software: it re-signs nested code with the *outer* bundle's
entitlements, silently widening every helper's permissions. `macos-sign.sh`
walks the bundle and signs from the inside out.

**The Linux build runs on ubuntu-22.04, not the newest image.** A binary linked
against a newer glibc refuses to start on an older distribution, with an error
message no user can act on. The oldest supported build environment is the only
honest one.

**`publish` refuses an unsigned build.** Each platform job records whether it
signed; `publish` reads those and fails if any says no. A build that reaches
users unsigned trains them to click through the warning.

---

## 5. The update feed

### 5.1 The document

`.github/scripts/make-update-feed.sh` writes it; `silka_dist::feed::Feed::parse`
reads it. The two are meant to be read side by side — every field is defended in
a comment in `crates/dist/src/feed.rs`.

```json
{
  "feed": 1,
  "app": "dev.silka.dashboard",
  "channel": "stable",
  "releases": [
    {
      "version": "1.4.0",
      "published": "2026-08-17T09:00:00Z",
      "mandatory": false,
      "rollout": 25,
      "notes": "https://example.com/notes/1.4.0.html",
      "minimum_os": { "macos": "11.0", "windows": "10.0.19041" },
      "artifacts": [
        {
          "platform": "macos-universal",
          "format": "dmg",
          "url": "https://downloads.example.com/v1.4.0/Silka-Dashboard-1.4.0.dmg",
          "size": 41234567,
          "sha256": "ba7816bf…",
          "signature": "…base64…"
        }
      ]
    }
  ]
}
```

The feed lists **every** release, not just the newest. Merging is what keeps the
older ones available: an install on macOS 11 whose newest allowed release is
1.2.0 finds it in the same document.

### 5.2 Staged rollout

`rollout` is a percentage, and it is the difference between a bad release
reaching everyone and reaching a twentieth of everyone.

Each install hashes a stable per-machine identifier into a bucket `0..=99`
(`silka_dist::update::bucket_for`). Because it is a hash and not a random draw:

- the same install lands in the same bucket forever, so **raising the rollout
  never takes the update away** from someone already offered it;
- the buckets are even, so 5% means 5% of installs — not 5% of whoever checked
  first.

Widening a rollout is one edit to `rollout` in the published `feed.json`. There
is nothing to rebuild.

Pausing a bad release is the same edit, back down to `0`. Anyone who already
installed it keeps it; nobody new is offered it. To actively pull people off it,
publish a *newer* release — an updater only ever moves forward, never back.

### 5.3 Mandatory releases

`"mandatory": true` bypasses exactly two checks: the user's skip list and the
staged rollout. It does **not** bypass the channel, the OS floor or a missing
artifact, because those describe installs that would *fail* rather than installs
that said no.

It is a hint to the application, not a power the updater has. `silka-dist`
reports it; the application decides what an undismissable dialog does to someone
mid-sentence.

### 5.4 `minimum_os`

Keeps a release that needs macOS 13 away from a Mac on 12. Without it the newest
release is offered forever, fails to launch, and leaves the user on a version
that at least ran.

The floor is written in three places and they must agree:
`packaging/macos/Info.plist` (`LSMinimumSystemVersion`),
`packaging/Packager.toml` (`minimum-system-version`), and the
`--min-macos` flag in the workflow. The `settings` job checks the first two.

### 5.5 Delta updates

The feed can carry a `deltas` array per artifact, keyed by the version each patch
applies *from*. `silka-dist` will only use one if the application opted in with
`Install::deltas(true)`, and that default is off for a reason: applying a delta
needs a patcher, `silka-dist` ships none, and handing an application a `.delta`
URL it cannot apply is worse than handing it the full download.

A version with no matching delta falls back to the full artifact. That is not an
error, it is a bigger download.

---

## 6. Applying an update

The shape a shipped application follows:

1. **Check.** Fetch the feed, `Feed::parse`, `update::choose`. `Ok(None)` is the
   overwhelmingly common answer and is not an error.
2. **Download.** `Offer::download()` gives a `Download`; feed it chunks. It
   checks the size first — a truncated 200 MB file is rejected without hashing
   it — then the digest, then the signature.
3. **Stage.** Write a `pending::Pending` record next to the payload, on the same
   volume as the installation. A rename inside one filesystem is atomic; a copy
   across two is a window during which the application is half-replaced.
4. **Restart.** At the next launch, `pending::next_launch` decides. Count the
   attempt and save the record **before** attempting the swap, never after: that
   is the difference between retrying forever and noticing.
5. **Swap.** `pending::swap_in_place` does three renames — live aside, staged in,
   backup kept. If the second fails, the first is undone and the error says
   whether that worked.

On Windows the swap cannot be done by the process being replaced: a running
executable cannot be renamed. That is what a small relauncher executable is for,
and it is the reason `swap_in_place` is a free function rather than a method.

The backup is deleted by the *application*, after the new version has started
successfully — not by the swap. Deleting it at swap time throws away the rollback
at exactly the moment the new build turns out not to launch.

---

## 7. Crash reports and symbols

A release build's stack trace is a list of addresses. Turning it into function
names needs the debug info that was discarded when the binary shipped, and the
only moment that debug info exists is on the machine that built it.

`.github/scripts/collect-symbols.sh` runs in every platform job, before anything
is stripped, and writes:

```
symbols/<platform>/<version>/<build>/…
```

which is exactly what `silka_dist::crash::CrashReport::symbol_path()` reports —
`macos-aarch64/1.4.0/9e75a29` names its own directory. **Keep the archive.** A
crash report without it is unreadable, and there is no way to regenerate it.

In the application:

```rust
silka_core::recover::install_hook();
silka_dist::crash::report_to_directory(
    silka_dist::crash::CrashContext::new("dev.silka.dashboard", version)
        .build(env!("SILKA_BUILD")),      // stamped by the release workflow
    crash_directory,
);
```

At the next launch, `crash::read_all` hands back what was written, so it can be
shown or uploaded. `crash::prune` bounds the directory: a crash loop must not
fill a user's disk with the same report ten thousand times.

Minidumps are **not** written. `crash::write_minidump` returns
`MinidumpError::Unsupported` naming what it waits for, following the convention
`silka-platform` uses for every backend it does not have. An in-process writer
(`minidump-writer`) is unsound the moment the crash was heap corruption — the
allocator it needs is the thing that broke — and the correct out-of-process
answer is a second executable that itself has to be signed and notarized. That
is a distribution problem as much as a code one, which is why it is written down
here rather than stubbed out as `Ok(())`.

---

## 8. The Mac App Store is a different product

Not a flag on the Developer ID build. Four differences, and each one is a
separate decision:

1. **The App Sandbox is mandatory.** Every file the application touches must
   come through a user-selected path, a security-scoped bookmark, or a container
   directory. Entitlements are in `packaging/macos/entitlements.mas.plist`, one
   justification per key.
2. **The updater must be compiled out.** An App Store build is updated by the
   App Store. A build that also replaces its own bundle is rejected in review.
   Keep `silka-dist::crash` — crash reports are allowed, they just have to be
   written inside the container.
3. **Different identities.** `3rd Party Mac Developer Application` for the
   bundle, `3rd Party Mac Developer Installer` for the `.pkg`. A Developer ID
   certificate cannot sign a submission.
4. **No notarization.** Submissions are reviewed, not notarized, and
   `xcrun notarytool` rejects a MAS-signed bundle.

There is no CI job for this yet, on purpose: a submission needs a review cycle
and a human, and automating the upload while the review is manual buys nothing.

---

## 9. When it goes wrong

| Symptom | What it means |
|---|---|
| `errSecInternalComponent` from `codesign` | The keychain is locked, or `set-key-partition-list` did not run. It is *not* a certificate problem |
| codesign hangs forever in CI | A GUI prompt nobody can answer — same cause as above |
| Notarization: `The signature does not include a secure timestamp` | `--timestamp` was missing, or the timestamp server was unreachable during signing |
| Notarization: `The executable does not have the hardened runtime enabled` | `--options runtime` was missing on some nested binary. `notarytool log` names the file |
| Notarization: `The binary is not signed with a valid Developer ID certificate` | An "Apple Development" certificate was used |
| "damaged and can't be opened" after a clean install | Signed and notarized but **not stapled**, and the machine was offline |
| Notifications do nothing, no error | Unsigned bundle. See `silka_platform::notification::needs_bundle()` |
| SmartScreen warns on a signed `.exe` | No timestamp, or an OV certificate with no reputation yet |
| Users are never offered an update | The feed was not uploaded; the `app` or `channel` does not match; `rollout` is 0; `minimum_os` excludes them. `silka_dist::update::explain` prints the reason for every release |
| `WrongApp` / `WrongChannel` in the logs | A server is handing out someone else's feed. Deliberately loud rather than silently ignored |
| The update downloads and then fails verification | `DigestMismatch` is a corrupt mirror or a stale cache. `BadSignature` means *attack* rather than accident — delete the file |

---

## 10. What is not automated yet

Written down rather than left to be rediscovered:

- **Uploading the feed and the artifacts to the CDN.** The workflow produces
  `feed.json` and attaches it to a draft release; putting it on the host is a
  manual step, and while releases are rare that is a feature.
- **The Mac App Store submission** (§8).
- **A Windows relauncher executable.** `swap_in_place` is written and tested;
  the small second binary that calls it after the application exits is not.
- **Delta generation.** The feed format carries deltas and the client reads
  them; nothing produces them yet.
- **Minidumps** (§7).
- **`SILKA_BUILD`.** The workflow knows the commit; stamping it into the binary
  through a build script, so `CrashContext::build` has something to report, is
  one line that has not been written.
