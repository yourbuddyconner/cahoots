//! Cards in a pile find out where they are, then run the ceremony over the pile.

mod common;

use cahoots_core::checkwords::check_words;
use cahoots_core::params::CircleParams;
use cahoots_hal::pile::{Pile, PileError, Ready};
use common::{cable, contribution, drive, open_end, Det, End};

/// `n` cards stacked: card 0 at the bottom. Each proposes its own name; only the bottom's counts.
fn stack(n: usize) -> Vec<Pile<End>> {
    let mut ups: Vec<Option<End>> = (0..n).map(|_| None).collect();
    let mut downs: Vec<Option<End>> = (0..n).map(|_| None).collect();
    for i in 0..n - 1 {
        let (lower, upper) = cable();
        ups[i] = Some(lower);
        downs[i + 1] = Some(upper);
    }
    (0..n)
        .map(|i| {
            let down = downs[i].take().unwrap_or_else(open_end);
            let up = ups[i].take().unwrap_or_else(open_end);
            Pile::new(down, up, &format!("card{i}"), 4096, false)
        })
        .collect()
}

fn settle(cards: &mut [Pile<End>]) -> Vec<Option<Ready>> {
    let mut out: Vec<Option<Ready>> = cards.iter().map(|_| None).collect();
    for _ in 0..20 {
        for (i, c) in cards.iter_mut().enumerate() {
            if let Some(r) = c.poll().unwrap() {
                out[i] = Some(r);
            }
        }
    }
    out
}

#[test]
fn three_stacked_cards_learn_their_seats_then_the_top_announces_the_count() {
    let mut cards = stack(3);
    settle(&mut cards);
    assert_eq!(
        cards.iter().map(|c| c.seat()).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2)]
    );
    assert!(
        cards.iter().all(|c| c.count().is_none()),
        "nobody has pressed the button"
    );
    assert_eq!(cards[0].start().unwrap_err(), PileError::NotTop);
    cards[2].start().unwrap();
    let ready = settle(&mut cards);
    for (i, r) in ready.iter().enumerate() {
        let r = r.as_ref().expect("every card is ready");
        assert_eq!(r.seat, i as u8);
        assert_eq!(r.params.n, 3);
        assert_eq!(r.params.pad_len, 4096);
        assert_eq!(r.params.name, "card0", "the bottom card names the circle");
    }
}

#[test]
fn a_card_placed_on_top_later_gets_the_next_seat() {
    let mut cards = stack(3);
    // Card 2 is not on the pile yet: its cable to card 1 reads as unplugged.
    cards[1].up_present_for_test().present.set(false);
    settle(&mut cards);
    assert_eq!(cards[1].seat(), Some(1));
    assert_eq!(
        cards[2].seat(),
        Some(0),
        "alone, a card is the bottom of a pile of one"
    );
    assert!(cards[2].is_bottom() && cards[2].is_top());
    // Now it lands.
    cards[1].up_present_for_test().present.set(true);
    settle(&mut cards);
    assert_eq!(cards[2].seat(), Some(2));
}

#[test]
fn a_pile_of_four_completes_a_ceremony() {
    let mut cards = stack(4);
    settle(&mut cards);
    cards[3].start().unwrap();
    let ready = settle(&mut cards);
    let mut devs: Vec<_> = cards
        .into_iter()
        .zip(ready)
        .enumerate()
        .map(|(i, (c, r))| {
            c.into_ceremony(r.unwrap(), &mut Det(7000 + i as u64), 0)
                .unwrap()
        })
        .collect();
    drive(&mut devs);
    let results: Vec<_> = devs.into_iter().map(|d| d.finish().unwrap().0).collect();
    let params = CircleParams::new(4, 4096, "card0", false).unwrap();
    let mut expected = vec![0u8; 4096];
    for i in 0..4u64 {
        for (e, c) in expected.iter_mut().zip(contribution(7000 + i, 4096)) {
            *e ^= c;
        }
    }
    let words = check_words(&params, &expected);
    for r in &results {
        assert_eq!(r.pad.as_slice(), expected.as_slice());
        assert_eq!(r.check_words, words);
    }
}

/// A `Ping` or `Log` frame on a link is ignored: it is a presence beacon or a log line, not a
/// pile message, so it must not disturb the seats or raise an error.
#[test]
fn ping_and_log_frames_are_ignored_by_the_pile() {
    use cahoots_core::frame::{encode, Frame};
    use cahoots_hal::pile::{KIND_LOG, KIND_PING};
    let (lower, upper) = cable();
    let inject_into_card1 = upper.rx.clone();
    let inject_into_card0 = lower.rx.clone();
    let mut cards = vec![
        Pile::new(open_end(), lower, "card0", 4096, false),
        Pile::new(upper, open_end(), "card1", 4096, false),
    ];
    settle(&mut cards);
    assert_eq!(cards[1].seat(), Some(1));
    for (kind, body) in [
        (KIND_PING, &b""[..]),
        (KIND_LOG, &b"hello from a board"[..]),
    ] {
        let bytes = encode(&Frame {
            kind,
            seq: 7,
            payload: body,
        });
        inject_into_card0.borrow_mut().extend(bytes.iter().copied());
        inject_into_card1.borrow_mut().extend(bytes.iter().copied());
    }
    settle(&mut cards);
    assert_eq!(cards[0].seat(), Some(0));
    assert_eq!(cards[1].seat(), Some(1));
    assert!(cards.iter().all(|c| c.count().is_none()));
}

/// A boxed link behaves like the link it holds, so one pile can mix link types.
#[test]
fn a_pile_can_be_built_from_boxed_links() {
    use cahoots_hal::Link;
    let (lower, upper) = cable();
    let down0: Box<dyn Link> = Box::new(open_end());
    let up0: Box<dyn Link> = Box::new(lower);
    let down1: Box<dyn Link> = Box::new(upper);
    let up1: Box<dyn Link> = Box::new(open_end());
    let mut cards = [
        Pile::new(down0, up0, "card0", 4096, false),
        Pile::new(down1, up1, "card1", 4096, false),
    ];
    for _ in 0..20 {
        for c in cards.iter_mut() {
            c.poll().unwrap();
        }
    }
    assert_eq!(cards[0].seat(), Some(0));
    assert_eq!(cards[1].seat(), Some(1));
}
