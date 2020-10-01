pub use std::{
    sync::Arc,
    str::FromStr,
    task::{Poll, Context},
    collections::{
        HashMap,
        HashSet,
        VecDeque,
    },
    cmp,
    convert::TryFrom,
    pin::Pin,
};

pub use futures::{AsyncRead, AsyncWrite, Future, FutureExt, StreamExt};
pub use tokio::{
    sync::mpsc::{UnboundedSender, UnboundedReceiver, unbounded_channel as unbounded},
    sync::oneshot::{Sender as OneSender, Receiver as OneReceiver, channel as oneshot},
};

pub use anyhow::Result;

pub use cid::Cid;
pub use multihash::{
    Multihash,
    MultihashDigest,
    StatefulHasher
};

pub use libp2p::multiaddr::Multiaddr;

pub fn base58(d: &[u8]) -> String {
    bs58::encode(&d).into_string()
}

pub fn exec_id(method: &Cid, args: &[Cid]) -> Vec<u8> {
    use multihash::Hasher;
    let mut id = multihash::Sha2_256::default();

    id.update(&method.to_bytes());
    for i in args {
        id.update(&i.to_bytes());
    }

    return id.finalize().as_ref().to_vec();
}
