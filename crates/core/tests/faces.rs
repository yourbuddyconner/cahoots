//! The faces against `fixtures/faces.txt`, made from `site/art/gen.py`: every seed's rows and
//! traits must match the Python's exactly.

use cahoots_core::faces::{self, Traits, FACE};

struct Case {
    seed: Vec<u8>,
    rows: Vec<String>,
    traits: String,
}

fn cases() -> Vec<Case> {
    let text = include_str!("fixtures/faces.txt");
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        let name = line.strip_prefix("seed ").expect("a seed line");
        let seed = if name == "(empty)" { Vec::new() } else { name.as_bytes().to_vec() };
        let rows: Vec<String> = (0..FACE).map(|_| lines.next().expect("a row").to_string()).collect();
        let traits = lines
            .next()
            .and_then(|l| l.strip_prefix("traits "))
            .expect("a traits line")
            .to_string();
        out.push(Case { seed, rows, traits });
    }
    out
}

/// The traits as Python prints its dict.
fn python_dict(t: &Traits) -> String {
    format!(
        "{{'head': '{}', 'hair': '{}', 'accessory': '{}', 'mouth': '{}', 'eyes': '{}', 'hat': '{}'}}",
        t.head, t.hair, t.accessory, t.mouth, t.eyes, t.hat
    )
}

#[test]
fn every_seed_matches_the_python() {
    let cases = cases();
    assert_eq!(cases.len(), 15);
    for c in &cases {
        let name = String::from_utf8_lossy(&c.seed);
        let got: Vec<String> = faces::rows(&c.seed)
            .iter()
            .map(|r| String::from_utf8(r.to_vec()).unwrap())
            .collect();
        assert_eq!(got, c.rows, "rows of {name:?}");
        assert_eq!(python_dict(&faces::traits(&c.seed)), c.traits, "traits of {name:?}");
    }
}

#[test]
fn pixels_and_rows_agree() {
    for c in cases() {
        let px = faces::pixels(&c.seed);
        let rows = faces::rows(&c.seed);
        for y in 0..FACE {
            for x in 0..FACE {
                assert_eq!(px[y][x], rows[y][x] == b'#');
                assert!(rows[y][x] == b'#' || rows[y][x] == b'.');
            }
        }
    }
}
