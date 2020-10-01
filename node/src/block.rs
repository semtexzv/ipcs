use std::sync::Arc;
use cid::Cid;
use cid::RAW;
use multihash::{Multihash, MultihashDigest, SHA2_256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub cid: Cid,
    pub data: Arc<[u8]>,
}

impl Block {
    pub fn new(data: Arc<[u8]>, cid: Cid) -> Self {
        Self { cid, data }
    }

    pub fn cid(&self) -> &Cid {
        &self.cid
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

pub fn create_block(bytes: Vec<u8>) -> Block {
    let digest = Multihash::new(SHA2_256, &bytes).unwrap().to_raw().unwrap();
    let cid = Cid::new_v1(RAW, digest);
    Block::new(bytes.into_boxed_slice().into(), cid)
}
