use cahoots_station::log::{entry_hash, Log, LogError, GENESIS};
use cahoots_station::rules::Submission;

fn cid(b: u8) -> [u8; 16] {
    [b; 16]
}

fn chat_msg(cid: [u8; 16], epoch: [u8; 5], sender: u8, counter: u32, pt_len: u32) -> Vec<u8> {
    let mut m = vec![2u8];
    m.extend_from_slice(&cid);
    m.extend_from_slice(&epoch);
    m.push(sender);
    m.extend_from_slice(&counter.to_le_bytes());
    m.extend_from_slice(&pt_len.to_le_bytes());
    m.push(0);
    m.extend(std::iter::repeat_n(0xCCu8, pt_len as usize + 16));
    m
}

#[test]
fn hash_chains_from_genesis() {
    let h0 = entry_hash(&GENESIS, 0, 1000, b"abc");
    let h1 = entry_hash(&h0, 1, 1001, b"def");
    assert_ne!(h0, GENESIS);
    assert_ne!(h0, h1);
    // Changing any input changes the hash.
    assert_ne!(entry_hash(&GENESIS, 0, 1000, b"abd"), h0);
    assert_ne!(entry_hash(&GENESIS, 1, 1000, b"abc"), h0);
    assert_ne!(entry_hash(&GENESIS, 0, 1001, b"abc"), h0);
}

#[test]
fn append_and_replay_verify_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let (seq1, h1) = {
        let mut log = Log::open(&path).unwrap();
        assert_eq!(log.head().seq, None);
        let m = chat_msg(cid(1), [0, 0, 0, 0, 0], 0, 0, 3);
        let sub = Submission::parse(&cid(1), &m).unwrap();
        let e = log.append(sub, 5).unwrap();
        assert_eq!(e.seq, 0);
        let m2 = chat_msg(cid(1), [0, 0, 0, 0, 0], 0, 1, 3);
        let e2 = log
            .append(Submission::parse(&cid(1), &m2).unwrap(), 6)
            .unwrap();
        assert_eq!(e2.seq, 1);
        assert_eq!(e2.hash, entry_hash(&e.hash, 1, 6, &m2));
        (e2.seq, e2.hash)
    };
    let log = Log::open(&path).unwrap();
    assert_eq!(log.head().seq, Some(seq1));
    assert_eq!(log.head().hash, h1);
    assert_eq!(log.entries_after(&cid(1), None).len(), 2);
    assert_eq!(log.entries_after(&cid(1), Some(0)).len(), 1);
    assert_eq!(log.entries_after(&cid(2), None).len(), 0);
}

#[test]
fn broken_chain_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    {
        let mut log = Log::open(&path).unwrap();
        for i in 0..3 {
            let m = chat_msg(cid(1), [0; 5], 0, i, 1);
            log.append(Submission::parse(&cid(1), &m).unwrap(), 1)
                .unwrap();
        }
    }
    // Tamper with the middle entry's message: its first byte (the version) 2 becomes 3.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let before: Vec<u8> = conn
            .query_row("SELECT message FROM entries WHERE seq = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut after = before.clone();
        after[0] = 3;
        conn.execute(
            "UPDATE entries SET message = ?1 WHERE seq = 1",
            rusqlite::params![after],
        )
        .unwrap();
        let stored: Vec<u8> = conn
            .query_row("SELECT message FROM entries WHERE seq = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_ne!(stored, before, "test must actually tamper");
    }
    assert!(matches!(
        Log::open(&path),
        Err(LogError::BrokenChain { seq: 1 })
    ));
}

#[test]
fn pure_pad_reuse_is_refused_after_replay_too() {
    use cahoots_core::message::{seal, MessageType};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let pad: Vec<u8> = (0..64).map(|i| i as u8).collect();
    let m1 = seal(&cid(7), 0, 100, MessageType::Text, b"hi", &pad[..34]).unwrap();
    let m_overlap = seal(&cid(7), 0, 120, MessageType::Text, b"yo", &pad[..34]).unwrap();
    let m_adjacent = seal(&cid(7), 0, 134, MessageType::Text, b"ok", &pad[..34]).unwrap();
    let m_other_slice = seal(&cid(7), 1, 100, MessageType::Text, b"ok", &pad[..34]).unwrap();
    let m_other_circle = seal(&cid(8), 0, 100, MessageType::Text, b"ok", &pad[..34]).unwrap();
    {
        let mut log = Log::open(&path).unwrap();
        log.append(Submission::parse(&cid(7), &m1).unwrap(), 1)
            .unwrap();
        let err = log
            .append(Submission::parse(&cid(7), &m_overlap).unwrap(), 2)
            .unwrap_err();
        assert!(matches!(err, LogError::Reuse { .. }), "{err:?}");
        assert!(matches!(err, LogError::Reuse { prior_seq: 0, .. }));
        log.append(Submission::parse(&cid(7), &m_adjacent).unwrap(), 3)
            .unwrap();
        log.append(Submission::parse(&cid(7), &m_other_slice).unwrap(), 4)
            .unwrap();
        log.append(Submission::parse(&cid(8), &m_other_circle).unwrap(), 5)
            .unwrap();
    }
    let mut log = Log::open(&path).unwrap();
    assert!(matches!(
        log.append(Submission::parse(&cid(7), &m1).unwrap(), 9),
        Err(LogError::Reuse { .. })
    ));
    let circles = log.circles();
    assert_eq!(circles.len(), 2);
    assert_eq!(
        circles
            .iter()
            .find(|c| c.circle_id == cid(7))
            .unwrap()
            .count,
        3
    );
}

#[test]
fn chat_messages_are_never_refused_for_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = Log::open(&dir.path().join("s.db")).unwrap();
    let m = chat_msg(cid(1), [0, 7, 0, 0, 0], 2, 0, 10);
    log.append(Submission::parse(&cid(1), &m).unwrap(), 1)
        .unwrap();
    log.append(Submission::parse(&cid(1), &m).unwrap(), 2)
        .unwrap();
    assert_eq!(log.head().seq, Some(1));
}

#[test]
fn database_survives_reopen_and_chain_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("station.db");
    let head = {
        let mut log = Log::open(&path).unwrap();
        let mut prev = GENESIS;
        for i in 0..3u32 {
            let m = chat_msg(cid(4), [0; 5], 1, i, 2);
            let e = log
                .append(Submission::parse(&cid(4), &m).unwrap(), 100 + i as u64)
                .unwrap();
            assert_eq!(e.seq, i as u64);
            assert_eq!(e.hash, entry_hash(&prev, i as u64, 100 + i as u64, &m));
            prev = e.hash;
        }
        assert!(log.bytes() > 0);
        log.head()
    };
    let log = Log::open(&path).unwrap();
    assert_eq!(log.head(), head);
    assert_eq!(log.head().seq, Some(2));
    assert_eq!(log.len(), 3);
    assert_eq!(log.circle_head(&cid(4)), head);
    let all = log.all_after(None, 10);
    assert_eq!(all.len(), 3);
    assert_eq!(all[1].received_at, 101);
    assert_eq!(all[1].message, chat_msg(cid(4), [0; 5], 1, 1, 2));
    let page = log.all_after(Some(0), 1);
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].seq, 1);
}
