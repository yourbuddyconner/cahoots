//! PC/SC transport. On macOS this links the system PCSC framework; on Linux it needs
//! `libpcsclite` and a running `pcscd`.

use crate::{Card, YkError};
use pcsc::{Context, Error, Protocols, Scope, ShareMode, MAX_BUFFER_SIZE_EXTENDED};

pub struct PcscCard {
    card: pcsc::Card,
    reader: String,
}

fn map(e: Error) -> YkError {
    match e {
        Error::NoSmartcard | Error::RemovedCard | Error::ResetCard | Error::NoService => {
            YkError::CardRemoved
        }
        Error::NoReadersAvailable | Error::UnknownReader | Error::ReaderUnavailable => {
            YkError::NoReader
        }
        other => YkError::Pcsc(other.to_string()),
    }
}

/// Names of all connected readers.
pub fn readers() -> Result<Vec<String>, YkError> {
    let ctx = Context::establish(Scope::User).map_err(map)?;
    let mut buf = vec![0u8; ctx.list_readers_len().map_err(map)?];
    Ok(ctx
        .list_readers(&mut buf)
        .map_err(map)?
        .map(|r| r.to_string_lossy().into_owned())
        .collect())
}

impl PcscCard {
    /// Connect to the first reader that holds a card.
    pub fn connect_first() -> Result<Self, YkError> {
        let ctx = Context::establish(Scope::User).map_err(map)?;
        let mut buf = vec![0u8; ctx.list_readers_len().map_err(map)?];
        let mut readers = ctx.list_readers(&mut buf).map_err(map)?.peekable();
        if readers.peek().is_none() {
            return Err(YkError::NoReader);
        }
        for reader in readers {
            match ctx.connect(reader, ShareMode::Shared, Protocols::ANY) {
                Ok(card) => {
                    return Ok(PcscCard {
                        card,
                        reader: reader.to_string_lossy().into_owned(),
                    })
                }
                Err(Error::NoSmartcard) => continue,
                Err(e) => return Err(map(e)),
            }
        }
        Err(YkError::NoCard)
    }

    pub fn reader(&self) -> &str {
        &self.reader
    }
}

impl Card for PcscCard {
    fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, YkError> {
        let mut resp = vec![0u8; MAX_BUFFER_SIZE_EXTENDED];
        let n = self.card.transmit(apdu, &mut resp).map_err(map)?.len();
        resp.truncate(n);
        Ok(resp)
    }
}
