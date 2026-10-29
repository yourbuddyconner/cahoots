//! `cahoots-host-git-sign`: the program git runs as `gpg.ssh.program`.
//!
//! git signs a commit by writing its text to a file and running
//! `<program> -Y sign -n git -f <key file> [-U] <file>`; it then reads the signature from
//! `<file>.sig`. With no file the text is read from standard input and the signature written
//! to standard output, as `ssh-keygen -Y sign` does. To sign, this program builds the `SSHSIG`
//! envelope (the namespace and the SHA-512 of the text), and asks the agent at
//! `SSH_AUTH_SOCK` (`cahoots-host agent`) to have the badge sign it, sending the text too so
//! the badge can show the commit's author and subject. The key file names the seat key, as
//! `ssh-add -L` prints it; the agent signs only with a key it holds.
//!
//! Every other `-Y` operation (`verify`, `find-principals`, `check-novalidate`, which git runs
//! for `git log --show-signature`) needs no key and is handed to `ssh-keygen` as it is.

use crate::agent;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The environment variable that names the agent's socket ahead of `SSH_AUTH_SOCK`.
pub const AGENT_SOCK_ENV: &str = "CAHOOTS_AGENT_SOCK";

/// Run with `args` (without the program name). Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    let op = args
        .iter()
        .position(|a| a == "-Y")
        .and_then(|i| args.get(i + 1));
    match op.map(String::as_str) {
        Some("sign") => match sign(args) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("cahoots-host-git-sign: {e}");
                1
            }
        },
        _ => ssh_keygen(args),
    }
}

/// Hand the call to `ssh-keygen` with the same arguments and the same standard streams.
fn ssh_keygen(args: &[String]) -> i32 {
    match Command::new("ssh-keygen").args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("cahoots-host-git-sign: ssh-keygen: {e}");
            1
        }
    }
}

/// `-Y sign -n <namespace> -f <key file> [-U] [-O option]... [file]...`
fn sign(args: &[String]) -> Result<(), String> {
    let mut namespace = None;
    let mut keyfile = None;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-Y" => {
                it.next();
            }
            "-n" => namespace = it.next().cloned(),
            "-f" => keyfile = it.next().map(PathBuf::from),
            // -O takes an option this program does not use; -U says the key is in an agent,
            // which it always is here.
            "-O" => {
                it.next();
            }
            "-U" | "-q" => {}
            f if !f.starts_with('-') || f == "-" => files.push(f.to_string()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    let namespace = namespace.ok_or("no namespace (-n)")?;
    let keyfile = keyfile.ok_or("no key file (-f)")?;
    let key = read_key(&keyfile)?;
    let socket = std::env::var_os(AGENT_SOCK_ENV)
        .or_else(|| std::env::var_os("SSH_AUTH_SOCK"))
        .map(PathBuf::from)
        .ok_or("no agent: set SSH_AUTH_SOCK to the socket `cahoots-host agent` printed")?;

    if files.is_empty() || files == ["-"] {
        let mut content = Vec::new();
        std::io::stdin()
            .read_to_end(&mut content)
            .map_err(|e| format!("standard input: {e}"))?;
        let armored = sign_one(&socket, &key, &namespace, &content)?;
        std::io::stdout()
            .write_all(armored.as_bytes())
            .map_err(|e| format!("standard output: {e}"))?;
        return Ok(());
    }
    for file in &files {
        let content = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
        let armored = sign_one(&socket, &key, &namespace, &content)?;
        let out = format!("{file}.sig");
        std::fs::write(&out, armored).map_err(|e| format!("{out}: {e}"))?;
    }
    Ok(())
}

/// The public key in `path`, as `ssh-add -L` prints it, or in `<path>.pub` when `path` is a
/// private key file.
pub fn read_key(path: &Path) -> Result<ssh_key::PublicKey, String> {
    let parse = |p: &Path| -> Option<ssh_key::PublicKey> {
        let text = std::fs::read_to_string(p).ok()?;
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))?;
        ssh_key::PublicKey::from_openssh(line).ok()
    };
    let mut pub_path = path.as_os_str().to_owned();
    pub_path.push(".pub");
    parse(path)
        .or_else(|| parse(Path::new(&pub_path)))
        .ok_or_else(|| format!("{}: no public key in it", path.display()))
}

/// One armored signature over `content`, with the content shown to the badge.
pub fn sign_one(
    socket: &Path,
    key: &ssh_key::PublicKey,
    namespace: &str,
    content: &[u8],
) -> Result<String, String> {
    let hash = ssh_key::HashAlg::Sha512;
    let data = ssh_key::SshSig::signed_data(namespace, hash, content).map_err(|e| e.to_string())?;
    let blob = key.to_bytes().map_err(|e| e.to_string())?;
    let sig = agent::sign_with_content(socket, &blob, &data, content)?;
    armor(key, namespace, &sig)
}

/// The `-----BEGIN SSH SIGNATURE-----` text for `sig` (an SSH signature blob) by `key`.
pub fn armor(key: &ssh_key::PublicKey, namespace: &str, sig: &[u8]) -> Result<String, String> {
    use ssh_encoding::Decode;
    let sig = ssh_key::Signature::decode(&mut &sig[..]).map_err(|e| format!("signature: {e}"))?;
    let sshsig = ssh_key::SshSig::new(
        key.key_data().clone(),
        namespace,
        ssh_key::HashAlg::Sha512,
        sig,
    )
    .map_err(|e| e.to_string())?;
    let mut pem = sshsig
        .to_pem(ssh_key::LineEnding::LF)
        .map_err(|e| e.to_string())?;
    if !pem.ends_with('\n') {
        pem.push('\n');
    }
    Ok(pem)
}
