//! age plugin binary. age clients start it with `--age-plugin=<state-machine>`; users run
//! it directly to print the recipient and identity strings for a circle.
//!
//! ```text
//! export CAHOOTS_DIR=~/.cahoots
//! age-plugin-cahoots --circle <hex>          # prints recipient + identity
//! age -r age1cahoots1... -o out.age in.bin
//! age -d -i identity.txt out.age
//! ```

use age_core::format::{FileKey, Stanza};
use age_plugin::identity::{self, IdentityPluginV1};
use age_plugin::recipient::{self, RecipientPluginV1};
use age_plugin::{run_state_machine, Callbacks, PluginHandler};
use age_plugin_cahoots::{
    circle_from_bytes, encode_identity, encode_recipient, unwrap_stanza, wrap_file_key, PluginError,
};
use cahoots_provider::{CircleId, FileProvider};
use clap::Parser;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "age-plugin-cahoots",
    about = "age plugin for Cahoots circle pads"
)]
struct Opts {
    /// Run the given age plugin state machine (set by the age client).
    #[arg(long)]
    age_plugin: Option<String>,
    /// Print the recipient and identity strings for a circle ID (hex).
    #[arg(long)]
    circle: Option<String>,
    /// Provider directory.
    #[arg(long, env = "CAHOOTS_DIR")]
    dir: Option<PathBuf>,
}

fn provider_dir(dir: Option<PathBuf>) -> PathBuf {
    dir.unwrap_or_else(|| {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        home.join(".cahoots")
    })
}

struct Handler {
    dir: PathBuf,
}

impl PluginHandler for Handler {
    type RecipientV1 = Recipient;
    type IdentityV1 = Identity;
    fn recipient_v1(self) -> io::Result<Recipient> {
        Ok(Recipient {
            dir: self.dir,
            circles: Vec::new(),
        })
    }
    fn identity_v1(self) -> io::Result<Identity> {
        Ok(Identity {
            dir: self.dir,
            circles: Vec::new(),
        })
    }
}

fn open(dir: &PathBuf) -> io::Result<FileProvider> {
    FileProvider::open(dir).map_err(|e| io::Error::other(format!("{e:?}")))
}

struct Recipient {
    dir: PathBuf,
    /// (index, circle) for every recipient or identity the client added.
    circles: Vec<(usize, CircleId)>,
}

impl RecipientPluginV1 for Recipient {
    fn add_recipient(
        &mut self,
        index: usize,
        _plugin_name: &str,
        bytes: &[u8],
    ) -> Result<(), recipient::Error> {
        let circle = circle_from_bytes(bytes).map_err(|_| recipient::Error::Recipient {
            index,
            message: "malformed cahoots recipient".into(),
        })?;
        self.circles.push((index, circle));
        Ok(())
    }

    fn add_identity(
        &mut self,
        index: usize,
        _plugin_name: &str,
        bytes: &[u8],
    ) -> Result<(), recipient::Error> {
        let circle = circle_from_bytes(bytes).map_err(|_| recipient::Error::Identity {
            index,
            message: "malformed cahoots identity".into(),
        })?;
        self.circles.push((index, circle));
        Ok(())
    }

    fn labels(&mut self) -> HashSet<String> {
        ["postquantum".to_string()].into_iter().collect()
    }

    fn wrap_file_keys(
        &mut self,
        file_keys: Vec<FileKey>,
        _callbacks: impl Callbacks<recipient::Error>,
    ) -> io::Result<Result<Vec<Vec<Stanza>>, Vec<recipient::Error>>> {
        let mut provider = open(&self.dir)?;
        let mut errors = Vec::new();
        let mut out = Vec::with_capacity(file_keys.len());
        for fk in &file_keys {
            let mut stanzas = Vec::new();
            for (index, circle) in &self.circles {
                match wrap_file_key(&mut provider, circle, fk) {
                    Ok(s) => stanzas.push(s),
                    Err(e) => errors.push(recipient::Error::Recipient {
                        index: *index,
                        message: describe(&e),
                    }),
                }
            }
            out.push(stanzas);
        }
        Ok(if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors)
        })
    }
}

struct Identity {
    dir: PathBuf,
    circles: Vec<CircleId>,
}

impl IdentityPluginV1 for Identity {
    fn add_identity(
        &mut self,
        index: usize,
        _plugin_name: &str,
        bytes: &[u8],
    ) -> Result<(), identity::Error> {
        let circle = circle_from_bytes(bytes).map_err(|_| identity::Error::Identity {
            index,
            message: "malformed cahoots identity".into(),
        })?;
        self.circles.push(circle);
        Ok(())
    }

    fn unwrap_file_keys(
        &mut self,
        files: Vec<Vec<Stanza>>,
        _callbacks: impl Callbacks<identity::Error>,
    ) -> io::Result<HashMap<usize, Result<FileKey, Vec<identity::Error>>>> {
        let mut provider = open(&self.dir)?;
        let mut result = HashMap::new();
        for (file_index, stanzas) in files.iter().enumerate() {
            let mut errors = Vec::new();
            for (stanza_index, stanza) in stanzas.iter().enumerate() {
                match unwrap_stanza(&mut provider, &self.circles, stanza) {
                    Ok(Some(fk)) => {
                        result.insert(file_index, Ok(fk));
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => errors.push(identity::Error::Stanza {
                        file_index,
                        stanza_index,
                        message: describe(&e),
                    }),
                }
            }
            if !errors.is_empty() && !result.contains_key(&file_index) {
                result.insert(file_index, Err(errors));
            }
        }
        Ok(result)
    }
}

fn describe(e: &PluginError) -> String {
    match e {
        PluginError::Encoding => "malformed cahoots stanza".into(),
        PluginError::Pad(cahoots_provider::PadError::AlreadySpent) => {
            "pad range already spent: this file key was unwrapped before, or the range was reused"
                .into()
        }
        PluginError::Pad(cahoots_provider::PadError::UnknownCircle) => {
            "no pad for this circle in CAHOOTS_DIR".into()
        }
        PluginError::Pad(cahoots_provider::PadError::Exhausted) => {
            "pad slice exhausted: hold a new ceremony".into()
        }
        PluginError::Pad(p) => format!("pad error: {p:?}"),
        PluginError::Unwrap => "file key did not authenticate: wrong pad or tampered file".into(),
    }
}

fn main() -> io::Result<()> {
    let opts = Opts::parse();
    let dir = provider_dir(opts.dir);
    if let Some(sm) = opts.age_plugin {
        return run_state_machine(&sm, Handler { dir });
    }
    if let Some(hex_id) = opts.circle {
        let bytes = hex::decode(&hex_id).map_err(|e| io::Error::other(e.to_string()))?;
        let circle = circle_from_bytes(&bytes)
            .map_err(|_| io::Error::other("circle id must be 16 bytes"))?;
        println!(
            "# recipient (give to senders)\n{}",
            encode_recipient(&circle)
        );
        println!(
            "# identity (keep in an identity file; CAHOOTS_DIR holds the pad)\n{}",
            encode_identity(&circle)
        );
        return Ok(());
    }
    eprintln!("usage: age-plugin-cahoots --circle <hex>   (or started by an age client)");
    std::process::exit(2);
}
