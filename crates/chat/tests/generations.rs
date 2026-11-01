//! Chat by generation (docs/chat.md section 1): epoch keys come from a generation key, a
//! signed bump moves the circle to a fresh generation, and the last two stay open.

use cahoots_chat::{
    generation_tag, parse, sign_bump, verify_bump, BumpOutcome, ChatError, ChatSession, EpochId,
    MessageType,
};
use cahoots_core::manifest::{Manifest, Seat};
use cahoots_core::params::CircleParams;
use cahoots_core::root::{chat_gen_key, gen0_id};
use cahoots_core::seat::SeatKey;
use cahoots_core::CircleId;

const ROOT: [u8; 32] = [0x42; 32];
const CID: CircleId = [0x0c; 16];
const ID1: [u8; 32] = [0x11; 32];
const ID2: [u8; 32] = [0x22; 32];
const ID3: [u8; 32] = [0x33; 32];

const MOVED: BumpOutcome = BumpOutcome::Moved { dropped_own: false };

fn g0() -> [u8; 32] {
    gen0_id(&CID)
}

fn synced(mut s: ChatSession) -> ChatSession {
    s.mark_synced();
    s
}

fn member(i: u8) -> ChatSession {
    synced(ChatSession::new(&ROOT, CID, 3, i))
}

fn epoch_of(wire: &[u8]) -> EpochId {
    parse(wire).unwrap().0.epoch
}

fn seat_key(i: u8) -> SeatKey {
    SeatKey::from_seed(&[i + 1; 32]).unwrap()
}

/// A three-seat manifest for `CID` whose seat keys come from `seat_key`.
fn manifest() -> Manifest {
    let params = CircleParams::new(3, 512, "ring", false).unwrap();
    Manifest {
        circle_id: CID,
        floor: Manifest::default_floor(&params),
        params,
        seats: (0..3)
            .map(|i| Seat {
                seat: i,
                key: seat_key(i).public(),
                tier: 0,
                evidence: Vec::new(),
            })
            .collect(),
        genesis: 0,
    }
}

#[test]
fn three_sessions_chat_in_generation_zero_and_rotate_for_free() {
    let mut s: Vec<ChatSession> = (0..3).map(member).collect();
    let tag = generation_tag(&gen0_id(&CID));
    for m in &s {
        assert_eq!(m.generation_id(), gen0_id(&CID));
    }
    let w = s[0].send(b"hello").unwrap();
    assert_eq!(epoch_of(&w), EpochId::generation(tag, 1));
    let h = parse(&w).unwrap().0;
    assert_eq!(h.version, 3);
    assert_eq!(w[0], 2, "the relay still sees version 2");
    assert_eq!(s[1].receive(&w).unwrap().1, b"hello");
    assert_eq!(s[2].receive(&w).unwrap().1, b"hello");

    s[1].start_epoch();
    let w2 = s[1].send(b"two").unwrap();
    s[2].start_epoch();
    s[2].start_epoch();
    let w3 = s[2].send(b"three").unwrap();
    assert_eq!(epoch_of(&w3), EpochId::generation(tag, 3));
    for (i, m) in s.iter_mut().enumerate() {
        if i != 1 {
            assert_eq!(m.receive(&w2).unwrap().1, b"two");
        }
        if i != 2 {
            assert_eq!(m.receive(&w3).unwrap().1, b"three");
        }
        assert_eq!(
            m.current_epoch().unwrap().generation_parts(),
            Some((tag, 3))
        );
    }
}

#[test]
fn a_bump_moves_every_root_holder_and_locks_out_a_device_without_the_key() {
    let mut a = member(0);
    let mut b = member(1);
    let mut d = synced(ChatSession::from_generation(
        gen0_id(&CID),
        chat_gen_key(&ROOT, &CID, &gen0_id(&CID)),
        CID,
        3,
        2,
    ));
    let hello = a.send(b"before").unwrap();
    assert_eq!(d.receive(&hello).unwrap().1, b"before");

    let payload = sign_bump(&seat_key(0), 0, &CID, &g0(), &ID1).unwrap();
    let wire = a.send_bump(ID1, &payload).unwrap();
    assert_eq!(a.generation_id(), ID1);

    let (h, got) = b.receive(&wire).unwrap();
    assert_eq!(h.msg_type, MessageType::Bump);
    assert_eq!(
        verify_bump(&got, &CID, &manifest()).unwrap(),
        (g0(), ID1, 0)
    );
    assert_eq!(b.apply_bump(g0(), ID1), Ok(MOVED));
    assert_eq!(b.generation_id(), ID1);

    let (h, got) = d.receive(&wire).unwrap();
    assert_eq!(h.msg_type, MessageType::Bump);
    let (from, to, _) = verify_bump(&got, &CID, &manifest()).unwrap();
    assert_eq!(d.apply_bump(from, to), Err(ChatError::NeedsGeneration(ID1)));
    assert_eq!(d.generation_id(), gen0_id(&CID));

    let w = a.send(b"after").unwrap();
    assert_eq!(epoch_of(&w), EpochId::generation(generation_tag(&ID1), 1));
    assert_eq!(b.receive(&w).unwrap().1, b"after");
    assert_eq!(d.receive(&w), Err(ChatError::NeedsGeneration(ID1)));

    d.install_generation(ID1, chat_gen_key(&ROOT, &CID, &ID1));
    assert_eq!(d.generation_id(), ID1);
    assert_eq!(d.receive(&w).unwrap().1, b"after");
    let back = d.send(b"from d").unwrap();
    assert_eq!(a.receive(&back).unwrap().1, b"from d");
}

#[test]
fn a_device_forwards_a_seat_signed_bump_and_then_needs_the_new_key() {
    // A device holds generation 0's key from a tap. Its badge signed a bump with the seat key
    // at unpair; the device seals it in generation 0, where the others are.
    let key0 = chat_gen_key(&ROOT, &CID, &gen0_id(&CID));
    let mut d = synced(ChatSession::from_generation(gen0_id(&CID), key0, CID, 3, 2));
    let mut a = member(0);
    assert_eq!(d.pending_generation(), None);
    let payload = sign_bump(&seat_key(2), 2, &CID, &g0(), &ID1).unwrap();
    let wire = d.send_bump(ID1, &payload).unwrap();
    assert_eq!(epoch_of(&wire), EpochId::generation(generation_tag(&g0()), 1));

    // A root holder reads it, checks the seat's signature and moves.
    let (h, got) = a.receive(&wire).unwrap();
    assert_eq!(h.msg_type, MessageType::Bump);
    assert_eq!(h.sender, 2);
    assert_eq!(verify_bump(&got, &CID, &manifest()).unwrap(), (g0(), ID1, 2));
    assert_eq!(a.apply_bump(g0(), ID1), Ok(MOVED));
    assert_eq!(a.generation_id(), ID1);

    // The device cannot derive the new key: it waits for it, and says so.
    assert_eq!(d.pending_generation(), Some(ID1));
    assert_eq!(d.generation_id(), g0());
    let w = a.send(b"after the bump").unwrap();
    assert_eq!(d.receive(&w), Err(ChatError::NeedsGeneration(ID1)));
    // A second bump while it waits would move from a generation the others have left.
    let again = sign_bump(&seat_key(2), 2, &CID, &g0(), &ID2).unwrap();
    assert_eq!(d.send_bump(ID2, &again), Err(ChatError::CannotBump));

    // Given the key by a tap, it reads on.
    d.install_generation(ID1, chat_gen_key(&ROOT, &CID, &ID1));
    assert_eq!(d.pending_generation(), None);
    assert_eq!(d.generation_id(), ID1);
    assert_eq!(d.receive(&w).unwrap().1, b"after the bump");
}

#[test]
fn a_bump_from_a_key_that_is_not_a_seat_is_refused() {
    let m = manifest();
    let stranger = SeatKey::from_seed(&[0x99; 32]).unwrap();
    // Claims seat 1 but is signed by a key the manifest does not hold.
    let forged = sign_bump(&stranger, 1, &CID, &g0(), &ID1).unwrap();
    assert!(verify_bump(&forged, &CID, &m).is_err());
    // A seat the manifest does not have.
    let no_seat = sign_bump(&seat_key(0), 7, &CID, &g0(), &ID1).unwrap();
    assert!(verify_bump(&no_seat, &CID, &m).is_err());
    // A real seat's signature with either id swapped.
    let good = sign_bump(&seat_key(1), 1, &CID, &g0(), &ID1).unwrap();
    let mut swapped = good.clone();
    swapped[32..64].copy_from_slice(&ID2);
    assert!(verify_bump(&swapped, &CID, &m).is_err());
    let mut swapped = good.clone();
    swapped[..32].copy_from_slice(&ID2);
    assert!(verify_bump(&swapped, &CID, &m).is_err());
    // A real seat's signature for another circle.
    let other = sign_bump(&seat_key(1), 1, &[0x0d; 16], &g0(), &ID1).unwrap();
    assert!(verify_bump(&other, &CID, &m).is_err());
    assert!(verify_bump(&forged[..20], &CID, &m).is_err());
    assert!(verify_bump(&forged[..60], &CID, &m).is_err());
    // The refused bump moves nobody: the caller does not apply it, and chat goes on.
    let mut a = member(0);
    let mut b = member(1);
    let w = a.send_typed(MessageType::Bump, &forged).unwrap();
    let (h, got) = b.receive(&w).unwrap();
    assert_eq!(h.msg_type, MessageType::Bump);
    assert!(verify_bump(&got, &CID, &m).is_err());
    let w = a.send(b"same generation").unwrap();
    assert_eq!(b.receive(&w).unwrap().1, b"same generation");
    assert_eq!(b.generation_id(), gen0_id(&CID));
    // A good one verifies and names its seat.
    let good = sign_bump(&seat_key(2), 2, &CID, &g0(), &ID1).unwrap();
    assert_eq!(verify_bump(&good, &CID, &m).unwrap(), (g0(), ID1, 2));
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// The two root-epoch messages seat 0 sent before generations existed. Both fixture files were
/// written at commit 2496588, before generations, by `tests/fixtures/make-chat-v5.rs`. Nothing
/// in that code is random, so running it again at that commit gives the same bytes.
fn v5_wires() -> Vec<Vec<u8>> {
    let all = fixture("chat-v5-wires.bin");
    let mut out = Vec::new();
    let mut rest = &all[..];
    while !rest.is_empty() {
        let len = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
        out.push(rest[4..4 + len].to_vec());
        rest = &rest[4 + len..];
    }
    out
}

#[test]
fn a_v5_state_with_root_epoch_history_imports_as_generation_zero() {
    // Seat 1 of 3 read the first root-epoch message, then exported its state.
    let state = fixture("chat-v5.bin");
    assert_eq!(&state[..4], b"CHS2");
    let wires = v5_wires();
    assert_eq!(epoch_of(&wires[0]), EpochId::root(1));
    assert_eq!(parse(&wires[0]).unwrap().0.version, 2);

    let mut b = ChatSession::import_state(&ROOT, CID, 3, 1, &state).unwrap();
    assert_eq!(b.generation_id(), gen0_id(&CID));
    // The message it already read is a replay, the stored one it had not read opens.
    assert_eq!(b.receive(&wires[0]), Err(ChatError::Replay));
    assert_eq!(b.receive(&wires[1]).unwrap().1, b"old history two");
    b.mark_synced();

    // A member on the new build with an empty state reads the old history too.
    let mut c = member(2);
    assert_eq!(c.receive(&wires[0]).unwrap().1, b"old history one");
    assert_eq!(c.receive(&wires[1]).unwrap().1, b"old history two");

    // The imported member now sends in generation 0, and the others read it.
    let w = b.send(b"new").unwrap();
    assert_eq!(
        epoch_of(&w),
        EpochId::generation(generation_tag(&gen0_id(&CID)), 1)
    );
    assert_eq!(c.receive(&w).unwrap().1, b"new");

    // Exported again, the state is the new form and keeps everything.
    let again = b.export_state();
    assert_eq!(&again[..4], b"CHS4");
    let mut b2 = ChatSession::import_state(&ROOT, CID, 3, 1, &again).unwrap();
    assert_eq!(b2.receive(&wires[1]), Err(ChatError::Replay));
    b2.mark_synced();
    let w2 = b2.send(b"newer").unwrap();
    assert_eq!(parse(&w2).unwrap().0.counter, 1);
    assert_eq!(c.receive(&w2).unwrap().1, b"newer");
}

#[test]
fn the_last_two_generations_stay_open() {
    let mut a = member(0);
    let mut b = member(1);
    let mut c = member(2);
    // c sends in generation 0 before it reads the bump.
    let straggler0 = c.send(b"late in gen 0").unwrap();
    let wire = a
        .send_bump(ID1, &sign_bump(&seat_key(0), 0, &CID, &g0(), &ID1).unwrap())
        .unwrap();
    b.receive(&wire).unwrap();
    assert_eq!(b.apply_bump(g0(), ID1), Ok(MOVED));
    assert_eq!(b.receive(&straggler0).unwrap().1, b"late in gen 0");

    c.receive(&wire).unwrap();
    assert_eq!(c.apply_bump(g0(), ID1), Ok(MOVED));
    let another0 = {
        // A second straggler, sealed by a member still in generation 0.
        let mut late = member(2);
        late.receive(&straggler0).unwrap();
        late.send(b"also late").unwrap()
    };
    let straggler1 = c.send(b"late in gen 1").unwrap();
    let wire2 = a
        .send_bump(ID2, &sign_bump(&seat_key(0), 0, &CID, &ID1, &ID2).unwrap())
        .unwrap();
    b.receive(&wire2).unwrap();
    assert_eq!(b.apply_bump(ID1, ID2), Ok(MOVED));
    assert_eq!(b.generation_id(), ID2);
    // One generation back still opens. Two back is gone.
    assert_eq!(b.receive(&straggler1).unwrap().1, b"late in gen 1");
    assert_eq!(
        b.receive(&another0),
        Err(ChatError::NeedsGeneration(gen0_id(&CID)))
    );
    // Kept across export and import.
    let state = b.export_state();
    let mut back = ChatSession::import_state(&ROOT, CID, 3, 1, &state).unwrap();
    assert_eq!(back.generation_id(), ID2);
    assert_eq!(
        back.receive(&another0),
        Err(ChatError::NeedsGeneration(gen0_id(&CID)))
    );
    let w = a.send(b"in gen 2").unwrap();
    assert_eq!(back.receive(&w).unwrap().1, b"in gen 2");
}

#[test]
fn a_device_without_the_root_exports_and_imports_its_generation() {
    let key0 = chat_gen_key(&ROOT, &CID, &gen0_id(&CID));
    let mut d = synced(ChatSession::from_generation(gen0_id(&CID), key0, CID, 3, 2));
    let mut a = member(0);
    let w = d.send(b"from a device").unwrap();
    assert_eq!(a.receive(&w).unwrap().1, b"from a device");
    // Root epochs need the root: a device without it cannot read old history.
    let old = v5_wires();
    assert_eq!(d.receive(&old[0]), Err(ChatError::UnknownEpoch));
    let state = d.export_state();
    let mut back = ChatSession::import_generation_state(CID, 3, 2, &state).unwrap();
    assert_eq!(back.generation_id(), gen0_id(&CID));
    back.mark_synced();
    let w = back.send(b"again").unwrap();
    assert_eq!(parse(&w).unwrap().0.counter, 1);
    assert_eq!(a.receive(&w).unwrap().1, b"again");
}

#[test]
fn only_a_bump_from_the_current_generation_to_a_new_one_is_sent() {
    // A device without the root sends a bump only from the generation it is in: one from a
    // generation it has left, or to one it already holds, would move nobody.
    let key0 = chat_gen_key(&ROOT, &CID, &gen0_id(&CID));
    let mut d = synced(ChatSession::from_generation(gen0_id(&CID), key0, CID, 3, 2));
    d.install_generation(ID1, chat_gen_key(&ROOT, &CID, &ID1));
    let from_old = sign_bump(&seat_key(2), 2, &CID, &g0(), &ID2).unwrap();
    assert_eq!(d.send_bump(ID2, &from_old), Err(ChatError::CannotBump));
    let to_held = sign_bump(&seat_key(2), 2, &CID, &ID1, &g0()).unwrap();
    assert_eq!(d.send_bump(g0(), &to_held), Err(ChatError::CannotBump));

    // A root holder refuses a bump to the generation it is already in, and to one it has left.
    let mut a = member(0);
    let to_gen0 = sign_bump(&seat_key(0), 0, &CID, &g0(), &g0()).unwrap();
    assert_eq!(a.send_bump(g0(), &to_gen0), Err(ChatError::CannotBump));
    let to_1 = sign_bump(&seat_key(0), 0, &CID, &g0(), &ID1).unwrap();
    a.send_bump(ID1, &to_1).unwrap();
    assert_eq!(a.send_bump(ID1, &to_1), Err(ChatError::CannotBump));
    let back_to_0 = sign_bump(&seat_key(0), 0, &CID, &ID1, &g0()).unwrap();
    assert_eq!(a.send_bump(g0(), &back_to_0), Err(ChatError::CannotBump));
    // A payload that moves from a generation other than the current one is refused.
    let from_old = sign_bump(&seat_key(0), 0, &CID, &g0(), &ID2).unwrap();
    assert_eq!(a.send_bump(ID2, &from_old), Err(ChatError::CannotBump));
    // A payload for another id is refused before anything is sent.
    let to_2 = sign_bump(&seat_key(0), 0, &CID, &ID1, &ID2).unwrap();
    assert_eq!(a.send_bump(ID3, &to_2), Err(ChatError::BadSignature));
    // Nothing was sent by the refusals: a's next message is counter 0 of generation ID1.
    let w = a.send(b"still here").unwrap();
    let h = parse(&w).unwrap().0;
    assert_eq!(h.epoch, EpochId::generation(generation_tag(&ID1), 1));
    assert_eq!(h.counter, 0);
}

/// A `CHS3` state for seat 2 of 3 with no epoch states: the given pending flag, retired ids and
/// generations (id, key). The layout is the one `export_state` writes.
fn chs3(retired: &[[u8; 32]], gens: &[([u8; 32], [u8; 32])]) -> Vec<u8> {
    let mut s = b"CHS3".to_vec();
    s.extend_from_slice(&[2, 3]);
    s.extend_from_slice(&0u32.to_le_bytes()); // sent_in_current
    s.extend_from_slice(&[0; 9]); // old root epochs: highest, floor, no states
    s.push(0); // no pending generation
    s.push(retired.len() as u8);
    for id in retired {
        s.extend_from_slice(id);
    }
    s.push(gens.len() as u8);
    for (id, key) in gens {
        s.extend_from_slice(id);
        s.extend_from_slice(key);
        s.extend_from_slice(&[0; 9]); // highest, floor, no states
    }
    s
}

#[test]
fn a_damaged_state_does_not_import() {
    let k = [7u8; 32];
    let import = |s: &[u8]| ChatSession::import_generation_state(CID, 3, 2, s).err();
    // The well-formed state imports, so each failure below is the change it makes.
    assert_eq!(import(&chs3(&[], &[(ID1, k)])), None);
    assert_eq!(
        import(&chs3(&[ID2], &[(ID1, k), (ID2.map(|b| b ^ 1), k)])),
        None
    );
    // No generation, or more than two.
    assert_eq!(import(&chs3(&[], &[])), Some(ChatError::State));
    assert_eq!(
        import(&chs3(&[], &[(ID1, k), (ID2, k), ([0x33; 32], k)])),
        Some(ChatError::State)
    );
    // The same generation twice, or a held generation also listed as dropped.
    assert_eq!(
        import(&chs3(&[], &[(ID1, k), (ID1, k)])),
        Some(ChatError::State)
    );
    assert_eq!(import(&chs3(&[ID1], &[(ID1, k)])), Some(ChatError::State));
    // More than four dropped generations.
    let five: Vec<[u8; 32]> = (0..5u8).map(|i| [0x40 + i; 32]).collect();
    assert_eq!(import(&chs3(&five[..4], &[(ID1, k)])), None);
    assert_eq!(import(&chs3(&five, &[(ID1, k)])), Some(ChatError::State));

    // The trailer of the current form: a flag that is not 0 or 1 does not import, and
    // neither does a state cut short of it.
    let good = member(2).export_state().to_vec();
    assert_eq!(&good[..4], b"CHS4");
    assert!(ChatSession::import_state(&ROOT, CID, 3, 2, &good).is_ok());
    let mut bad = good.clone();
    *bad.last_mut().unwrap() = 2;
    assert_eq!(
        ChatSession::import_state(&ROOT, CID, 3, 2, &bad).err(),
        Some(ChatError::State)
    );
    let short = &good[..good.len() - 1];
    assert_eq!(
        ChatSession::import_state(&ROOT, CID, 3, 2, short).err(),
        Some(ChatError::State)
    );

    // A state with root epochs does not import without the root.
    let mut b = member(1);
    b.receive(&v5_wires()[0]).unwrap();
    let with_old = b.export_state();
    assert!(ChatSession::import_state(&ROOT, CID, 3, 1, &with_old).is_ok());
    assert_eq!(
        ChatSession::import_generation_state(CID, 3, 1, &with_old).err(),
        Some(ChatError::State)
    );
    // Nor does the state written before generations.
    assert_eq!(
        ChatSession::import_generation_state(CID, 3, 1, &fixture("chat-v5.bin")).err(),
        Some(ChatError::State)
    );
}

#[test]
fn a_bump_moves_only_from_the_generation_it_was_sealed_in_and_names() {
    let mut a = member(0);
    let mut b = member(1);
    let first = sign_bump(&seat_key(0), 0, &CID, &g0(), &ID1).unwrap();
    let w1 = a.send_bump(ID1, &first).unwrap();
    b.receive(&w1).unwrap();
    assert_eq!(b.apply_bump(g0(), ID1), Ok(MOVED));
    let w2 = a
        .send_bump(ID2, &sign_bump(&seat_key(0), 0, &CID, &ID1, &ID2).unwrap())
        .unwrap();
    b.receive(&w2).unwrap();
    assert_eq!(b.apply_bump(ID1, ID2), Ok(MOVED));

    // The first bump's payload sealed again in the current generation: it moves from a
    // generation the circle left, so it is stale and moves nothing.
    let replay = a.send_typed(MessageType::Bump, &first).unwrap();
    let (_, got) = b.receive(&replay).unwrap();
    let (from, to, _) = verify_bump(&got, &CID, &manifest()).unwrap();
    assert_eq!(b.apply_bump(from, to), Ok(BumpOutcome::Stale));
    assert_eq!(b.generation_id(), ID2);

    // A bump that names the current generation but was sealed in the one before: stale.
    let mut c = member(2);
    c.receive(&w1).unwrap();
    assert_eq!(c.apply_bump(g0(), ID1), Ok(MOVED));
    let sideways = sign_bump(&seat_key(0), 0, &CID, &ID2, &ID3).unwrap();
    let in_old = c.send_typed(MessageType::Bump, &sideways).unwrap();
    assert_eq!(b.receive(&in_old).unwrap().0.msg_type, MessageType::Bump);
    assert_eq!(b.apply_bump(ID2, ID3), Ok(BumpOutcome::Stale));
    assert_eq!(b.generation_id(), ID2);

    // The same bump sealed in the generation it names moves.
    let in_current = a.send_typed(MessageType::Bump, &sideways).unwrap();
    b.receive(&in_current).unwrap();
    assert_eq!(b.apply_bump(ID2, ID3), Ok(MOVED));
    assert_eq!(b.generation_id(), ID3);
}

/// Two bumps from generation 0, to `low` and `high`, sealed by seats 0 and 1 of `a` and `b`.
fn concurrent(
    a: &mut ChatSession,
    b: &mut ChatSession,
    low: [u8; 32],
    high: [u8; 32],
) -> (Vec<u8>, Vec<u8>) {
    let wa = a
        .send_bump(
            high,
            &sign_bump(&seat_key(0), 0, &CID, &g0(), &high).unwrap(),
        )
        .unwrap();
    let wb = b
        .send_bump(low, &sign_bump(&seat_key(1), 1, &CID, &g0(), &low).unwrap())
        .unwrap();
    (wa, wb)
}

#[test]
fn two_bumps_at_once_end_in_the_lower_generation_for_everyone() {
    let (low, high) = (ID1, ID2);
    assert!(low < high);
    let mut a = member(0);
    let mut b = member(1);
    let mut c = member(2);
    // a sends something before it hears of b's bump.
    let (wa, wb) = concurrent(&mut a, &mut b, low, high);
    let early = a.send(b"from a, in its own generation").unwrap();

    // a reads b's bump: b's id is lower, so a follows it and drops its own.
    a.receive(&wb).unwrap();
    assert_eq!(
        a.apply_bump(g0(), low),
        Ok(BumpOutcome::Moved { dropped_own: true })
    );
    assert_eq!(a.generation_id(), low);
    // b reads a's bump: b's own id is lower, so b stays, and holds a's generation too.
    b.receive(&wa).unwrap();
    assert_eq!(b.apply_bump(g0(), high), Ok(BumpOutcome::Stale));
    assert_eq!(b.generation_id(), low);
    assert_eq!(
        b.receive(&early).unwrap().1,
        b"from a, in its own generation"
    );

    // c reads both, in either order, and ends in the lower one.
    c.receive(&wa).unwrap();
    assert_eq!(c.apply_bump(g0(), high), Ok(MOVED));
    c.receive(&wb).unwrap();
    assert_eq!(c.apply_bump(g0(), low), Ok(MOVED));
    assert_eq!(c.generation_id(), low);
    assert_eq!(
        c.receive(&early).unwrap().1,
        b"from a, in its own generation"
    );

    // Everyone reads everyone in the lower generation.
    let from_a = a.send(b"a").unwrap();
    let from_b = b.send(b"b").unwrap();
    let from_c = c.send(b"c").unwrap();
    for w in [&from_a, &from_b, &from_c] {
        assert_eq!(
            parse(w).unwrap().0.epoch.generation_parts().unwrap().0,
            generation_tag(&low)
        );
    }
    assert_eq!(a.receive(&from_b).unwrap().1, b"b");
    assert_eq!(a.receive(&from_c).unwrap().1, b"c");
    assert_eq!(b.receive(&from_a).unwrap().1, b"a");
    assert_eq!(b.receive(&from_c).unwrap().1, b"c");
    assert_eq!(c.receive(&from_a).unwrap().1, b"a");
    assert_eq!(c.receive(&from_b).unwrap().1, b"b");

    // Kept across export and import: the losing bump read again moves nobody.
    let mut back = ChatSession::import_state(&ROOT, CID, 3, 0, &a.export_state()).unwrap();
    back.receive(&wb).ok();
    assert_eq!(back.generation_id(), low);
    // b holds both new generations now. Generation 0 was dropped to make room, so the losing
    // bump does not open again.
    let mut back_b = ChatSession::import_state(&ROOT, CID, 3, 1, &b.export_state()).unwrap();
    assert_eq!(back_b.receive(&wa), Err(ChatError::NeedsGeneration(g0())));
    assert_eq!(back_b.receive(&early), Err(ChatError::Replay));
    assert_eq!(back_b.generation_id(), low);
}

#[test]
fn a_device_that_followed_the_losing_bump_needs_the_winner() {
    let (low, high) = (ID1, ID2);
    let mut a = member(0);
    let mut b = member(1);
    let mut d = synced(ChatSession::from_generation(
        g0(),
        chat_gen_key(&ROOT, &CID, &g0()),
        CID,
        3,
        2,
    ));
    let (wa, wb) = concurrent(&mut a, &mut b, low, high);
    // The device reads a's bump first and is given its key.
    d.receive(&wa).unwrap();
    assert_eq!(
        d.apply_bump(g0(), high),
        Err(ChatError::NeedsGeneration(high))
    );
    d.install_generation(high, chat_gen_key(&ROOT, &CID, &high));
    assert_eq!(d.generation_id(), high);
    // Then b's, with the lower id: the device needs that key, and keeps its own meanwhile.
    d.receive(&wb).unwrap();
    assert_eq!(
        d.apply_bump(g0(), low),
        Err(ChatError::NeedsGeneration(low))
    );
    assert_eq!(d.generation_id(), high);
    let w = b.send(b"in the winner").unwrap();
    assert_eq!(d.receive(&w), Err(ChatError::NeedsGeneration(low)));
    // Kept across export and import.
    let mut d = ChatSession::import_generation_state(CID, 3, 2, &d.export_state()).unwrap();
    d.mark_synced();
    assert_eq!(d.receive(&w), Err(ChatError::NeedsGeneration(low)));
    d.install_generation(low, chat_gen_key(&ROOT, &CID, &low));
    assert_eq!(d.generation_id(), low);
    assert_eq!(d.receive(&w).unwrap().1, b"in the winner");
    let back = d.send(b"from d").unwrap();
    assert_eq!(b.receive(&back).unwrap().1, b"from d");
    // A second competing bump from generation 0 is compared with the winner, not with the
    // device's first generation.
    let lower = [0x01; 32];
    let wl = {
        let mut e = member(0);
        e.send_bump(
            lower,
            &sign_bump(&seat_key(0), 0, &CID, &g0(), &lower).unwrap(),
        )
        .unwrap()
    };
    assert_eq!(d.receive(&wl), Err(ChatError::NeedsGeneration(g0())));
}

#[test]
fn an_own_bump_that_wins_is_not_dropped() {
    let (low, high) = (ID1, ID2);
    let mut a = member(0);
    let mut b = member(1);
    let (wa, wb) = concurrent(&mut a, &mut b, low, high);
    // b made the lower one: reading a's moves nothing and drops nothing.
    b.receive(&wa).unwrap();
    assert_eq!(b.apply_bump(g0(), high), Ok(BumpOutcome::Stale));
    // a made the higher one: its own is dropped.
    a.receive(&wb).unwrap();
    assert_eq!(
        a.apply_bump(g0(), low),
        Ok(BumpOutcome::Moved { dropped_own: true })
    );
    // A bump a only followed is not its own: a later loss drops nothing of its own.
    let mut c = member(2);
    c.receive(&wa).unwrap();
    assert_eq!(c.apply_bump(g0(), high), Ok(MOVED));
    c.receive(&wb).unwrap();
    assert_eq!(c.apply_bump(g0(), low), Ok(MOVED));
}
