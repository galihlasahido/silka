//! Read a real feed document with the same parser the shipped binary uses.
//!
//! The release pipeline (`.github/scripts/make-update-feed.sh`) generates the
//! feed with Python and then runs this test with `FEED_PATH` pointing at the
//! result. That closes the one gap a unit test cannot: the generator and the
//! reader are two different programs written in two different languages, and the
//! only proof they agree is one reading the other's output.
//!
//! Without `FEED_PATH` set — which is every ordinary `cargo test` run — the test
//! checks the parser against an inline document instead, so the file is never a
//! silently skipped no-op.

use std::path::PathBuf;

use silka_dist::feed::{Feed, Platform};
use silka_dist::update::{choose, Install};
use silka_dist::version::Version;

/// The document shape the generator emits, kept here as the fallback so this
/// test always asserts something.
const INLINE: &str = r#"{
  "feed": 1,
  "app": "dev.silka.dashboard",
  "channel": "stable",
  "releases": [
    {
      "version": "1.4.0",
      "published": "2026-08-17T09:00:00Z",
      "mandatory": false,
      "rollout": 25,
      "minimum_os": { "macos": "11.0", "windows": "10.0.19041" },
      "artifacts": [
        {
          "platform": "linux-x86_64",
          "format": "AppImage",
          "url": "https://example.com/v1.4.0/Silka-Dashboard-1.4.0.AppImage",
          "size": 61234567,
          "sha256": "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        },
        {
          "platform": "macos-universal",
          "format": "dmg",
          "url": "https://example.com/v1.4.0/Silka-Dashboard-1.4.0.dmg",
          "size": 41234567,
          "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
          "signature": "c2lnbmF0dXJl"
        }
      ]
    }
  ]
}"#;

fn document() -> (String, String) {
    match std::env::var_os("FEED_PATH") {
        Some(path) => {
            let path = PathBuf::from(path);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("tidak bisa membaca {}: {error}", path.display()));
            (path.display().to_string(), text)
        }
        None => (String::from("<inline>"), INLINE.to_string()),
    }
}

#[test]
fn feed_yang_dihasilkan_pipeline_terbaca() {
    let (source, text) = document();
    let feed = Feed::parse(&text)
        .unwrap_or_else(|error| panic!("feed di {source} tidak terbaca: {error}"));

    assert!(
        !feed.app().is_empty(),
        "feed di {source} tidak menyebut aplikasinya"
    );
    assert!(
        !feed.channel().is_empty(),
        "feed di {source} tidak menyebut kanalnya"
    );
    assert!(
        !feed.releases().is_empty(),
        "feed di {source} tidak berisi satu rilis pun"
    );

    let newest = feed.latest().expect("sudah dipastikan tidak kosong");
    assert!(
        !newest.artifacts().is_empty(),
        "rilis {} tidak punya artefak: tidak ada yang bisa diunduh siapa pun",
        newest.version()
    );

    println!(
        "{source}: {} kanal {}, {} rilis, terbaru {} dengan {} artefak",
        feed.app(),
        feed.channel(),
        feed.releases().len(),
        newest.version(),
        newest.artifacts().len()
    );
}

#[test]
fn rilis_terbaru_menawarkan_diri_ke_instalasi_yang_lebih_tua() {
    let (source, text) = document();
    let feed = Feed::parse(&text).expect("feed harus terbaca");
    let newest = feed.latest().expect("feed harus punya rilis");

    if newest.rollout() == 0 && !newest.is_mandatory() {
        // A release published at 0% reaches nobody by design — that is how a bad
        // release is paused. Asserting reachability here would turn a deliberate
        // pause into a red release job.
        println!("{source}: rilis {} masih di rollout 0%", newest.version());
        return;
    }

    // Every platform the newest release ships for must actually be reachable
    // from an install on that platform. The failure this catches is a `platform`
    // string that parses but matches nothing — a typo in the generator's table.
    for artifact in newest.artifacts() {
        let host = match artifact.platform() {
            Platform::MacosUniversal => Platform::MacosArm64,
            other => other.clone(),
        };

        let install = Install::new(feed.app(), Version::ZERO)
            .channel(feed.channel())
            .platform(host.clone())
            .pre_release(true)
            .bucket(0);

        let offer = choose(&feed, &install)
            .unwrap_or_else(|error| panic!("feed di {source} ditolak: {error}"))
            .unwrap_or_else(|| {
                panic!(
                    "feed di {source} tidak menawarkan apa pun ke {host}, padahal ia menerbitkan {}",
                    artifact.platform()
                )
            });

        assert_eq!(
            offer.version(),
            newest.version(),
            "instalasi kosong harus selalu ditawari rilis terbaru"
        );
        assert!(
            !offer.url().is_empty(),
            "artefak {} tidak punya URL",
            artifact.platform()
        );
    }
}

#[test]
fn setiap_artefak_punya_ukuran_dan_digest_yang_masuk_akal() {
    let (source, text) = document();
    let feed = Feed::parse(&text).expect("feed harus terbaca");

    for release in feed.releases() {
        for artifact in release.artifacts() {
            assert!(
                artifact.size() > 0,
                "{source}: artefak {} pada {} berukuran nol",
                artifact.platform(),
                release.version()
            );
            assert!(
                artifact.url().starts_with("https://"),
                "{source}: artefak {} pada {} tidak diunduh lewat HTTPS: {}",
                artifact.platform(),
                release.version(),
                artifact.url()
            );
            // The digest already parsed — `Feed::parse` refuses one that did
            // not — so this only checks it is not the digest of an empty file,
            // which is what a truncated upload produces.
            assert_ne!(
                artifact.sha256().to_string(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "{source}: artefak {} pada {} punya digest berkas kosong",
                artifact.platform(),
                release.version()
            );
        }
    }
}
