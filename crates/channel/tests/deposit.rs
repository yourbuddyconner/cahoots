use cahoots_channel::deposit::{
    self, open_with_key, open_with_link, seal_for_key, seal_for_link, Deposit, KIND_KEY, KIND_LINK,
    XWING_ENC_LEN,
};
use cahoots_channel::keys::EpochId;
use cahoots_channel::link::LinkSecret;
use cahoots_channel::ChannelError;
use cahoots_hpke::{gen_keypair, Suite};
use sha2::{Digest, Sha256};

const CHANNEL: [u8; 16] = [3; 16];
const EPOCH_KEY: [u8; 32] = [0x42; 32];

fn epoch(n: u32) -> EpochId {
    EpochId {
        channel_id: CHANNEL,
        epoch: n,
    }
}

#[test]
fn link_deposit_opens_with_the_secret_and_not_another() {
    let secret = LinkSecret([9; 32]);
    let d = seal_for_link(&EPOCH_KEY, &secret, &epoch(5));
    assert_eq!(d.channel_id, CHANNEL);
    assert_eq!(d.epoch_id, epoch(5));
    assert_eq!(d.to, secret.id());
    assert_eq!(d.kind, KIND_LINK);
    assert_eq!(d.sealed.len(), 32 + 16);
    assert_eq!(*open_with_link(&d, &secret).unwrap(), EPOCH_KEY);

    // The encoding round-trips, and the id is the SHA-256 of it.
    let bytes = d.encode();
    assert_eq!(Deposit::decode(&bytes).unwrap(), d);
    assert_eq!(d.id(), <[u8; 32]>::from(Sha256::digest(&bytes)));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(Deposit::decode(&trailing), Err(ChannelError::Malformed));

    // Another secret has another seal key, whether or not `to` is changed to its id.
    assert_eq!(
        open_with_link(&d, &LinkSecret([8; 32])).map(|_| ()),
        Err(ChannelError::Tag)
    );
    let mut forged = d.clone();
    forged.to = LinkSecret([8; 32]).id();
    assert_eq!(
        open_with_link(&forged, &LinkSecret([8; 32])).map(|_| ()),
        Err(ChannelError::Tag)
    );
    // A key deposit is not opened as a link deposit.
    let mut wrong_kind = d.clone();
    wrong_kind.kind = KIND_KEY;
    assert_eq!(
        open_with_link(&wrong_kind, &secret).map(|_| ()),
        Err(ChannelError::Malformed)
    );
}

#[test]
fn key_deposit_opens_with_the_xwing_secret_key() {
    let mut rng = rand::rng();
    let (sk, pk) = gen_keypair(Suite::XWing, &mut rng);
    let (other_sk, _) = gen_keypair(Suite::XWing, &mut rng);
    let kid = [0x11; 32];
    let d = seal_for_key(&EPOCH_KEY, &epoch(2), kid, &pk, &mut rng).unwrap();
    assert_eq!(d.kind, KIND_KEY);
    assert_eq!(d.to, kid);
    assert_eq!(d.sealed.len(), XWING_ENC_LEN + 32 + 16);
    assert_eq!(Deposit::decode(&d.encode()).unwrap(), d);
    assert_eq!(*open_with_key(&d, &sk).unwrap(), EPOCH_KEY);

    assert_eq!(
        open_with_key(&d, &other_sk).map(|_| ()),
        Err(ChannelError::Hpke)
    );
    let mut short = d.clone();
    short.sealed.truncate(XWING_ENC_LEN);
    assert_eq!(
        open_with_key(&short, &sk).map(|_| ()),
        Err(ChannelError::Malformed)
    );
    assert_eq!(
        seal_for_key(&EPOCH_KEY, &epoch(2), kid, &pk[1..], &mut rng).map(|_| ()),
        Err(ChannelError::Hpke)
    );
}

#[test]
fn deposit_aad_binds_channel_epoch_and_recipient() {
    let mut want = deposit::DOMAIN.to_vec();
    want.extend_from_slice(&CHANNEL);
    want.extend_from_slice(&epoch(7).encode());
    want.extend_from_slice(&[5; 32]);
    assert_eq!(deposit::aad(&CHANNEL, &epoch(7), &[5; 32]), want);

    let secret = LinkSecret([9; 32]);
    let link = seal_for_link(&EPOCH_KEY, &secret, &epoch(7));
    let mut rng = rand::rng();
    let (sk, pk) = gen_keypair(Suite::XWing, &mut rng);
    let key = seal_for_key(&EPOCH_KEY, &epoch(7), [5; 32], &pk, &mut rng).unwrap();

    // Change `to`, the epoch, or the channel: neither kind opens.
    let edits: [fn(&mut Deposit); 3] = [
        |d| d.to[0] ^= 1,
        |d| d.epoch_id.epoch += 1,
        |d| {
            d.channel_id[0] ^= 1;
            d.epoch_id.channel_id[0] ^= 1;
        },
    ];
    for edit in edits {
        let mut l = link.clone();
        edit(&mut l);
        assert_eq!(
            open_with_link(&l, &secret).map(|_| ()),
            Err(ChannelError::Tag)
        );
        let mut k = key.clone();
        edit(&mut k);
        assert_eq!(open_with_key(&k, &sk).map(|_| ()), Err(ChannelError::Hpke));
    }
    // An epoch id naming another channel than the deposit's is refused when decoding.
    let mut mixed = link.clone();
    mixed.epoch_id.channel_id[0] ^= 1;
    assert_eq!(
        Deposit::decode(&mixed.encode()),
        Err(ChannelError::WrongChannel)
    );
}
