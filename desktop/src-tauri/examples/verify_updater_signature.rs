use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};
use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let artifact = args.next().ok_or("artifact path is required")?;
    let signature = args.next().ok_or("signature path is required")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }

    let public_key =
        env::var("TAURI_UPDATER_PUBKEY").map_err(|_| "TAURI_UPDATER_PUBKEY is required")?;
    let public_key = String::from_utf8(STANDARD.decode(public_key.trim())?)?;
    let public_key = PublicKey::decode(&public_key)?;
    let signature = fs::read_to_string(Path::new(&signature))?;
    let signature = String::from_utf8(STANDARD.decode(signature.trim())?)?;
    let signature = Signature::decode(&signature)?;
    let artifact = fs::read(Path::new(&artifact))?;
    public_key.verify(&artifact, &signature, false)?;
    Ok(())
}
