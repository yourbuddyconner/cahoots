//! Channels on the phone against a real station (`objects`, `channels`, `gate`) in process,
//! reached over HTTP: an owner starts a channel from a circle, members join by link, ask to
//! post, are let in, post, and are removed.

use cahoots_phone::signer::SoftSigner;
use cahoots_phone::{
    CeremonyPhase, ChannelMessage, ChannelRole, Http, HttpHeader, HttpResponse, Phone, PhoneError,
};
use std::io::Read;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const PAD: u32 = 1024;

/// HTTP over the network, as Swift's `UrlSessionHttp` does it. It counts the requests made.
#[derive(Clone, Default)]
struct Net {
    requests: Arc<Mutex<Vec<String>>>,
}

impl Http for Net {
    fn get(&self, url: String) -> Result<Vec<u8>, PhoneError> {
        ok_body(self.request("GET".into(), url, Vec::new(), None)?)
    }

    fn post(&self, url: String, body: Vec<u8>) -> Result<Vec<u8>, PhoneError> {
        ok_body(self.request("POST".into(), url, Vec::new(), Some(body))?)
    }

    fn request(
        &self,
        method: String,
        url: String,
        headers: Vec<HttpHeader>,
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, PhoneError> {
        self.requests
            .lock()
            .unwrap()
            .push(format!("{method} {url}"));
        let mut req = ureq::request(&method, &url);
        for h in &headers {
            req = req.set(&h.name, &h.value);
        }
        let res = match body {
            Some(b) => req.send_bytes(&b),
            None => req.call(),
        };
        let resp = match res {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => {
                return Err(PhoneError::Network {
                    message: e.to_string(),
                })
            }
        };
        let status = resp.status();
        let mut body = Vec::new();
        resp.into_reader().read_to_end(&mut body).unwrap();
        Ok(HttpResponse { status, body })
    }
}

fn ok_body(r: HttpResponse) -> Result<Vec<u8>, PhoneError> {
    if (200..300).contains(&r.status) {
        Ok(r.body)
    } else {
        Err(PhoneError::Network {
            message: format!("status {}", r.status),
        })
    }
}

struct Station {
    _rt: tokio::runtime::Runtime,
    base: String,
    _dir: TempDir,
}

fn station() -> Station {
    let dir = TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let limits = cahoots_station::limits::Limits {
        posts_per_minute: None,
        reads_per_minute: None,
        trust_forwarded: false,
        channel_posts_per_minute: None,
    };
    let state =
        cahoots_station::server::State::open_with(&dir.path().join("s.db"), limits, None).unwrap();
    let app = cahoots_station::server::app(state)
        .into_make_service_with_connect_info::<std::net::SocketAddr>();
    rt.spawn(async move { axum::serve(listener, app).await.unwrap() });
    Station {
        _rt: rt,
        base: format!("http://{addr}"),
        _dir: dir,
    }
}

struct Device {
    _dir: TempDir,
    path: String,
    signer: SoftSigner,
    http: Net,
    phone: Arc<Phone>,
}

fn device() -> Device {
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    let signer = SoftSigner::new();
    let http = Net::default();
    let phone = Phone::open(
        path.clone(),
        Box::new(signer.clone()),
        Box::new(http.clone()),
    )
    .unwrap();
    Device {
        _dir: dir,
        path,
        signer,
        http,
        phone,
    }
}

impl Device {
    /// The same phone opened again from its directory, as after the app was closed.
    fn reopen(&self) -> Arc<Phone> {
        Phone::open(
            self.path.clone(),
            Box::new(self.signer.clone()),
            Box::new(self.http.clone()),
        )
        .unwrap()
    }
}

fn show(from: &Phone, from_id: &str, to: &Phone, to_id: &str) {
    if from.ceremony_state(from_id.into()).unwrap().needs_signature {
        from.ceremony_sign(from_id.into()).unwrap();
    }
    for f in from.ceremony_frames(from_id.into()).unwrap() {
        to.ceremony_scan(to_id.into(), f).unwrap();
    }
}

/// A two-seat circle made by codes between `a` and `b`, on `relay`. Returns its id.
fn circle(a: &Device, b: &Device, relay: &str) -> String {
    let ida = a
        .phone
        .start_ceremony("park".into(), 2, PAD, false)
        .unwrap();
    let code = a.phone.ceremony_frames(ida.clone()).unwrap()[0].clone();
    let idb = b.phone.join_ceremony(code, 1).unwrap();
    for _ in 0..6 {
        show(&a.phone, &ida, &b.phone, &idb);
        show(&b.phone, &idb, &a.phone, &ida);
        let done = |p: &Phone, id: &str| {
            p.ceremony_state(id.into()).unwrap().phase == CeremonyPhase::Complete
        };
        if done(&a.phone, &ida) && done(&b.phone, &idb) {
            break;
        }
    }
    let id = a.phone.ceremony_keep(ida).unwrap();
    assert_eq!(b.phone.ceremony_keep(idb).unwrap(), id);
    a.phone.set_relay(id.clone(), relay.into()).unwrap();
    id
}

/// Change a channel's stored record on `d`, as an older version of the app might have left it.
fn edit_meta(d: &Device, id: &str, f: impl FnOnce(&mut cahoots_phone::store::ChannelMeta)) {
    let dir = std::path::Path::new(&d.path).join("channels").join(id);
    let mut meta = cahoots_phone::store::ChannelMeta::load(&dir).unwrap();
    f(&mut meta);
    meta.save(&dir).unwrap();
}

fn texts(msgs: &[ChannelMessage]) -> Vec<String> {
    msgs.iter()
        .filter(|m| !m.note && !m.refused)
        .map(|m| m.text.clone())
        .collect()
}

#[test]
fn owner_and_members_through_a_station() {
    let st = station();
    let (a, b) = (device(), device());
    let circle_id = circle(&a, &b, &st.base);

    // The owner starts a group chat. Every seat signature goes through the seat key's label.
    let before = a.signer.sign_calls().len();
    let s = a
        .phone
        .channel_start(circle_id.clone(), "Garden".into(), 2)
        .unwrap();
    assert_eq!(s.role, ChannelRole::Owner);
    assert_eq!(s.name, "Garden");
    assert_eq!(s.policy_line, "anyone with the link can read and post");
    assert!(s.can_post && !s.dead && !s.waiting);
    assert_eq!(s.circle.as_deref(), Some(circle_id.as_str()));
    let label = a.phone.circle_seat_label(circle_id.clone()).unwrap();
    let signs = a.signer.sign_calls();
    assert!(signs.len() >= before + 4, "{signs:?}");
    assert!(signs[before..].iter().all(|l| *l == label));
    let id = s.id.clone();
    assert_eq!(a.phone.channels(), vec![s.clone()]);

    let link = a.phone.channel_link(id.clone()).unwrap();
    let host = st.base.trim_start_matches("http://");
    assert!(link.contains(&format!("#/ch/{host}/{id}/")), "{link}");
    assert_eq!(a.phone.channel_link(id.clone()).unwrap(), link);
    // The owner opening its own link gets its own channel, whatever the stored relay's form.
    edit_meta(&a, &id, |m| m.relay = format!("{}/", st.base));
    let own = a.phone.channel_join(link.clone()).unwrap();
    assert_eq!(own.role, ChannelRole::Owner);
    edit_meta(&a, &id, |m| m.relay = st.base.clone());
    a.phone
        .channel_send(id.clone(), "hello all".into())
        .unwrap();
    let hist = a.phone.channel_history(id.clone()).unwrap();
    assert_eq!(texts(&hist), ["hello all"]);
    assert!(hist[0].mine);

    // Before joining, the link's preview reads the channel and keeps nothing.
    let c = device();
    let p = c.phone.channel_preview(link.clone()).unwrap();
    assert_eq!(p.name, "Garden");
    assert_eq!(p.policy_line, "anyone with the link can read and post");
    assert_eq!(p.relay, host);
    assert_eq!(p.channel_id, id);
    assert!(p.has_secret);
    let bare = link.rsplit_once('/').unwrap().0.to_string();
    assert!(!c.phone.channel_preview(bare).unwrap().has_secret);
    assert!(c.phone.channels().is_empty());

    // A member joins by the link; the join reads what is there.
    let cs = c.phone.channel_join(link.clone()).unwrap();
    assert_eq!(cs.id, id);
    assert_eq!(cs.role, ChannelRole::Member);
    assert_eq!(cs.relay, host);
    assert!(!cs.can_post && !cs.waiting && !cs.dead);
    assert!(cs.may_ask_to_post && !cs.closed);
    // The same channel's link naming another relay is refused, and changes nothing.
    let elsewhere = link.replace(&format!("#/ch/{host}/"), "#/ch/127.0.0.1:1/");
    match c.phone.channel_join(elsewhere) {
        Err(PhoneError::Channel { message }) => assert_eq!(
            message,
            "This link names a different relay than the one this channel uses."
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(c.phone.channel(id.clone()).unwrap(), cs);
    // The same relay written another way is the same relay: a cased scheme and host, a
    // trailing slash. (Host case on its own is in the crate's unit test; this station's host
    // is an address.)
    let cased = format!("HTTP://{}/", host.to_uppercase());
    edit_meta(&c, &id, |m| m.relay = cased.clone());
    let elsewhere_cased = link.replace(
        &format!("#/ch/{host}/"),
        &format!("#/ch/{}/", host.to_uppercase()),
    );
    assert_eq!(c.phone.channel_join(elsewhere_cased).unwrap().id, id);
    edit_meta(&c, &id, |m| m.relay = st.base.clone());
    assert_eq!(c.phone.channel(id.clone()).unwrap(), cs);
    assert_eq!(cs.unread, 1);
    let hist = c.phone.channel_history(id.clone()).unwrap();
    assert_eq!(texts(&hist), ["hello all"]);
    assert_eq!(hist[0].sender, "owner");
    assert!(!hist[0].mine);
    assert_eq!(c.phone.channel(id.clone()).unwrap().unread, 0);
    // Joining again with the same link changes nothing.
    assert_eq!(c.phone.channel_join(link.clone()).unwrap().id, id);
    assert_eq!(c.phone.channels().len(), 1);
    assert!(c.phone.channel_send(id.clone(), "too soon".into()).is_err());

    // It asks to post and waits; the owner lets it in.
    c.phone
        .channel_ask_to_post(id.clone(), "Cee".into())
        .unwrap();
    assert!(c.phone.channel(id.clone()).unwrap().waiting);
    let reqs = a.phone.channel_requests(id.clone()).unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].name, "Cee");
    assert_eq!(reqs[0].actions, "read,post");

    // A refused Face ID changes nothing: the request is still there.
    a.signer.set_locked(true);
    assert!(matches!(
        a.phone
            .channel_allow(id.clone(), reqs[0].id.clone(), reqs[0].actions.clone()),
        Err(PhoneError::Locked)
    ));
    a.signer.set_locked(false);
    assert_eq!(a.phone.channel_requests(id.clone()).unwrap(), reqs);
    a.phone
        .channel_allow(id.clone(), reqs[0].id.clone(), reqs[0].actions.clone())
        .unwrap();
    assert!(a.phone.channel_requests(id.clone()).unwrap().is_empty());
    let members = a.phone.channel_members(id.clone()).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, "Cee");
    assert_eq!(members[0].actions, "read,post");

    c.phone.channel_poll(id.clone()).unwrap();
    let cs = c.phone.channel(id.clone()).unwrap();
    assert!(cs.can_post && !cs.waiting);
    c.phone
        .channel_send(id.clone(), "hi from Cee".into())
        .unwrap();
    let got = a.phone.channel_poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["hi from Cee"]);
    assert_eq!(got[0].sender, "Cee");
    assert_eq!(a.phone.channel(id.clone()).unwrap().unread, 1);

    // A reader with the first link, who never asks to post.
    let d = device();
    d.phone.channel_join(link.clone()).unwrap();
    assert_eq!(
        texts(&d.phone.channel_history(id.clone()).unwrap()),
        ["hello all", "hi from Cee"]
    );

    // The owner removes Cee: a new link, new keys. Both old readers go dead.
    let new_link = a
        .phone
        .channel_revoke(id.clone(), members[0].grant_id.clone())
        .unwrap();
    assert!(!new_link.is_empty() && new_link != link);
    assert_eq!(a.phone.channel_link(id.clone()).unwrap(), new_link);
    assert!(a.phone.channel_members(id.clone()).unwrap().is_empty());
    a.phone.channel_send(id.clone(), "after".into()).unwrap();
    c.phone.channel_poll(id.clone()).unwrap();
    let cs = c.phone.channel(id.clone()).unwrap();
    assert!(cs.dead && !cs.can_post);
    assert!(c.phone.channel_send(id.clone(), "still me".into()).is_err());
    d.phone.channel_poll(id.clone()).unwrap();
    assert!(d.phone.channel(id.clone()).unwrap().dead);
    assert!(!texts(&d.phone.channel_history(id.clone()).unwrap()).contains(&"after".into()));

    // The reader takes the new link and reads on, keeping its history.
    let ds = d.phone.channel_join(new_link.clone()).unwrap();
    assert!(!ds.dead);
    assert_eq!(
        texts(&d.phone.channel_history(id.clone()).unwrap()),
        ["hello all", "hi from Cee", "after"]
    );

    // The owner's phone, opened again from its records, carries on.
    let a2 = a.reopen();
    a2.channel_send(id.clone(), "still here".into()).unwrap();
    let got = d.phone.channel_poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["still here"]);
    assert_eq!(a2.channels().len(), 1);
}

#[test]
fn announcements_retire_and_new_keys() {
    let st = station();
    let (a, b) = (device(), device());
    let circle_id = circle(&a, &b, &st.base);
    let id = a
        .phone
        .channel_start(circle_id, "News".into(), 0)
        .unwrap()
        .id;
    let link = a.phone.channel_link(id.clone()).unwrap();
    a.phone.channel_send(id.clone(), "first".into()).unwrap();

    let c = device();
    let cs = c.phone.channel_join(link.clone()).unwrap();
    assert_eq!(
        cs.policy_line,
        "anyone with the link can read; only the owner can post"
    );
    assert!(!cs.may_ask_to_post && !cs.closed);
    // Only the owner posts here: asking is refused.
    assert!(matches!(
        c.phone.channel_ask_to_post(id.clone(), "Cee".into()),
        Err(PhoneError::Channel { .. })
    ));

    // New keys: the link's reader keeps reading.
    a.phone.channel_new_keys(id.clone()).unwrap();
    a.phone.channel_send(id.clone(), "second".into()).unwrap();
    let got = c.phone.channel_poll(id.clone()).unwrap();
    assert_eq!(texts(&got), ["second"]);

    // Retiring the link: a new one, and the old one's reader goes dead.
    let new_link = a.phone.channel_retire_link(id.clone()).unwrap();
    assert_ne!(new_link, link);
    a.phone.channel_send(id.clone(), "third".into()).unwrap();
    c.phone.channel_poll(id.clone()).unwrap();
    assert!(c.phone.channel(id.clone()).unwrap().dead);
    // Polling a dead channel reads nothing.
    assert!(c.phone.channel_poll(id.clone()).unwrap().is_empty());

    let e = device();
    e.phone.channel_join(new_link).unwrap();
    assert_eq!(
        texts(&e.phone.channel_history(id.clone()).unwrap()),
        ["third"]
    );

    // Deleting a channel takes its records away.
    c.phone.channel_delete(id.clone()).unwrap();
    assert!(c.phone.channels().is_empty());
    assert!(matches!(
        c.phone.channel(id.clone()),
        Err(PhoneError::NotFound { .. })
    ));
}

#[test]
fn a_closed_channel_has_no_link_and_a_bad_link_does_not_read() {
    let st = station();
    let (a, b) = (device(), device());
    let circle_id = circle(&a, &b, &st.base);
    let s = a.phone.channel_start(circle_id, "Inner".into(), 4).unwrap();
    assert_eq!(
        s.policy_line,
        "members the owner lets in can read; only the owner can post"
    );
    assert!(s.closed && !s.may_ask_to_post);
    assert!(matches!(
        a.phone.channel_link(s.id.clone()),
        Err(PhoneError::Channel { .. })
    ));
    a.phone.channel_send(s.id.clone(), "inside".into()).unwrap();
    assert!(a.phone.channel_poll(s.id.clone()).unwrap().is_empty());

    let c = device();
    assert!(matches!(
        c.phone
            .channel_join("https://cahoots.network/app/#/ch/nowhere".into()),
        Err(PhoneError::BadCode { .. })
    ));
    assert!(c.phone.channels().is_empty());
    assert!(matches!(
        c.phone
            .channel_preview("https://cahoots.network/app/#/ch/nowhere".into()),
        Err(PhoneError::BadCode { .. })
    ));
}

#[test]
fn announcements_with_approvals_are_refused_before_anything_is_uploaded() {
    let st = station();
    let (a, b) = (device(), device());
    let circle_id = circle(&a, &b, &st.base);
    let before = a.http.requests.lock().unwrap().len();
    match a.phone.channel_start(circle_id, "News".into(), 1) {
        Err(PhoneError::Channel { message }) => assert_eq!(
            message,
            "Announcements with approvals need more than one owner device to approve, which this version cannot collect yet."
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(a.http.requests.lock().unwrap().len(), before);
    assert!(a.phone.channels().is_empty());
}
