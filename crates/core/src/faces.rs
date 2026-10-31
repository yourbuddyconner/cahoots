//! Cahoots, the faces: a 24 by 24 one-bit portrait from SHA-256 of a seed.
//!
//! This mirrors the generator in `site/art/gen.py` (`PUNK_LAYERS`, `punk_traits`,
//! `punk_layers`, `punk_pixels`) cell for cell. The Python stays the source for the site, so
//! the two must change together: a change to a sprite, a weight, `FACE_SHIFT`, `HAT_BRIM` or
//! the dotted skin goes into both, and `tests/fixtures/faces.txt` is made again from the
//! Python. `tests/faces.rs` checks this module against that file.
//!
//! The face is built from six layers drawn in order: head, hair, accessory, mouth, eyes,
//! hat. Layer `i` picks its trait with byte `i` of the digest, by weight out of 256. A
//! sprite is rows of `#` ink, `o` paper and `_` nothing, with its top left corner at
//! `(x, y)`. Head sprites also use `s` for skin, which is paper or, on a dotted head, a
//! checker. Nothing here allocates.

use sha2::{Digest, Sha256};

/// The side of a face, in cells.
pub const FACE: usize = 24;

/// The trait each layer picked, by the names the Python uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traits {
    /// Shape and skin, as `"square, plain"`.
    pub head: &'static str,
    pub hair: &'static str,
    pub accessory: &'static str,
    pub mouth: &'static str,
    pub eyes: &'static str,
    pub hat: &'static str,
}

struct Trait {
    name: &'static str,
    weight: u16,
    x: i32,
    y: i32,
    rows: &'static [&'static str],
}

const fn t(name: &'static str, weight: u16, x: i32, y: i32, rows: &'static [&'static str]) -> Trait {
    Trait { name, weight, x, y, rows }
}

const HEAD: usize = 0;
const HAIR: usize = 1;
const MOUTH: usize = 3;
const EYES: usize = 4;
const HAT: usize = 5;

// The common head: flat top, straight face, the jaw cut in under the mouth.
const SQUARE: &[&str] = &[
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "_______#########________",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "_______#ssssssss#_______",
    "________#####sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
];

// Rounder: the top corners and the jaw cut on a slant.
const ROUND: &[&str] = &[
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "________#######_________",
    "_______#sssssss#________",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#ssssssssss#______",
    "______#sssssssss#_______",
    "______#sssssssss#_______",
    "_______#ssssssss#_______",
    "________#sssssss#_______",
    "_________####sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
    "____________#sss#_______",
];

// Slim: set one cell further back, with a tapered jaw and a thin neck.
const SLIM: &[&str] = &[
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "________________________",
    "_________#######________",
    "________#sssssss#_______",
    "_______#sssssssss#______",
    "_______#sssssssss#______",
    "_______#sssssssss#______",
    "_______#sssssssss#______",
    "_______#ssssssssss#_____",
    "_______#ssssssssss#_____",
    "_______#ssssssssss#_____",
    "_______#ssssssssss#_____",
    "_______#sssssssss#______",
    "_______#sssssssss#______",
    "________#ssssssss#______",
    "_________#sssssss#______",
    "__________#####ss#______",
    "______________#ss#______",
    "______________#ss#______",
    "______________#ss#______",
    "______________#ss#______",
];

// HEAD_WEIGHTS square 128, round 64, slim 64; SKINS plain 160, dotted 96. A head's weight
// is the two multiplied, over 256. Shape is the index over 2, dotted the odd indexes.
const HEADS: &[Trait] = &[
    t("square, plain", 128 * 160 / 256, 0, 0, SQUARE),
    t("square, dotted", 128 * 96 / 256, 0, 0, SQUARE),
    t("round, plain", 64 * 160 / 256, 0, 0, ROUND),
    t("round, dotted", 64 * 96 / 256, 0, 0, ROUND),
    t("slim, plain", 64 * 160 / 256, 0, 0, SLIM),
    t("slim, dotted", 64 * 96 / 256, 0, 0, SLIM),
];

const HAIRS: &[Trait] = &[
    t("bald", 60, 0, 0, &[]),
    t("short", 70, 5, 3, &[
        "___#######____",
        "__##########__",
        "_############_",
        "#############_",
        "#_#_#__######_",
        "___________###",
        "___________##_",
    ]),
    t("mohawk", 40, 10, 0, &[
        "_#_",
        "###",
        "###",
        "###",
        "###",
        "###",
    ]),
    t("long", 46, 5, 4, &[
        "__#########___",
        "_###########__",
        "###_______###_",
        "##________###_",
        "##________####",
        "#_________####",
        "#_________####",
        "__________####",
        "__________####",
        "__________####",
        "___________###",
        "___________###",
        "___________###",
        "___________###",
        "___________##_",
    ]),
    t("wild", 40, 3, 1, &[
        "___#__#___#__#___",
        "__###_##_###_##__",
        "_###############_",
        "#################",
        "_###############_",
        "##################",
        "_##_#__#_#__######",
        "_#____________###",
        "______________##_",
    ]),
];

const ACCESSORIES: &[Trait] = &[
    t("none", 170, 0, 0, &[]),
    t("earring", 54, 16, 16, &[
        "_#_",
        "#o#",
        "_#_",
    ]),
    t("chain", 32, 11, 20, &[
        "_#####_",
        "#_____#",
        "_#####_",
        "___#___",
    ]),
];

const MOUTHS: &[Trait] = &[
    t("plain", 80, 8, 17, &["###"]),
    t("smile", 44, 8, 16, &[
        "___#",
        "###_",
    ]),
    t("moustache", 34, 7, 15, &[
        "_###_",
        "#####",
    ]),
    t("beard", 40, 6, 15, &[
        "_______####",
        "###########",
        "##ooo######",
        "###########",
        "_#########_",
        "__#####____",
    ]),
    t("pipe", 30, 1, 12, &[
        "_#______",
        "#_______",
        "_#______",
        "#_______",
        "________",
        "####_###",
        "#oo#####",
        "#oo#____",
        "_##_____",
    ]),
    t("cigarette", 28, 1, 11, &[
        "#______",
        "_#_____",
        "#______",
        "_#_____",
        "_______",
        "#######",
        "#ooooo#",
        "#######",
    ]),
];

const EYESES: &[Trait] = &[
    t("plain", 120, 7, 9, &[
        "oooooooo",
        "o##oo##o",
        "oooooooo",
        "o#ooo#oo",
        "oooooooo",
        "oooooooo",
    ]),
    t("shades", 56, 7, 11, &[
        "##########",
        "#o###o####",
        "_###_###__",
    ]),
    t("3D glasses", 44, 7, 10, &[
        "##########",
        "#ooo#o#o#_",
        "#ooo##o##_",
        "#########_",
    ]),
    t("monocle", 36, 8, 10, &[
        "___#####",
        "oo_#ooo#",
        "#o_#o#o#",
        "oo_#ooo#",
        "___#####",
        "_______#",
        "________",
        "_______#",
    ]),
];

const HATS: &[Trait] = &[
    t("none", 100, 0, 0, &[]),
    t("beanie", 34, 6, 1, &[
        "____###____",
        "____###____",
        "__#######__",
        "_#########_",
        "###########",
        "#o#o#o#o#o#",
        "###########",
    ]),
    t("cap", 36, 1, 3, &[
        "_______#########_",
        "______###########",
        "______###########",
        "_################",
        "#################",
    ]),
    t("fedora", 28, 4, 2, &[
        "_____##_##_____",
        "____#######____",
        "____#######____",
        "____#######____",
        "_#############_",
        "###############",
    ]),
    t("top hat", 22, 4, 0, &[
        "___#########___",
        "___#########___",
        "___#########___",
        "___#########___",
        "___#########___",
        "___#ooooooo#___",
        "___#########___",
        "_#############_",
    ]),
    t("bandana", 22, 5, 4, &[
        "__#########___",
        "_#o#o#o#o#o##_",
        "#############_",
        "____________###",
        "____________#_#",
    ]),
    t("crown", 14, 7, 1, &[
        "#___#___#",
        "##_###_##",
        "#o#o#o#o#",
        "#########",
        "#########",
    ]),
];

/// The layers in drawing order, as `PUNK_LAYERS`.
const LAYERS: [&[Trait]; 6] = [HEADS, HAIRS, ACCESSORIES, MOUTHS, EYESES, HATS];

/// `HAT_BRIM`, by hat: hair on this row and above it is not drawn.
const HAT_BRIM: [Option<i32>; 7] = [None, Some(7), Some(7), Some(7), Some(7), Some(6), Some(5)];
/// Under a hat only short or long hair shows, from this column.
const HAIR_UNDER_HAT_X: i32 = 15;
/// `HAIR_UNDER_HAT`: short and long, by hair index.
const HAIR_UNDER_HAT: [bool; 5] = [false, true, false, true, false];
/// `FACE_SHIFT`, by shape (square, round, slim): the eyes' and the mouth's (dx, dy).
const FACE_SHIFT: [[(i32, i32); 2]; 3] = [[(0, 0), (1, 0)], [(0, 0), (1, -1)], [(1, 0), (2, -1)]];
/// `UNSHIFTED`: the beard covers the jaw and stays put.
const BEARD: usize = 3;

const NONE: u8 = 0;
const INK: u8 = 1;
const PAPER: u8 = 2;

type Grid = [[u8; FACE]; FACE];

/// The trait whose range of the 256 byte values holds `byte`.
fn pick(traits: &[Trait], byte: u8) -> usize {
    let mut b = u16::from(byte);
    for (i, t) in traits.iter().enumerate() {
        if b < t.weight {
            return i;
        }
        b -= t.weight;
    }
    unreachable!("weights add up to 256")
}

fn picks(seed: &[u8]) -> [usize; 6] {
    let digest = Sha256::digest(seed);
    let mut out = [0; 6];
    for (i, layer) in LAYERS.iter().enumerate() {
        out[i] = pick(layer, digest[i]);
    }
    out
}

/// The traits this seed picks.
pub fn traits(seed: &[u8]) -> Traits {
    let p = picks(seed);
    Traits {
        head: HEADS[p[0]].name,
        hair: HAIRS[p[1]].name,
        accessory: ACCESSORIES[p[2]].name,
        mouth: MOUTHS[p[3]].name,
        eyes: EYESES[p[4]].name,
        hat: HATS[p[5]].name,
    }
}

/// A cell inside the face, or none.
fn cell(x: i32, y: i32) -> Option<(usize, usize)> {
    let r = 0..FACE as i32;
    (r.contains(&x) && r.contains(&y)).then_some((x as usize, y as usize))
}

/// How far a layer's trait moves on a head of this shape.
fn shift(layer: usize, pick: usize, shape: usize) -> (i32, i32) {
    match layer {
        MOUTH if pick == BEARD => (0, 0),
        EYES => FACE_SHIFT[shape][0],
        MOUTH => FACE_SHIFT[shape][1],
        _ => (0, 0),
    }
}

/// The cells one layer's trait draws on a head of this shape (`layer_pixels`), skin as paper.
fn layer_cells(layer: usize, pick: usize, shape: usize) -> Grid {
    let tr = &LAYERS[layer][pick];
    let (dx, dy) = shift(layer, pick, shape);
    let mut g = [[NONE; FACE]; FACE];
    for (ry, row) in tr.rows.iter().enumerate() {
        for (rx, ch) in row.bytes().enumerate() {
            if ch == b'_' {
                continue;
            }
            // Every sprite lies inside the face; the tests check each one.
            if let Some((x, y)) = cell(tr.x + rx as i32 + dx, tr.y + ry as i32 + dy) {
                g[y][x] = if ch == b'#' { INK } else { PAPER };
            }
        }
    }
    g
}

/// The hair after the hat clips it (`punk_layers`): under a hat only short or long hair
/// below the brim at the back of the head is kept, and any hair cell left with no hair
/// beside it is dropped.
fn clip_hair(g: &mut Grid, hair: usize, hat: usize) {
    if let Some(brim) = HAT_BRIM[hat] {
        let keep = HAIR_UNDER_HAT[hair];
        for (y, row) in g.iter_mut().enumerate() {
            for (x, c) in row.iter_mut().enumerate() {
                if !(keep && y as i32 > brim && x as i32 >= HAIR_UNDER_HAT_X) {
                    *c = NONE;
                }
            }
        }
    }
    let ink = *g;
    for (y, row) in g.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            if *c == NONE {
                continue;
            }
            let lonely = !neighbours(x, y, false).any(|(nx, ny)| ink[ny][nx] == INK);
            if lonely {
                *c = NONE;
            }
        }
    }
}

/// The cells around (x, y) inside the face, and (x, y) itself when `with_self`.
fn neighbours(x: usize, y: usize, with_self: bool) -> impl Iterator<Item = (usize, usize)> {
    (-1i32..=1)
        .flat_map(move |dy| (-1i32..=1).map(move |dx| (dx, dy)))
        .filter(move |&(dx, dy)| with_self || dx != 0 || dy != 0)
        .filter_map(move |(dx, dy)| cell(x as i32 + dx, y as i32 + dy))
}

/// The face, `true` for ink (`punk_pixels`).
///
/// Dotted skin is a checker laid on the skin once the head is drawn, kept off the outline.
/// Each later layer first clears the dots around its own ink, so every feature sits on a
/// ring of plain paper.
pub fn pixels(seed: &[u8]) -> [[bool; FACE]; FACE] {
    let p = picks(seed);
    let shape = p[HEAD] / 2;
    let dotted = p[HEAD] % 2 == 1;
    let mut grid = [[false; FACE]; FACE];
    let mut dots = [[false; FACE]; FACE];
    for layer in 0..LAYERS.len() {
        let mut g = layer_cells(layer, p[layer], shape);
        if layer == HAIR {
            clip_hair(&mut g, p[HAIR], p[HAT]);
        }
        for (y, row) in g.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                if c == INK {
                    for (nx, ny) in neighbours(x, y, true) {
                        if dots[ny][nx] {
                            dots[ny][nx] = false;
                            grid[ny][nx] = false;
                        }
                    }
                }
            }
        }
        for (y, row) in g.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                if c != NONE {
                    grid[y][x] = c == INK;
                    dots[y][x] = false;
                }
            }
        }
        if layer == HEAD && dotted {
            let head = &HEADS[p[HEAD]];
            for (ry, row) in head.rows.iter().enumerate() {
                for (rx, ch) in row.bytes().enumerate() {
                    let (x, y) = (head.x as usize + rx, head.y as usize + ry);
                    // `skin_ink`: the checker, which reads as a mid tone.
                    if ch == b's' && (x + y) % 2 == 0 {
                        dots[y][x] = true;
                        grid[y][x] = true;
                    }
                }
            }
        }
    }
    grid
}

/// The face as 24 rows of `#` ink and `.` paper, as `punk_pixels` returns it.
pub fn rows(seed: &[u8]) -> [[u8; FACE]; FACE] {
    let px = pixels(seed);
    let mut out = [[b'.'; FACE]; FACE];
    for (o, p) in out.iter_mut().zip(px.iter()) {
        for (c, &ink) in o.iter_mut().zip(p.iter()) {
            if ink {
                *c = b'#';
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_add_up_to_256() {
        for layer in LAYERS {
            assert_eq!(layer.iter().map(|t| t.weight).sum::<u16>(), 256);
        }
    }

    /// Python wraps a negative index and fails past the edge, so every sprite, shifted on
    /// every head, must lie inside the face for the two to agree.
    #[test]
    fn every_sprite_lies_inside_the_face() {
        for (layer, traits) in LAYERS.iter().enumerate() {
            for (pick, tr) in traits.iter().enumerate() {
                for shape in 0..3 {
                    let (dx, dy) = shift(layer, pick, shape);
                    for (ry, row) in tr.rows.iter().enumerate() {
                        for (rx, ch) in row.bytes().enumerate() {
                            if ch != b'_' {
                                let at = (tr.x + rx as i32 + dx, tr.y + ry as i32 + dy);
                                assert!(cell(at.0, at.1).is_some(), "{} at {at:?}", tr.name);
                            }
                        }
                    }
                }
            }
        }
    }
}
