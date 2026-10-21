#![no_main]
use arbitrary::Arbitrary;
use cahoots_core::ceremony::Ceremony;
use cahoots_core::params::CircleParams;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Op {
    Commit { from: u8, c: [u8; 32] },
    Reveal { from: u8, r: Vec<u8> },
    Phase,
    Missing,
}

#[derive(Arbitrary, Debug)]
struct Input {
    n: u8,
    pad_len: u16,
    agent: bool,
    index: u8,
    contribution: Vec<u8>,
    ops: Vec<Op>,
}

fuzz_target!(|input: Input| {
    let Ok(params) = CircleParams::new(input.n, u32::from(input.pad_len), "fuzz", input.agent) else {
        return;
    };
    let Ok(mut c) = Ceremony::new(params, input.index, input.contribution) else {
        return;
    };
    for op in input.ops {
        match op {
            Op::Commit { from, c: cm } => {
                let _ = c.add_commitment(from, cm);
            }
            Op::Reveal { from, r } => {
                let _ = c.add_reveal(from, &r);
            }
            Op::Phase => {
                let _ = c.phase();
                let _ = c.my_reveal();
            }
            Op::Missing => {
                let _ = c.missing_commitments();
                let _ = c.missing_reveals();
            }
        }
    }
    let _ = c.finish();
});
