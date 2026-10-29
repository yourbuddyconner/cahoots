//! `cahoots-host sign --file <path> (--circle <name> | --key <pub>) [--namespace <ns>] [--out <path>]`:
//! an SSH signature over a file by a badge's seat key, the way `ssh-keygen -Y sign` makes
//! one, through the agent. A file of up to [`SHOW_LIMIT`] bytes goes to the badge, which
//! checks the hash and shows the file's first line (or "binary") and its size; a bigger
//! file is signed by its hash, and the badge shows that hash marked as the laptop's word.
//! The signature is `<file>.sig`, which `ssh-keygen -Y verify` checks.

use crate::agent;
use crate::gitsign::{armor, sign_one};
use std::path::{Path, PathBuf};

/// The largest file the badge is shown whole: under the request cap, with room for the
/// envelope.
pub const SHOW_LIMIT: usize = 12 * 1024;

/// The default namespace for a file signature.
pub const FILE_NAMESPACE: &str = "file";

pub struct Signed {
    /// Where the armored signature was written.
    pub sig_path: PathBuf,
    /// The key, as an `allowed_signers` line wants it: `<type> <base64>`.
    pub key_line: String,
    /// Whether the badge saw the whole file.
    pub shown: bool,
}

/// Find the agent's key for `circle` (comment `cahoots:<circle>:<seat>`).
pub fn key_for_circle(socket: &Path, circle: &str) -> Result<ssh_key::PublicKey, String> {
    let prefix = format!("cahoots:{circle}:");
    let ids = agent::list_identities(socket)?;
    let (blob, _) = ids
        .into_iter()
        .find(|(_, c)| c.starts_with(&prefix))
        .ok_or_else(|| format!("the agent holds no key for a circle named {circle}"))?;
    ssh_key::PublicKey::from_bytes(&blob).map_err(|e| e.to_string())
}

pub fn sign_file(
    socket: &Path,
    key: &ssh_key::PublicKey,
    namespace: &str,
    file: &Path,
    out: Option<&Path>,
) -> Result<Signed, String> {
    let content = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let (armored, shown) = if content.len() <= SHOW_LIMIT {
        (sign_one(socket, key, namespace, &content)?, true)
    } else {
        let data = ssh_key::SshSig::signed_data(namespace, ssh_key::HashAlg::Sha512, &content)
            .map_err(|e| e.to_string())?;
        let blob = key.to_bytes().map_err(|e| e.to_string())?;
        let sig = agent::sign_plain(socket, &blob, &data)?;
        (armor(key, namespace, &sig)?, false)
    };
    let sig_path = out.map(Path::to_path_buf).unwrap_or_else(|| {
        let mut p = file.as_os_str().to_owned();
        p.push(".sig");
        PathBuf::from(p)
    });
    std::fs::write(&sig_path, armored).map_err(|e| format!("{}: {e}", sig_path.display()))?;
    let key_line = key.to_openssh().map_err(|e| e.to_string())?;
    Ok(Signed {
        sig_path,
        key_line,
        shown,
    })
}
