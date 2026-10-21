//! `cahoots-yk`: YubiKey randomness over the OpenPGP card application (handoff 3.3.1).
//!
//! Sends `SELECT` for AID `D2 76 00 01 24 01`, reads the maximum challenge length from
//! the extended-capabilities DO, then loops `GET CHALLENGE` (`00 84 00 00 Le`). This is
//! the same path `gpg-connect-agent "SCD RANDOM n" /bye` uses. No PIN is needed.
//!
//! Every failure is fail-closed: a removed card, a short read, a non-`9000` status
//! word, or a health-check failure aborts the read and the caller gets no bytes.

pub mod health;
pub mod pcsc_card;

use health::{Health, HealthCheck};

pub const OPENPGP_AID: [u8; 6] = [0xD2, 0x76, 0x00, 0x01, 0x24, 0x01];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum YkError {
    NoReader,
    NoCard,
    CardRemoved,
    /// Non-`9000` status word.
    Status(u16),
    ShortRead {
        want: usize,
        got: usize,
    },
    /// Extended capabilities say the card does not support `GET CHALLENGE`.
    NoChallengeSupport,
    /// Could not parse application-related data.
    Malformed,
    Health(HealthCheck),
    Pcsc(String),
}

impl std::fmt::Display for YkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YkError::NoReader => write!(f, "no smart card reader found"),
            YkError::NoCard => write!(f, "no card in any reader"),
            YkError::CardRemoved => write!(f, "card removed or stopped responding"),
            YkError::Status(sw) => write!(f, "card returned status {sw:04X}"),
            YkError::ShortRead { want, got } => {
                write!(f, "short read: wanted {want} bytes, got {got}")
            }
            YkError::NoChallengeSupport => write!(f, "card does not support GET CHALLENGE"),
            YkError::Malformed => write!(f, "could not parse card data"),
            YkError::Health(h) => write!(f, "health check failed: {h:?}"),
            YkError::Pcsc(e) => write!(f, "PC/SC: {e}"),
        }
    }
}

impl std::error::Error for YkError {}

/// A transport that sends one APDU and returns the full response including the status word.
pub trait Card {
    fn transmit(&self, apdu: &[u8]) -> Result<Vec<u8>, YkError>;
}

/// An opened OpenPGP application on a card.
#[derive(Debug)]
pub struct OpenPgp<C: Card> {
    card: C,
    max_challenge: u16,
}

fn split_status(resp: &[u8]) -> Result<(&[u8], u16), YkError> {
    if resp.len() < 2 {
        return Err(YkError::CardRemoved);
    }
    let (body, sw) = resp.split_at(resp.len() - 2);
    Ok((body, u16::from_be_bytes([sw[0], sw[1]])))
}

/// Send an APDU, follow `61xx` GET RESPONSE chaining, and require `9000`.
fn exchange<C: Card>(card: &C, apdu: &[u8]) -> Result<Vec<u8>, YkError> {
    let resp = card.transmit(apdu)?;
    let (body, sw) = split_status(&resp)?;
    let mut out = body.to_vec();
    let mut sw = sw;
    while sw >> 8 == 0x61 {
        let more = card.transmit(&[0x00, 0xC0, 0x00, 0x00, (sw & 0xFF) as u8])?;
        let (body, next) = split_status(&more)?;
        out.extend_from_slice(body);
        sw = next;
    }
    if sw != 0x9000 {
        return Err(YkError::Status(sw));
    }
    Ok(out)
}

/// Walk a BER-TLV sequence and return the value of the first DO with `tag`,
/// descending into constructed DOs.
fn find_do(mut data: &[u8], tag: u16) -> Option<&[u8]> {
    while !data.is_empty() {
        let (t, rest) = if data[0] & 0x1F == 0x1F {
            (u16::from_be_bytes([data[0], *data.get(1)?]), &data[2..])
        } else {
            (u16::from(data[0]), &data[1..])
        };
        let (len, rest) = match *rest.first()? {
            0x81 => (usize::from(*rest.get(1)?), &rest[2..]),
            0x82 => (
                usize::from(u16::from_be_bytes([*rest.get(1)?, *rest.get(2)?])),
                &rest[3..],
            ),
            l if l < 0x80 => (usize::from(l), &rest[1..]),
            _ => return None,
        };
        let value = rest.get(..len)?;
        if t == tag {
            return Some(value);
        }
        let constructed = (t >> 8) & 0x20 != 0 || (t <= 0xFF && t & 0x20 != 0);
        if constructed {
            if let Some(v) = find_do(value, tag) {
                return Some(v);
            }
        }
        data = &rest[len..];
    }
    None
}

impl<C: Card> OpenPgp<C> {
    /// Select the OpenPGP application and read its maximum challenge length.
    pub fn open(card: C) -> Result<Self, YkError> {
        let mut select = vec![0x00, 0xA4, 0x04, 0x00, OPENPGP_AID.len() as u8];
        select.extend_from_slice(&OPENPGP_AID);
        select.push(0x00);
        exchange(&card, &select)?;
        let app_data = exchange(&card, &[0x00, 0xCA, 0x00, 0x6E, 0x00])?;
        // 6E is itself a constructed DO; search from the top.
        let caps = find_do(&app_data, 0xC0).ok_or(YkError::Malformed)?;
        if caps.len() < 4 {
            return Err(YkError::Malformed);
        }
        let max_challenge = u16::from_be_bytes([caps[2], caps[3]]);
        if max_challenge == 0 {
            return Err(YkError::NoChallengeSupport);
        }
        Ok(OpenPgp {
            card,
            max_challenge,
        })
    }

    pub fn max_challenge(&self) -> u16 {
        self.max_challenge
    }

    pub fn card(&self) -> &C {
        &self.card
    }

    fn get_challenge_apdu(le: usize) -> Vec<u8> {
        let mut apdu = vec![0x00, 0x84, 0x00, 0x00];
        if le <= 256 {
            apdu.push((le % 256) as u8);
        } else {
            apdu.extend_from_slice(&[0x00, (le >> 8) as u8, (le & 0xFF) as u8]);
        }
        apdu
    }

    /// Fill `buf` with card randomness, or fail closed. On error, `buf` MUST NOT be used:
    /// it is zeroed before returning.
    pub fn random(&mut self, buf: &mut [u8]) -> Result<(), YkError> {
        let result = self.random_inner(buf);
        if result.is_err() {
            buf.fill(0);
        }
        result
    }

    fn random_inner(&mut self, buf: &mut [u8]) -> Result<(), YkError> {
        let mut health = Health::new();
        let mut filled = 0;
        while filled < buf.len() {
            let want = (buf.len() - filled).min(usize::from(self.max_challenge));
            let chunk = exchange(&self.card, &Self::get_challenge_apdu(want))?;
            if chunk.len() != want {
                return Err(YkError::ShortRead {
                    want,
                    got: chunk.len(),
                });
            }
            health.check(&chunk).map_err(YkError::Health)?;
            buf[filled..filled + want].copy_from_slice(&chunk);
            filled += want;
        }
        Ok(())
    }
}

impl<C: Card> cahoots_hal::Entropy for OpenPgp<C> {
    fn fill(&mut self, buf: &mut [u8]) -> Result<(), cahoots_hal::HealthError> {
        self.random(buf).map_err(|e| match e {
            YkError::Health(HealthCheck::RepetitionCount) => {
                cahoots_hal::HealthError::RepetitionCount
            }
            YkError::Health(_) => cahoots_hal::HealthError::AdaptiveProportion,
            _ => cahoots_hal::HealthError::SourceUnavailable,
        })
    }
}
