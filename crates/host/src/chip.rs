//! `cahoots-host chip`: the phone's own chip client (`cahoots_phone::nfc`) run from the laptop
//! against a real badge's chip, through a bench firmware's APDU pass-through (frame kind 32).
//! The badge raises the chip's pin for the exchange and forwards each APDU over I2C in no
//! session, so the laptop meets the applet as a phone's tap does.
//!
//! ```text
//! cahoots-host chip pairing --serial <port>
//! cahoots-host chip inbox --serial <port> --name <text> --key <hex> [--state <dir>]
//! cahoots-host chip open --serial <port> --key <hex>
//! cahoots-host chip hmac --serial <port> --key <hex> --circle <slot> --label <text>
//! cahoots-host chip public --serial <port>
//! cahoots-host chip refusals --serial <port> --circle <slot>
//! cahoots-host chip raw --serial <port> --apdu <hex> [--apdu <hex>]...
//! cahoots-host chip goto --serial <port> --entry <name>
//! cahoots-host chip press --serial <port> (left|right|both) [--after <text>] [--until <text> [--expect <text>] [--then <text>]]
//! cahoots-host chip forget --serial <port> --circle <name>
//! ```
//!
//! `--key` is the phone slot's 16-byte AES key, in hex. `inbox` signs with a software P-256 key
//! kept in `--state` (`phone.key`, made on first use), the laptop's stand-in for the phone's
//! enclave key. `refusals` tries, in no session, what an unpaired tap must not do, and says
//! what the applet answered to each. `goto` presses through the badge's menu and list to an
//! entry of the list ("pair a phone", "phone") and opens it; `press` waits for a log line
//! holding `--after` if given, sends one press, and waits for a line holding `--until` (see
//! [`press`] for `--expect` and `--then`); `forget` forgets a circle through the badge's pages.
//!
//! What the bench showed an unpaired tap can do: read the public objects and the pairing
//! object, write the inbox, and make a binary object at any id that holds nothing, under the
//! applet's default policy (every right for every session). The badge never adopts such an
//! object: it writes a circle's public object at the keep after deleting whatever stands at
//! its id, and reads back only public objects it wrote. But a tap can fill the ids of circle
//! slots not yet used with objects nobody may delete, and each such slot is then burnt: the
//! badge passes it over when it picks a slot for a new circle (see `cahoots_badge::chip`).
//! That is a denial of service on the badge's sixteen circle slots, not a way to any key.
//!
//! Every APDU and its answer are printed to stderr, as `-> ` and `<- ` hex lines.

use crate::link::SerialLink;
use cahoots_badge::public::PublicObject;
use cahoots_chip::{KeyId, KeyKind};
use cahoots_core::seat::SeatKey;
use cahoots_phone::nfc::{self, Nfc, Tap, TapError};
use cahoots_phone::PhoneError;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The NFC reader as the phone crate sees it, over the bench pass-through. A tap begins and
/// ends with an empty `Apdu` frame, which drops the chip's pin, so each tap finds the chip
/// fresh, in no session.
pub struct SerialNfc {
    link: Mutex<SerialLink>,
}

impl SerialNfc {
    pub fn open(port: &str) -> Result<SerialNfc, String> {
        let link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
        Ok(SerialNfc { link: Mutex::new(link) })
    }

    fn exchange(&self, apdu: &[u8]) -> Result<Vec<u8>, PhoneError> {
        let mut link = self.link.lock().map_err(|_| PhoneError::NoCard)?;
        link.apdu(apdu).map_err(|e| PhoneError::Nfc { message: e.to_string() })
    }
}

impl Nfc for SerialNfc {
    fn begin(&self, _prompt: String) -> Result<(), PhoneError> {
        self.exchange(&[]).map(|_| ())
    }

    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        eprintln!("-> {}", hex::encode(&apdu));
        let answer = self.exchange(&apdu)?;
        if answer.is_empty() {
            eprintln!("<- nothing");
            return Err(PhoneError::NoCard);
        }
        eprintln!("<- {}", hex::encode(&answer));
        Ok(answer)
    }

    fn end(&self, _message: String, _failed: bool) -> Result<(), PhoneError> {
        self.exchange(&[]).map(|_| ())
    }
}

/// A tap's failure in plain words.
fn tap_error(e: TapError) -> String {
    match e {
        TapError::NotPaired => "the card does not open with this key".into(),
        TapError::Status(sw) => format!("the card answered {sw:04x}"),
        e => PhoneError::from(e).to_string(),
    }
}

fn key16(hex_key: &str) -> Result<[u8; 16], String> {
    hex::decode(hex_key)
        .ok()
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| "--key: 16 bytes in hex".to_string())
}

/// The laptop's stand-in for the phone's enclave key, kept in `dir/phone.key`.
fn phone_key(dir: &Path) -> Result<SeatKey, String> {
    let path = dir.join("phone.key");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let seed: [u8; 32] = hex::decode(text.trim())
            .ok()
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| format!("{}: not a key", path.display()))?;
        return SeatKey::from_seed(&seed).map_err(|_| format!("{}: not a key", path.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    loop {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
        if let Ok(k) = SeatKey::from_seed(&seed) {
            std::fs::write(&path, hex::encode(seed)).map_err(|e| format!("{}: {e}", path.display()))?;
            return Ok(k);
        }
    }
}

fn describe(slot: u8, p: &PublicObject) -> String {
    let phone = p.phone_public.map(hex::encode).unwrap_or_else(|| "none".into());
    format!(
        "slot {slot}: circle {}, generation {}, device grant {}, phone key {phone}, pending bumps {}",
        hex::encode(p.circle_id),
        hex::encode(&p.generation[..8]),
        if p.grant.is_some() { "yes" } else { "no" },
        p.pending_bumps.len()
    )
}

/// Run `cahoots-host chip <sub> ...`. `args` starts at the subcommand.
pub fn run(args: &[String]) -> Result<(), String> {
    let get = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let req = |key: &str| get(key).ok_or_else(|| format!("missing {key}"));
    let sub = args.first().map(String::as_str).unwrap_or("");
    let port = req("--serial")?;
    match sub {
        "goto" => return goto(port, req("--entry")?),
        "forget" => return forget(port, req("--circle")?),
        "press" => {
            let code = match args.get(1).map(String::as_str) {
                Some("left") => 0,
                Some("right") => 1,
                Some("both") => 2,
                _ => return Err("press: left, right or both".into()),
            };
            return press(
                port,
                code,
                Presses { after: get("--after"), until: get("--until"), expect: get("--expect"), then: get("--then") },
            );
        }
        _ => {}
    }
    let reader = SerialNfc::open(port)?;
    let mut tap = Tap::begin(&reader, "").map_err(tap_error)?;
    let result = match sub {
        "pairing" => {
            let p = tap.read_pairing().map_err(tap_error)?;
            if p.nonce == [0; 16] {
                println!("pairing: none open");
            } else {
                println!("pairing: open, nonce {}", hex::encode(p.nonce));
                println!("badge key: {}", hex::encode(p.badge_key));
                println!("badge name: {}", p.name);
            }
            Ok(())
        }
        "inbox" => {
            let name = req("--name")?;
            let key = key16(req("--key")?)?;
            let state = PathBuf::from(get("--state").unwrap_or("/tmp/cahoots-chip"));
            let signer = phone_key(&state)?;
            let public = signer.public().0;
            let p = tap.read_pairing().map_err(tap_error)?;
            let inbox = nfc::sealed_inbox(&signer, &p, name, &key).map_err(tap_error)?;
            tap.write_inbox(&inbox).map_err(tap_error)?;
            let words = cahoots_badge::pairing::words(&p.nonce, &p.badge_key, &public, &key);
            println!("inbox written for {name}");
            println!("phone key: {}", hex::encode(public));
            println!("words: {}", words.join(" "));
            Ok(())
        }
        "open" => {
            let key = key16(req("--key")?)?;
            let mut s = tap.open_slot(&key).map_err(tap_error)?;
            let v = s.version().map_err(tap_error)?;
            s.close().map_err(tap_error)?;
            println!("slot session open, applet {}.{}.{} answered in it", v[0], v[1], v[2]);
            Ok(())
        }
        "hmac" => {
            let key = key16(req("--key")?)?;
            let slot: u8 = req("--circle")?.parse().map_err(|_| "--circle: a slot, 0 to 15")?;
            let label = req("--label")?;
            let mut s = tap.open_slot(&key).map_err(tap_error)?;
            // A command with no data first: the MAC after it shows the applet counts it in
            // the encryption counter.
            s.version().map_err(tap_error)?;
            let mac = s.hmac_root(slot, label.as_bytes()).map_err(tap_error)?;
            s.close().map_err(tap_error)?;
            println!("hmac: {}", hex::encode(mac));
            Ok(())
        }
        "public" => {
            let all = tap.list_public().map_err(tap_error)?;
            if all.is_empty() {
                println!("no public objects");
            }
            for (slot, p) in &all {
                println!("{}", describe(*slot, p));
            }
            Ok(())
        }
        "refusals" => {
            let slot: u8 = req("--circle")?.parse().map_err(|_| "--circle: a slot, 0 to 15")?;
            refusals(&reader, slot)
        }
        "raw" => {
            // Each `--apdu` in turn, in one tap, in no session: for probing the applet.
            let all: Vec<&String> = args.windows(2).filter(|w| w[0] == "--apdu").map(|w| &w[1]).collect();
            for a in all {
                let bytes = hex::decode(a).map_err(|_| format!("--apdu {a}: not hex"))?;
                let t = Instant::now();
                let answer = reader.transceive(bytes).map_err(|e| e.to_string())?;
                println!("{} in {} ms", hex::encode(&answer), t.elapsed().as_millis());
            }
            Ok(())
        }
        _ => Err(format!("unknown chip command {sub}")),
    };
    let _ = tap.end("", result.is_err());
    result
}

/// What an unpaired tap must not do to the circle in `slot` and the badge's own objects, tried
/// in no session, each with the applet's answer. `Err` if any is allowed. Nothing is tried at
/// an id that holds nothing: the applet lets any session make an object there.
fn refusals(reader: &SerialNfc, slot: u8) -> Result<(), String> {
    use cahoots_phone::apdu::{tlv, Apdu};
    let id = |k: KeyKind, s: u8| KeyId::new(k, s).0.to_be_bytes();
    let cmd = |ins: u8, p1: u8, p2: u8, data: Vec<u8>, le: Option<usize>| Apdu { cla: 0x80, ins, p1, p2, data, le };
    let cat = |parts: &[Vec<u8>]| parts.concat();
    let tries: Vec<(&str, Apdu)> = vec![
        (
            "MAC with the circle's root",
            cmd(0x03, 0x0D, 0x45, cat(&[tlv(0x41, &id(KeyKind::Root, slot)), tlv(0x42, &[0x19]), tlv(0x43, b"chat/gen")]), Some(256)),
        ),
        (
            "MAC with the circle's hello object for seat 0",
            cmd(0x03, 0x0D, 0x45, cat(&[tlv(0x41, &id(KeyKind::Hello, slot * 8)), tlv(0x42, &[0x19]), tlv(0x43, b"x")]), Some(256)),
        ),
        (
            "sign with the circle's seat key",
            cmd(0x03, 0x0C, 0x09, cat(&[tlv(0x41, &id(KeyKind::Seat, slot)), tlv(0x42, &[0x21]), tlv(0x43, &[7; 32])]), Some(256)),
        ),
        (
            "make an AES key at the phone slot's id",
            cmd(0x41, 0x03, 0x00, cat(&[tlv(0x41, &KeyId::SLOT.0.to_be_bytes()), tlv(0x43, &[9; 16])]), None),
        ),
        ("delete the phone slot", cmd(0x04, 0x00, 0x28, tlv(0x41, &KeyId::SLOT.0.to_be_bytes()), None)),
        ("delete the owner object", cmd(0x04, 0x00, 0x28, tlv(0x41, &KeyId::OWNER.0.to_be_bytes()), None)),
        ("delete the circle's root", cmd(0x04, 0x00, 0x28, tlv(0x41, &id(KeyKind::Root, slot)), None)),
        ("read the inbox", cmd(0x02, 0x00, 0x00, tlv(0x41, &KeyId::INBOX.0.to_be_bytes()), Some(256))),
        (
            "write the circle's public object",
            cmd(0x01, 0x06, 0x00, cat(&[tlv(0x41, &id(KeyKind::Public, slot)), tlv(0x42, &[0, 0]), tlv(0x44, &[0])]), None),
        ),
    ];
    let mut allowed = Vec::new();
    for (what, apdu) in tries {
        let answer = reader.transceive(apdu.encode()).map_err(|e| e.to_string())?;
        let Some((_, sw)) = cahoots_phone::apdu::split(&answer) else {
            return Err(format!("{what}: the card's answer has no status word"));
        };
        if sw == 0x9000 {
            println!("{what}: ALLOWED");
            allowed.push(what);
        } else {
            println!("{what}: refused, {sw:04x}");
        }
    }
    if allowed.is_empty() {
        println!("an unpaired tap is refused all of these");
        Ok(())
    } else {
        Err(format!("an unpaired tap was allowed to {}", allowed.join(", ")))
    }
}

/// The fields of a `screen: ` line.
fn fields(screen: &str) -> Vec<&str> {
    screen.split(" | ").map(str::trim).collect()
}

/// Press through the badge's menu and list to the list entry `entry`, open it, and return
/// once its page is on the screen.
fn goto(port: &str, entry: &str) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    link.keep_logs();
    let deadline = Instant::now() + Duration::from_secs(90);
    let started = Instant::now();
    let mut last = String::new();
    let mut nudged = false;
    let mut opened = false;
    while Instant::now() < deadline {
        link.pump();
        for line in link.take_logs() {
            let Some(screen) = line.strip_prefix("screen: ") else { continue };
            if screen == last {
                continue;
            }
            last = screen.to_string();
            let f = fields(screen);
            let title = f.first().copied().unwrap_or("");
            let footer = f.last().copied().unwrap_or("");
            if footer.contains("discard") || footer.contains("forget") {
                // Never the screens that lose something: step back off them.
                link.send_press(0);
                continue;
            }
            let at_entry = match entry {
                "pair a phone" => title == "pair a phone" && footer.contains("A+B  ready"),
                "phone" => title == "phone" && footer.contains("A+B  unpair"),
                _ => false,
            };
            if at_entry && opened {
                println!("on the page: {screen}");
                return Ok(());
            }
            if opened && title != "circles" {
                return Err(format!("the badge did not open \"{entry}\": {screen}"));
            }
            if title == "self-test" || footer == "A+B  continue" {
                link.send_press(2);
                continue;
            }
            let menu = f
                .get(1)
                .is_some_and(|x| x.split_once('/').is_some_and(|(a, b)| a.parse::<u8>().is_ok() && b.parse::<u8>().is_ok()));
            let code = if title == "circles" || title == "chip needs a restart" {
                match f.iter().find_map(|x| x.strip_prefix("> ")) {
                    Some(e) if e == entry => {
                        opened = true;
                        2
                    }
                    _ => 1,
                }
            } else if menu {
                if f.get(2) == Some(&"circles") { 2 } else { 1 }
            } else if footer.contains("A+B") {
                // Some other page with a question or a way back: Left is back or no.
                0
            } else {
                // A note: any press goes back to the list.
                1
            };
            link.send_press(code);
        }
        if !nudged && last.is_empty() && started.elapsed() > Duration::from_millis(1500) {
            // A badge that was sitting on a screen logs nothing until it changes.
            nudged = true;
            link.send_press(1);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("did not reach \"{entry}\" in 90 seconds"))
}

/// What `press` waits for. See [`press`].
struct Presses<'a> {
    after: Option<&'a str>,
    until: Option<&'a str>,
    expect: Option<&'a str>,
    then: Option<&'a str>,
}

/// Wait for `line` to arrive on `link`, `secs` at most, and return it.
fn wait_line(link: &mut SerialLink, text: &str, secs: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        link.pump();
        if let Some(l) = link.take_logs().into_iter().find(|l| l.contains(text)) {
            return Ok(l);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("the badge did not say \"{text}\" in {secs} seconds"))
}

/// Wait for a log line holding `after` (60 seconds) if given, send one press, then wait for a
/// line holding `until` (20 seconds) and print it, or watch for three seconds. With `expect`,
/// the `until` line must hold it too, or the press is answered with Left (no) and this fails.
/// With `then`, once the screen the press led to is drawn, the same press goes again, for
/// the next screen, and this waits for a line holding `then`: one process, so no line is
/// missed between the two presses.
fn press(port: &str, code: u8, p: Presses<'_>) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    link.keep_logs();
    if let Some(a) = p.after {
        wait_line(&mut link, a, 60)?;
        // The badge throws away a gesture seen while its prompt is drawn.
        std::thread::sleep(Duration::from_millis(600));
    }
    link.send_press(code);
    let Some(u) = p.until else {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            link.pump();
            link.take_logs();
            std::thread::sleep(Duration::from_millis(5));
        }
        return Ok(());
    };
    let line = wait_line(&mut link, u, 20)?;
    println!("{line}");
    if let Some(e) = p.expect {
        if !line.contains(e) {
            wait_line(&mut link, "drawn", 10)?;
            std::thread::sleep(Duration::from_millis(600));
            link.send_press(0);
            return Err(format!("the badge said \"{line}\", not \"{e}\". Answered no"));
        }
    }
    if let Some(t) = p.then {
        wait_line(&mut link, "drawn", 10)?;
        std::thread::sleep(Duration::from_millis(600));
        link.send_press(code);
        println!("{}", wait_line(&mut link, t, 20)?);
    }
    Ok(())
}

/// Forget the circle named `name` through the badge's own pages: open it from the list, turn
/// to its forget page, and answer the question with Both. Returns once the badge says it is
/// forgotten.
fn forget(port: &str, name: &str) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    link.keep_logs();
    let deadline = Instant::now() + Duration::from_secs(120);
    let started = Instant::now();
    let mut last = String::new();
    let mut nudged = false;
    let done = format!("forgotten: {name} ");
    while Instant::now() < deadline {
        link.pump();
        for line in link.take_logs() {
            if line.starts_with(&done) {
                println!("{line}");
                return Ok(());
            }
            let Some(screen) = line.strip_prefix("screen: ") else { continue };
            if screen == last {
                continue;
            }
            last = screen.to_string();
            let f = fields(screen);
            let title = f.first().copied().unwrap_or("");
            let footer = f.last().copied().unwrap_or("");
            let menu = f
                .get(1)
                .is_some_and(|x| x.split_once('/').is_some_and(|(a, b)| a.parse::<u8>().is_ok() && b.parse::<u8>().is_ok()));
            let ours = title.strip_prefix(name).is_some_and(|rest| rest.starts_with("  "));
            let code = if title == "self-test" || footer == "A+B  continue" {
                2
            } else if title == "circles" || title == "chip needs a restart" {
                if f.iter().find_map(|x| x.strip_prefix("> ")) == Some(name) { 2 } else { 1 }
            } else if menu && ours {
                if f.get(2) == Some(&"forget this circle") { 2 } else { 1 }
            } else if menu {
                if f.get(2) == Some(&"circles") { 2 } else { 1 }
            } else if title == "forget this circle?" {
                if f.get(1) == Some(&name) { 2 } else { 0 }
            } else if footer.contains("A+B") {
                0
            } else {
                1
            };
            link.send_press(code);
        }
        if !nudged && last.is_empty() && started.elapsed() > Duration::from_millis(1500) {
            nudged = true;
            link.send_press(1);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("did not forget \"{name}\" in 120 seconds"))
}

