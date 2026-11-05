//! Подпись обновлений.
//!
//! ```text
//! bolh-sign keygen <файл закрытого ключа> <файл открытого ключа>
//! bolh-sign sign <версия> <установщик>   # ключ — из UPDATE_SIGNING_KEY
//! ```
//!
//! Закрытый ключ никогда не печатается: ни при создании, ни при подписи.

use std::path::Path;
use std::process::ExitCode;

const KEY_ENV: &str = "UPDATE_SIGNING_KEY";
const PUBLIC_KEY_FILE: &str = "packaging/update-key.pub";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["keygen", secret, public] => keygen(Path::new(secret), Path::new(public)),
        ["sign", version, file] => {
            let key = std::env::var(KEY_ENV).unwrap_or_default();
            let public = std::fs::read_to_string(PUBLIC_KEY_FILE).map_err(|e| e.to_string());
            public.and_then(|p| sign(&key, &p, version, Path::new(file)))
        }
        _ => Err("использование: keygen <secret> <public> | sign <version> <file>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bolh-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

fn keygen(secret: &Path, public: &Path) -> Result<(), String> {
    if secret.exists() {
        return Err(format!("{} уже есть — не перезаписываю", secret.display()));
    }
    let kp = minisign::KeyPair::generate_unencrypted_keypair().map_err(|e| e.to_string())?;
    let sk = kp
        .sk
        .to_box(Some("Bolh Place update signing key"))
        .map_err(|e| e.to_string())?
        .into_string();
    let pk = kp.pk.to_box().map_err(|e| e.to_string())?.into_string();
    write_private(secret, sk.as_bytes())?;
    std::fs::write(public, pk).map_err(|e| e.to_string())?;
    eprintln!("ключи созданы: {} и {}", secret.display(), public.display());
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Подписывает файл и сразу проверяет подпись открытым ключом из
/// репозитория: если секрет не от того ключа, выпуск падает здесь, а не у
/// людей при обновлении.
fn sign(secret: &str, public: &str, version: &str, file: &Path) -> Result<(), String> {
    if secret.trim().is_empty() {
        return Err(format!("нет секрета {KEY_ENV}"));
    }
    let version = semver::Version::parse(version.trim_start_matches('v'))
        .map_err(|e| format!("версия: {e}"))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("имя файла")?
        .to_string();
    // Текст ошибки разбора ключа не выводим: в нём может оказаться кусок ключа.
    let sk = minisign::SecretKeyBox::from_string(secret.trim())
        .and_then(|b| b.into_unencrypted_secret_key())
        .map_err(|_| format!("{KEY_ENV} не похож на ключ minisign"))?;
    let data = std::fs::read(file).map_err(|e| e.to_string())?;
    let comment = bolh_core::update::trusted_comment(&version, &name);
    let sig = minisign::sign(None, &sk, &data[..], Some(&comment), None)
        .map_err(|_| "не удалось подписать".to_string())?
        .into_string();

    let release = bolh_core::update::Release {
        version,
        installer_name: name.clone(),
        installer_url: String::new(),
        signature_url: String::new(),
        page_url: String::new(),
    };
    bolh_core::update::verify(&data, &sig, public, &release)
        .map_err(|_| format!("{KEY_ENV} не подходит к открытому ключу {PUBLIC_KEY_FILE}"))?;
    let out = file.with_file_name(format!("{name}{}", bolh_core::update::SIGNATURE_EXT));
    std::fs::write(&out, sig).map_err(|e| e.to_string())?;
    eprintln!("подпись: {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keygen_then_sign_and_verify() {
        let d = tempfile::tempdir().unwrap();
        let (s, p) = (d.path().join("k.key"), d.path().join("k.pub"));
        keygen(&s, &p).unwrap();
        assert!(keygen(&s, &p).is_err(), "существующий ключ не затирается");
        let secret = std::fs::read_to_string(&s).unwrap();
        let public = std::fs::read_to_string(&p).unwrap();
        let file = d.path().join("BolhPlace-1.2.3-setup.exe");
        std::fs::write(&file, b"exe").unwrap();
        sign(&secret, &public, "v1.2.3", &file).unwrap();
        let sig =
            std::fs::read_to_string(d.path().join("BolhPlace-1.2.3-setup.exe.minisig")).unwrap();
        assert!(sig.contains("bolh-place 1.2.3 BolhPlace-1.2.3-setup.exe"));
        assert!(!sig.contains(secret.lines().nth(1).unwrap()));
    }

    #[test]
    fn wrong_or_missing_secret_fails_without_leaking() {
        let d = tempfile::tempdir().unwrap();
        let (s, p) = (d.path().join("a.key"), d.path().join("a.pub"));
        let (s2, p2) = (d.path().join("b.key"), d.path().join("b.pub"));
        keygen(&s, &p).unwrap();
        keygen(&s2, &p2).unwrap();
        let file = d.path().join("x-setup.exe");
        std::fs::write(&file, b"exe").unwrap();
        let other_secret = std::fs::read_to_string(&s2).unwrap();
        let public = std::fs::read_to_string(&p).unwrap();
        let err = sign(&other_secret, &public, "1.0.0", &file).unwrap_err();
        assert!(err.contains("не подходит"));
        assert!(sign("", &public, "1.0.0", &file).is_err());
        let err = sign("секрет-мусор", &public, "1.0.0", &file).unwrap_err();
        assert!(!err.contains("секрет-мусор"));
    }
}
