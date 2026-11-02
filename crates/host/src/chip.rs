//! `cahoots-host chip`: the phone's own chip client (`cahoots_phone::nfc`) run from the laptop
//! against a real badge's chip, through a bench firmware's APDU pass-through (frame kind 32).
//! The badge raises the chip's pin for the exchange and forwards each APDU over I2C in no
//! session, so the laptop meets the applet as a phone's tap does.
//!
//! ```text
//! cahoots-host chip pairing --serial <port>
//! cahoots-host chip inbox --serial <port> --name <text> --key <hex> [--state <dir>]
//! cahoots-host chip open --serial <port> --key <hex>
//! cahoots-host chip hmac --serial <port> --key <hex> --circle <slot> --label <text> [--hello <seat>]
//! cahoots-host chip public --serial <port> [--key <hex>]
//! cahoots-host chip refusals --serial <port> --circle <slot>
//! cahoots-host chip raw --serial <port> --apdu <hex> [--apdu <hex>]...
//! cahoots-host chip goto --serial <port> --entry <name>
//! cahoots-host chip press --serial <port> (left|right|both) [--after <text>] [--until <text> [--expect <text>] [--then <text>]]
//! cahoots-host chip forget --serial <port> --circle <name>
//! cahoots-host chip pair --serial <port> --name <text> --state <dir>
//! ```
//!
//! `--key` is the phone slot's 16-byte AES key, in hex. `inbox` signs with a software P-256 key
//! kept in `--state` (`phone.key`, made on first use), the laptop's stand-in for the phone's
//! enclave key. `refusals` tries, in no session, what an unpaired tap must not do, and says
//! what the applet answered to each. `goto` presses from the badge's home through its list to
//! an entry ("pair a phone", "phone") and opens it. `press` waits for a log line holding
//! `--after` if given, sends one press, and waits for a line holding `--until` (see [`press`]
//! for `--expect` and `--then`). `forget` forgets a circle through the badge's pages. `pair`
//! runs the app's own pairing, the phone crate's `Phone` with no network, from the badge's
//! pairing page: both taps, the presses between them, and the second tap timed (see [`pair`]).
//!
//! `public` reads the public objects of the circle slots the pairing object names: with
//! `--key`, in the slot's session, as a paired phone reads them. Without it they are tried
//! in no session, where the chip refuses each, and the command says so.
//!
//! What the bench showed an unpaired tap can do: read the pairing object, write the inbox,
//! and make an object at any id that holds nothing, under the applet's default policy: a
//! binary object, an HMAC key, an AES key (as an authentication object too), an EC key pair
//! and a counter (SE051 applet 7.2.0, 2026-10-31). The badge never adopts such an object: it
//! writes a circle's public object at the keep after deleting whatever stands at its id,
//! reads back only public objects it wrote, and a phone reads only the circle slots the
//! pairing object names. But a tap can fill the ids of circle slots not yet used with
//! objects nobody may delete, and each such slot is then burnt: the badge passes it over when
//! it picks a slot for a new circle (see `cahoots_badge::chip`). That is a denial of service
//! on the badge's sixteen circle slots, not a way to any key.
//!
//! Every APDU and its answer are printed to stderr, as `-> ` and `<- ` hex lines.

use crate::link::SerialLink;
use cahoots_badge::public::PublicObject;
use cahoots_chip::{KeyId, KeyKind};
use cahoots_core::seat::SeatKey;
use cahoots_phone::nfc::{self, Nfc, Tap, TapError};
use cahoots_phone::signer::SoftSigner;
use cahoots_phone::{Http, HttpHeader, HttpResponse, Phone, PhoneError, Signer};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
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
        "window" => return window(port, get("--seconds").and_then(|s| s.parse().ok()).unwrap_or(90)),
        "pintest" => return pintest(port, 0x02),
        "starttime" => return pintest(port, 0x03),
        "forget" => return forget(port, req("--circle")?),
        "pair" => return pair(port, req("--name")?, Path::new(req("--state")?)),
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
            let slots: Vec<String> = p.circle_slots().iter().map(u8::to_string).collect();
            println!("circles: {}", if slots.is_empty() { "none".into() } else { slots.join(" ") });
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
            let hello: Option<u8> = match get("--hello") {
                Some(h) => Some(h.parse().map_err(|_| "--hello: a seat, 0 to 7")?),
                None => None,
            };
            let mut s = tap.open_slot(&key).map_err(tap_error)?;
            // A command with no data first: the MAC after it shows the applet counts it in
            // the encryption counter.
            s.version().map_err(tap_error)?;
            let mac = match hello {
                Some(other) => s.hmac_hello(slot, other, label.as_bytes()),
                None => s.hmac_root(slot, label.as_bytes()),
            }
            .map_err(tap_error)?;
            s.close().map_err(tap_error)?;
            println!("hmac: {}", hex::encode(mac));
            Ok(())
        }
        "public" => match get("--key") {
            Some(k) => {
                let key = key16(k)?;
                let mut s = tap.open_slot(&key).map_err(tap_error)?;
                let all = s.list_public().map_err(tap_error)?;
                s.close().map_err(tap_error)?;
                if all.is_empty() {
                    println!("no public objects");
                }
                for (slot, p) in &all {
                    println!("{}", describe(*slot, p));
                }
                Ok(())
            }
            None => {
                let slots = tap.read_pairing().map_err(tap_error)?.circle_slots();
                let mut read = Vec::new();
                for slot in slots {
                    match tap.read_public(slot) {
                        Ok(_) => read.push(slot),
                        Err(TapError::NotAllowed) => println!("slot {slot}: refused in no session"),
                        Err(e) => return Err(format!("slot {slot}: {}", tap_error(e))),
                    }
                }
                if read.is_empty() {
                    Ok(())
                } else {
                    Err(format!("a tap in no session read the public objects of slots {read:?}"))
                }
            }
        },
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
            "read the circle's public object",
            cmd(0x02, 0x00, 0x00, tlv(0x41, &id(KeyKind::Public, slot)), Some(256)),
        ),
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

/// The footer of a list: the circles, or one circle's actions under its name.
const LIST_FOOTER: &str = "prev  A+B  open  next";

/// The title of home, the page the badge rests on.
const HOME: &str = "badge";

/// How long a walker waits on a screen it does not press on, or before it has seen any, until
/// it presses Right to see where the badge is. Longer than a note stays: a note goes home by
/// itself first.
const NUDGE_AFTER: Duration = Duration::from_secs(6);

/// What a walker does about one screen of the badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// Press `0` Left, `1` Right or `2` Both.
    Press(u8),
    /// Press Both on the list entry the walker is going to.
    Open,
    /// Press nothing and wait for the next screen.
    Wait,
    /// The walker is where it was going.
    There,
    /// The entry was opened and the badge showed something else.
    Refused,
}

/// Home: the badge's circles by name over the footer "tap  A+B  menu  menu", which starts
/// with "close" while the window for a tap is open.
fn is_home(f: &[&str]) -> bool {
    f.first() == Some(&HOME) && f.last().is_some_and(|x| x.contains("A+B  menu"))
}

/// The list of circles, under its usual title or the one it has while the chip is held.
fn is_list(title: &str) -> bool {
    title == "circles" || title == "chip needs a restart"
}

/// The list entry marked with `>`.
fn chosen<'a>(f: &[&'a str]) -> Option<&'a str> {
    f.iter().find_map(|x| x.strip_prefix("> "))
}

/// The press every walker makes on a screen that is not on its own way: Both past the
/// self-test and from home to the list (never Left there, which opens or closes the window
/// for a tap), Right along the list, "back" out of a circle's actions, and Left off any other
/// page with a way back or a question. A note gets no press: it goes home by itself within
/// five seconds. Neither does a page in the middle of something.
fn walk_on(f: &[&str]) -> Walk {
    let title = f.first().copied().unwrap_or("");
    let footer = f.last().copied().unwrap_or("");
    if title == "self-test" || footer == "A+B  continue" || is_home(f) {
        Walk::Press(2)
    } else if is_list(title) {
        Walk::Press(1)
    } else if footer == LIST_FOOTER {
        Walk::Press(if f.contains(&"> back") { 2 } else { 1 })
    } else if footer.contains("A+B") || matches!(footer.split("  ").next(), Some("back" | "no" | "refuse")) {
        Walk::Press(0)
    } else {
        Walk::Wait
    }
}

/// Press Right once a walker has waited [`NUDGE_AFTER`] on a screen it does not press on, or
/// with no screen at all: a badge sitting on a page logs nothing until it changes. Right
/// opens the list from home, moves along a list, and goes back from any other page.
fn nudge(link: &mut SerialLink, waiting: &mut Option<Instant>) {
    if waiting.is_some_and(|t| t.elapsed() > NUDGE_AFTER) {
        *waiting = None;
        link.send_press(1);
    }
}

/// Ask the badge to raise the chip's pin and say nothing on the wire for `seconds`, so a
/// phone's tap meets the chip powered. The badge's log prints meanwhile.
fn window(port: &str, seconds: u64) -> Result<(), String> {
    let reader = SerialNfc::open(port)?;
    reader.begin(String::new()).map_err(|e| e.to_string())?;
    let answer = reader.transceive(vec![0x01]).map_err(|e| e.to_string())?;
    if answer != [0x90, 0x00] {
        return Err("the badge did not open a window (not a bench build?)".into());
    }
    println!("window open for {seconds} s: tap the phone now");
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if let Ok(mut link) = reader.link.lock() {
            link.pump();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    reader.end(String::new(), false).map_err(|e| e.to_string())?;
    println!("window closed");
    Ok(())
}

/// Ask the badge to speak to the chip with the pin low, and print what it logs for 15 s.
fn pintest(port: &str, request: u8) -> Result<(), String> {
    let reader = SerialNfc::open(port)?;
    reader.begin(String::new()).map_err(|e| e.to_string())?;
    let answer = reader.transceive(vec![request]).map_err(|e| e.to_string())?;
    if answer != [0x90, 0x00] {
        return Err("the badge did not run the pin test (not a bench build?)".into());
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(mut link) = reader.link.lock() {
            link.pump();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

/// Press through the badge's home and list to the list entry `entry`, open it, and return
/// once its page is on the screen.
fn goto(port: &str, entry: &str) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    link.keep_logs();
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut last = String::new();
    let mut waiting = Some(Instant::now());
    let mut opened = false;
    while Instant::now() < deadline {
        link.pump();
        for line in link.take_logs() {
            let Some(screen) = line.strip_prefix("screen: ") else { continue };
            if screen == last {
                continue;
            }
            last = screen.to_string();
            waiting = None;
            match goto_walk(entry, &fields(screen), opened) {
                Walk::There => {
                    println!("on the page: {screen}");
                    return Ok(());
                }
                Walk::Refused => return Err(format!("the badge did not open \"{entry}\": {screen}")),
                Walk::Open => {
                    opened = true;
                    link.send_press(2);
                }
                Walk::Press(code) => link.send_press(code),
                Walk::Wait => waiting = Some(Instant::now()),
            }
        }
        nudge(&mut link, &mut waiting);
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("did not reach \"{entry}\" in 90 seconds"))
}

/// What `goto` does on the screen `f`, on its way to the list entry `entry`, which it chose
/// already when `opened`.
fn goto_walk(entry: &str, f: &[&str], opened: bool) -> Walk {
    let title = f.first().copied().unwrap_or("");
    let footer = f.last().copied().unwrap_or("");
    let there = match entry {
        "pair a phone" => title == "pair a phone" && footer.contains("A+B  ready"),
        "phone" => title == "phone" && footer.contains("A+B  unpair"),
        _ => false,
    };
    if there && opened {
        return Walk::There;
    }
    if is_list(title) {
        return if chosen(f) == Some(entry) { Walk::Open } else { Walk::Press(1) };
    }
    if opened {
        return Walk::Refused;
    }
    walk_on(f)
}

/// What `press` waits for. See [`press`].
struct Presses<'a> {
    after: Option<&'a str>,
    until: Option<&'a str>,
    expect: Option<&'a str>,
    then: Option<&'a str>,
}

/// Wait for a line holding `text` to arrive on `link`, `secs` at most, and return it. The lines
/// read past it wait in `backlog` for the next call: the badge can send a prompt's line and
/// its "drawn" in one read, since it sends its log only between draws.
fn wait_line(link: &mut SerialLink, backlog: &mut VecDeque<String>, text: &str, secs: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        while let Some(l) = backlog.pop_front() {
            if l.contains(text) {
                return Ok(l);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!("the badge did not say \"{text}\" in {secs} seconds"));
        }
        link.pump();
        backlog.extend(link.take_logs());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Wait for a log line holding `after` (60 seconds) if given, send one press, then wait for a
/// line holding `until` (20 seconds) and print it, or watch for three seconds. With `expect`,
/// the `until` line must hold it too, or the press is answered with Left (no) and this fails.
/// With `then`, once the screen the press led to is drawn, the same press goes again, for
/// the next screen, and this waits for a line holding `then`: one process, so no line is
/// missed between the two presses.
fn press(port: &str, code: u8, p: Presses<'_>) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    press_on(&mut link, code, p)
}

/// [`press`], on a link already open.
fn press_on(link: &mut SerialLink, code: u8, p: Presses<'_>) -> Result<(), String> {
    link.keep_logs();
    let mut backlog = VecDeque::new();
    if let Some(a) = p.after {
        wait_line(link, &mut backlog, a, 60)?;
        // The badge throws away a gesture seen while its prompt is drawn.
        std::thread::sleep(Duration::from_millis(600));
    }
    // What the badge said before the press does not answer it.
    backlog.clear();
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
    let line = wait_line(link, &mut backlog, u, 20)?;
    println!("{line}");
    if let Some(e) = p.expect {
        if !line.contains(e) {
            wait_line(link, &mut backlog, "drawn", 10)?;
            std::thread::sleep(Duration::from_millis(600));
            link.send_press(0);
            return Err(format!("the badge said \"{line}\", not \"{e}\". Answered no"));
        }
    }
    if let Some(t) = p.then {
        wait_line(link, &mut backlog, "drawn", 10)?;
        std::thread::sleep(Duration::from_millis(600));
        backlog.clear();
        link.send_press(code);
        println!("{}", wait_line(link, &mut backlog, t, 20)?);
    }
    Ok(())
}

/// Forget the circle named `name` through the badge's own pages: open it from the list, open
/// "forget this circle" from its actions, and answer the question with Both. Returns once the
/// badge says it is forgotten.
fn forget(port: &str, name: &str) -> Result<(), String> {
    let mut link = SerialLink::open(port, "badge").map_err(|e| format!("{port}: {e}"))?;
    link.keep_logs();
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut last = String::new();
    let mut waiting = Some(Instant::now());
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
            waiting = None;
            match forget_walk(name, &fields(screen)) {
                Walk::Press(code) => link.send_press(code),
                _ => waiting = Some(Instant::now()),
            }
        }
        nudge(&mut link, &mut waiting);
        std::thread::sleep(Duration::from_millis(5));
    }
    Err(format!("did not forget \"{name}\" in 120 seconds"))
}

/// What `forget` does on the screen `f`, on its way to forgetting the circle `name`.
fn forget_walk(name: &str, f: &[&str]) -> Walk {
    let title = f.first().copied().unwrap_or("");
    let footer = f.last().copied().unwrap_or("");
    // A page one of the circle's actions opened has its header, "name  tier n".
    let ours = title.strip_prefix(name).is_some_and(|rest| rest.starts_with("  "));
    if is_list(title) {
        Walk::Press(if chosen(f) == Some(name) { 2 } else { 1 })
    } else if footer == LIST_FOOTER && title == name {
        // Its actions, under its name: open "forget this circle".
        Walk::Press(if f.contains(&"> forget this circle") { 2 } else { 1 })
    } else if ours && f.get(1) == Some(&"forget this circle") {
        Walk::Press(2)
    } else if title == "forget this circle?" {
        Walk::Press(if f.get(1) == Some(&name) { 2 } else { 0 })
    } else {
        walk_on(f)
    }
}

/// The laptop's stand-in for the phone's Secure Enclave and keychain, for [`pair`]: the phone
/// crate's software signer, which also keeps a copy of the last secret put, the phone slot's
/// key, for the bench's checks after the pairing.
#[derive(Clone, Default)]
struct BenchSigner {
    soft: SoftSigner,
    slot_key: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Signer for BenchSigner {
    fn make_key(&self, label: String) -> Result<Vec<u8>, PhoneError> {
        self.soft.make_key(label)
    }
    fn sign(&self, label: String, message: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        self.soft.sign(label, message)
    }
    fn batch_begin(&self) {
        self.soft.batch_begin();
    }
    fn batch_end(&self) {
        self.soft.batch_end();
    }
    fn put_secret(&self, label: String, secret: Vec<u8>) -> Result<(), PhoneError> {
        if let Ok(mut k) = self.slot_key.lock() {
            *k = Some(secret.clone());
        }
        self.soft.put_secret(label, secret)
    }
    fn get_secret(&self, label: String) -> Result<Option<Vec<u8>>, PhoneError> {
        self.soft.get_secret(label)
    }
}

/// No network for the bench's phone: the badge's other circles may be circles a person
/// chats in, and the bench must not post their bumps or the badge's card to their relay.
struct Offline;

fn offline() -> PhoneError {
    PhoneError::Network {
        message: "the bench phone has no network".into(),
    }
}

impl Http for Offline {
    fn get(&self, _url: String) -> Result<Vec<u8>, PhoneError> {
        Err(offline())
    }
    fn post(&self, _url: String, _body: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        Err(offline())
    }
    fn request(
        &self,
        _method: String,
        _url: String,
        _headers: Vec<HttpHeader>,
        _body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, PhoneError> {
        Err(offline())
    }
}

/// The bench pass-through as the phone's reader, shared with the presses between the two
/// taps, counting the APDUs it carries.
struct SharedNfc(Arc<SerialNfc>, Arc<AtomicU32>);

impl Nfc for SharedNfc {
    fn begin(&self, prompt: String) -> Result<(), PhoneError> {
        self.0.begin(prompt)
    }
    fn transceive(&self, apdu: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.0.transceive(apdu)
    }
    fn end(&self, message: String, failed: bool) -> Result<(), PhoneError> {
        self.0.end(message, failed)
    }
}

/// Pair the laptop with the badge as the app pairs a phone, with the phone crate's own
/// `Phone`, made in `dir`, which must be empty or new, and given no network. The badge must be on its pairing
/// page. `pair_card_start` is the first tap: it writes the inbox under `name` and shows the
/// words. Both on the badge shows its words, and only if they are the laptop's, Both again
/// pairs. `pair_card_finish` is the second tap: it keeps every circle of the badge as a card
/// circle, with its chat key and the hello words ahead. It is timed. Prints the words, the
/// phone's key, the second tap's time and the phone slot's key.
fn pair(port: &str, name: &str, dir: &Path) -> Result<(), String> {
    if std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!(
            "{} is not empty. Give an empty or new directory for the laptop's phone.",
            dir.display()
        ));
    }
    let reader = Arc::new(SerialNfc::open(port)?);
    let apdus = Arc::new(AtomicU32::new(0));
    let signer = BenchSigner::default();
    let phone = Phone::open(
        dir.to_string_lossy().into_owned(),
        Box::new(signer.clone()),
        Box::new(Offline),
        Box::new(SharedNfc(reader.clone(), apdus.clone())),
    )
    .map_err(|e| e.to_string())?;
    let preview = phone
        .pair_card_start(name.into())
        .map_err(|e| format!("the first tap: {e}"))?;
    let public = signer
        .soft
        .public(cahoots_phone::PHONE_KEY)
        .ok_or("the phone made no key of its own")?;
    let words = preview.words.join(" ");
    println!("phone key: {}", hex::encode(public));
    println!("words: {words}");
    {
        let mut link = reader.link.lock().map_err(|_| "the reader's link is poisoned")?;
        link.keep_logs();
        link.take_logs();
        let expect = format!("words {words}");
        let p = Presses { after: None, until: Some("pairing: "), expect: Some(&expect), then: Some("paired with") };
        press_on(&mut link, 2, p)?;
    }
    apdus.store(0, Ordering::SeqCst);
    let started = Instant::now();
    let kept = phone
        .pair_card_finish()
        .map_err(|e| format!("the second tap: {e}"))?;
    let took = started.elapsed();
    println!(
        "second tap: {} circles kept, {} APDUs, {:.1} s",
        kept.len(),
        apdus.load(Ordering::SeqCst),
        took.as_secs_f64()
    );
    let key = signer
        .slot_key
        .lock()
        .ok()
        .and_then(|k| k.clone())
        .ok_or("the phone kept no slot key")?;
    println!("slot key: {}", hex::encode(key));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk for a `screen: ` line as the badge logs it.
    fn goto_on(entry: &str, screen: &str, opened: bool) -> Walk {
        goto_walk(entry, &fields(screen), opened)
    }

    fn forget_on(name: &str, screen: &str) -> Walk {
        forget_walk(name, &fields(screen))
    }

    const HOME_SCREEN: &str = "badge | owner-bench3 | pair-1712 | closed to taps | tap  A+B  menu  menu";
    const OPEN_HOME: &str = "badge | owner-bench3 | open for a tap, 2 min | close  A+B  menu  menu";

    #[test]
    fn home_opens_the_list_with_both_never_left() {
        for screen in [HOME_SCREEN, OPEN_HOME] {
            assert_eq!(goto_on("pair a phone", screen, false), Walk::Press(2));
            assert_eq!(forget_on("pair-1712", screen), Walk::Press(2));
        }
    }

    #[test]
    fn goto_moves_along_the_list_and_opens_its_entry() {
        let on_circle = "circles | > owner-bench3 |   pair-1712 |   new circle |   pair a phone |   back | prev  A+B  open  next";
        let on_pair = "circles |   owner-bench3 |   pair-1712 |   new circle | > pair a phone |   back | prev  A+B  open  next";
        assert_eq!(goto_on("pair a phone", on_circle, false), Walk::Press(1));
        assert_eq!(goto_on("pair a phone", on_pair, false), Walk::Open);
        assert_eq!(goto_on("phone", on_pair, false), Walk::Press(1));
    }

    #[test]
    fn goto_stops_on_the_page_it_opened_and_fails_on_anything_else() {
        let pair = "pair a phone | hold the phone | to the chip | then press both | back  A+B  ready  back";
        let phone = "phone | bench laptop | reads and posts in | this badge's circles | unpairing ends its taps | back  A+B  unpair  back";
        assert_eq!(goto_on("pair a phone", pair, true), Walk::There);
        assert_eq!(goto_on("phone", phone, true), Walk::There);
        assert_eq!(goto_on("pair a phone", "unpair the current phone first", true), Walk::Refused);
        // A pairing page the walker did not open is left, which closes the pairing.
        assert_eq!(goto_on("pair a phone", pair, false), Walk::Press(0));
    }

    #[test]
    fn a_note_gets_no_press_since_it_goes_home_by_itself() {
        for note in ["phone unpaired | its taps end now", "saved pair-1712", "not paired", "circle forgotten"] {
            assert_eq!(goto_on("phone", note, false), Walk::Wait, "{note}");
            assert_eq!(forget_on("pair-1712", note), Walk::Wait, "{note}");
        }
    }

    #[test]
    fn a_circles_actions_and_pages_lead_back_to_the_list() {
        let actions = "owner-bench3 | > words |   key as code |   grants |   forget this circle |   back | prev  A+B  open  next";
        let at_back = "owner-bench3 |   words |   key as code |   grants |   forget this circle | > back | prev  A+B  open  next";
        let words = "owner-bench3  tier 2 | words | a | b | c | d | e | f | back  back";
        assert_eq!(goto_on("phone", actions, false), Walk::Press(1));
        assert_eq!(goto_on("phone", at_back, false), Walk::Press(2));
        assert_eq!(goto_on("phone", words, false), Walk::Press(0));
        assert_eq!(forget_on("pair-1712", actions), Walk::Press(1));
        assert_eq!(forget_on("pair-1712", at_back), Walk::Press(2));
        assert_eq!(forget_on("pair-1712", words), Walk::Press(0));
    }

    #[test]
    fn pair_refuses_a_state_directory_that_holds_anything() {
        let dir = std::env::temp_dir().join(format!("cahoots-pair-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), b"not the phone's").unwrap();
        let e = pair("/dev/null", "bench laptop", &dir).err().unwrap();
        assert!(e.contains("is not empty"), "{e}");
        assert!(dir.join("keep.txt").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_opens_its_own_circle_and_answers_only_its_own_question() {
        let list = "circles |   owner-bench3 | > pair-1712 |   new circle |   back | prev  A+B  open  next";
        let actions = "pair-1712 |   words |   key as code |   grants | > forget this circle |   back | prev  A+B  open  next";
        let page = "pair-1712  tier 2 | forget this circle | Both to forget this circle | the others stay | back  A+B  choose  back";
        let ours = "forget this circle? | pair-1712 | its words, pad and keys | are erased for good | other circles stay | back  A+B  forget  back";
        let other = "forget this circle? | owner-bench3 | its words, pad and keys | are erased for good | other circles stay | back  A+B  forget  back";
        assert_eq!(forget_on("pair-1712", list), Walk::Press(2));
        assert_eq!(forget_on("owner-bench4", list), Walk::Press(1));
        assert_eq!(forget_on("pair-1712", actions), Walk::Press(2));
        assert_eq!(forget_on("pair-1712", page), Walk::Press(2));
        assert_eq!(forget_on("pair-1712", ours), Walk::Press(2));
        assert_eq!(forget_on("pair-1712", other), Walk::Press(0));
        // Another circle's forget page is left by Left, back to its actions.
        let theirs = "owner-bench3  tier 2 | forget this circle | Both to forget this circle | the others stay | back  A+B  choose  back";
        assert_eq!(forget_on("pair-1712", theirs), Walk::Press(0));
    }
}
