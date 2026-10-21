//! End to end through the real age plugin protocol (C2SP age-plugin), using age-core's
//! `Connection` as the client, exactly as the `age` crate does: start the binary with
//! `--age-plugin=recipient-v1` on member A's provider directory to wrap a file key, then
//! `--age-plugin=identity-v1` on member B's directory to unwrap it.

use age_core::format::Stanza;
use age_core::plugin::Connection;
use age_plugin_cahoots::{encode_identity, encode_recipient, STANZA_TAG};
use cahoots_core::params::CircleParams;
use cahoots_provider::{FileProvider, PadProvider};
use std::path::Path;

fn pad_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(41) ^ (i >> 1)) as u8)
        .collect()
}

fn bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_age-plugin-cahoots"))
}

type Wrapped = (Vec<Vec<Stanza>>, Vec<String>);

/// recipient-v1: returns (stanzas per file, labels) or the plugin's error messages.
fn wrap(dir: &Path, recipient: &str, file_keys: &[[u8; 16]]) -> Result<Wrapped, Vec<String>> {
    std::env::set_var("CAHOOTS_DIR", dir);
    let mut conn = Connection::open(bin(), "recipient-v1").unwrap();
    conn.unidir_send(|mut phase| {
        phase.send("add-recipient", &[recipient], &[])?;
        phase.send("extension-labels", &[], &[])?;
        for fk in file_keys {
            phase.send("wrap-file-key", &[], fk)?;
        }
        Ok(())
    })
    .unwrap();
    let mut stanzas: Vec<Vec<Stanza>> = (0..file_keys.len()).map(|_| Vec::new()).collect();
    let mut labels = Vec::new();
    let mut errors = Vec::new();
    conn.bidir_receive(
        &[
            "msg",
            "confirm",
            "request-public",
            "request-secret",
            "recipient-stanza",
            "labels",
            "error",
        ],
        |stanza, reply| {
            match stanza.tag.as_str() {
                "recipient-stanza" => {
                    let idx: usize = stanza.args[0].parse().unwrap();
                    stanzas[idx].push(Stanza {
                        tag: stanza.args[1].clone(),
                        args: stanza.args[2..].to_vec(),
                        body: stanza.body.clone(),
                    });
                }
                "labels" => labels = stanza.args.clone(),
                "error" => errors.push(format!(
                    "{:?} {}",
                    stanza.args,
                    String::from_utf8_lossy(&stanza.body)
                )),
                _ => {}
            }
            reply.ok(None)
        },
    )
    .unwrap();
    if errors.is_empty() {
        Ok((stanzas, labels))
    } else {
        Err(errors)
    }
}

/// identity-v1: returns the unwrapped file key per file index, or the error messages.
fn unwrap(
    dir: &Path,
    identity: &str,
    files: &[Vec<Stanza>],
) -> (Vec<Option<Vec<u8>>>, Vec<String>) {
    std::env::set_var("CAHOOTS_DIR", dir);
    let mut conn = Connection::open(bin(), "identity-v1").unwrap();
    conn.unidir_send(|mut phase| {
        phase.send("add-identity", &[identity], &[])?;
        for (file_index, stanzas) in files.iter().enumerate() {
            for s in stanzas {
                phase.send_stanza("recipient-stanza", &[&file_index.to_string()], s)?;
            }
        }
        Ok(())
    })
    .unwrap();
    let mut keys: Vec<Option<Vec<u8>>> = vec![None; files.len()];
    let mut errors = Vec::new();
    conn.bidir_receive(
        &[
            "msg",
            "confirm",
            "request-public",
            "request-secret",
            "file-key",
            "error",
        ],
        |stanza, reply| {
            match stanza.tag.as_str() {
                "file-key" => {
                    let idx: usize = stanza.args[0].parse().unwrap();
                    keys[idx] = Some(stanza.body.clone());
                }
                "error" => errors.push(format!(
                    "{:?} {}",
                    stanza.args,
                    String::from_utf8_lossy(&stanza.body)
                )),
                _ => {}
            }
            reply.ok(None)
        },
    )
    .unwrap();
    (keys, errors)
}

#[test]
fn two_members_wrap_and_unwrap_through_the_plugin_protocol() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let params = CircleParams::new(2, 4096, "age-e2e", false).unwrap();
    let cid = FileProvider::open(dir_a.path())
        .unwrap()
        .add_circle(params.clone(), 0, pad_bytes(4096))
        .unwrap();
    FileProvider::open(dir_b.path())
        .unwrap()
        .add_circle(params, 1, pad_bytes(4096))
        .unwrap();
    let recipient = encode_recipient(&cid);
    let identity = encode_identity(&cid);
    let file_keys = [[0x11u8; 16], [0x22u8; 16]];

    let (stanzas, labels) = wrap(dir_a.path(), &recipient, &file_keys).unwrap();
    assert_eq!(labels, vec!["postquantum".to_string()]);
    assert_eq!(stanzas.len(), 2);
    assert_eq!(stanzas[0].len(), 1);
    assert_eq!(stanzas[0][0].tag, STANZA_TAG);
    assert_eq!(
        FileProvider::open(dir_a.path()).unwrap().remaining(&cid),
        2048 - 64
    );

    let (keys, errors) = unwrap(dir_b.path(), &identity, &stanzas);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(keys[0].as_deref(), Some(&file_keys[0][..]));
    assert_eq!(keys[1].as_deref(), Some(&file_keys[1][..]));
    assert_eq!(
        FileProvider::open(dir_b.path()).unwrap().remaining(&cid),
        2048
    );

    // Replay: the same stanzas can't be unwrapped twice on B.
    let (keys, errors) = unwrap(dir_b.path(), &identity, &stanzas);
    assert!(keys.iter().all(Option::is_none));
    assert!(
        errors.iter().any(|e| e.contains("already spent")),
        "{errors:?}"
    );

    // A directory without this circle reports it, and unwraps nothing.
    let dir_c = tempfile::tempdir().unwrap();
    let (keys, errors) = unwrap(dir_c.path(), &identity, &stanzas);
    assert!(keys.iter().all(Option::is_none));
    assert!(
        errors.iter().any(|e| e.contains("no pad for this circle")),
        "{errors:?}"
    );

    // A recipient for an unknown circle fails to wrap.
    let err = wrap(dir_c.path(), &recipient, &file_keys).unwrap_err();
    assert!(
        err.iter().any(|e| e.contains("no pad for this circle")),
        "{err:?}"
    );
}
