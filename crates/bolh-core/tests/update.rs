//! Подпись обновлений: настоящая пара ключей minisign, как в выпуске.

use bolh_core::update::{Release, is_newer, parse_latest, trusted_comment, verify};
use semver::Version;

struct Keys {
    public: String,
    secret: minisign::SecretKey,
}

fn keys() -> Keys {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    Keys {
        public: kp.pk.to_box().unwrap().into_string(),
        secret: kp.sk,
    }
}

fn sign(k: &Keys, data: &[u8], comment: &str) -> String {
    minisign::sign(None, &k.secret, data, Some(comment), None)
        .unwrap()
        .into_string()
}

fn release(v: &str) -> Release {
    Release {
        version: Version::parse(v).unwrap(),
        installer_name: format!("BolhPlace-{v}-setup.exe"),
        installer_url: "https://example.invalid/i".into(),
        signature_url: "https://example.invalid/s".into(),
        page_url: "https://example.invalid/p".into(),
    }
}

#[test]
fn good_signature_passes() {
    let k = keys();
    let r = release("0.2.0");
    let data = b"installer bytes";
    let sig = sign(&k, data, &trusted_comment(&r.version, &r.installer_name));
    verify(data, &sig, &k.public, &r).unwrap();
}

#[test]
fn tampered_file_fails() {
    let k = keys();
    let r = release("0.2.0");
    let sig = sign(&k, b"good", &trusted_comment(&r.version, &r.installer_name));
    assert!(verify(b"evil", &sig, &k.public, &r).is_err());
}

#[test]
fn other_key_fails() {
    let (k, other) = (keys(), keys());
    let r = release("0.2.0");
    let sig = sign(
        &other,
        b"x",
        &trusted_comment(&r.version, &r.installer_name),
    );
    assert!(verify(b"x", &sig, &k.public, &r).is_err());
}

#[test]
fn old_installer_cannot_pretend_to_be_new() {
    let k = keys();
    let old = release("0.1.0");
    let sig = sign(
        &k,
        b"old",
        &trusted_comment(&old.version, &old.installer_name),
    );
    assert!(verify(b"old", &sig, &k.public, &old).is_ok());
    let mut new = release("0.2.0");
    new.installer_name = old.installer_name.clone();
    assert!(verify(b"old", &sig, &k.public, &new).is_err());
}

#[test]
fn garbage_inputs_fail_cleanly() {
    let k = keys();
    let r = release("0.2.0");
    assert!(verify(b"x", "не подпись", &k.public, &r).is_err());
    assert!(verify(b"x", "", "не ключ", &r).is_err());
}

#[test]
fn parse_github_release() {
    let json = br#"{
      "tag_name": "v0.2.0", "draft": false, "prerelease": false,
      "html_url": "https://github.com/Sandman-ing/Bolh-Place/releases/tag/v0.2.0",
      "assets": [
        {"name": "BolhPlace-0.2.0-setup.exe.minisig", "browser_download_url": "https://x/s"},
        {"name": "BolhPlace-0.2.0-setup.exe", "browser_download_url": "https://x/i"}
      ]}"#;
    let r = parse_latest(json).unwrap();
    assert_eq!(r.version, Version::new(0, 2, 0));
    assert_eq!(r.installer_url, "https://x/i");
    assert_eq!(r.signature_url, "https://x/s");
    assert!(is_newer("0.1.0", &r));
    assert!(!is_newer("0.2.0", &r));
    assert!(!is_newer("0.3.0", &r));
    assert!(!is_newer("мусор", &r));
}

#[test]
fn release_without_signature_or_prerelease_is_rejected() {
    let no_sig = br#"{"tag_name":"v1.0.0","html_url":"h","assets":[
        {"name":"BolhPlace-1.0.0-setup.exe","browser_download_url":"u"}]}"#;
    assert!(parse_latest(no_sig).is_err());
    let pre = br#"{"tag_name":"v1.0.0","prerelease":true,"html_url":"h","assets":[]}"#;
    assert!(parse_latest(pre).is_err());
    assert!(parse_latest(b"{}").is_err());
}
