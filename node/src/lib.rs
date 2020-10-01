#![deny(unused_must_use)]
#![allow(unused_imports)]

use futures::future::{join, join_all};

use executor::Result;

pub mod prelude;
pub mod api;
pub mod net;
pub mod block;

use crate::prelude::*;

#[derive(Debug)]
pub enum IPCSCommand {
    Exec(Cid, Vec<Cid>, OneSender<Result<Cid, String>>),
}


/// Configuration of IPCS node.
pub struct NodeConfig {
    /// Disable the provided HTTP API
    pub no_api: bool,
    /// URL pointing to IPFS node
    pub ipfs_url: String,
    /// Bootstrap multiaddrs
    pub bootstrap_nodes: Vec<String>,
    /// List of multiaddrs on which to listen
    pub listen_on: Vec<String>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            no_api: false,
            ipfs_url: format!("http://localhost:5001"),
            bootstrap_nodes: vec![],
            listen_on: vec![],
        }
    }
}

/// Run the node with provided config. Future should never resolve
pub async fn run(config: NodeConfig) {
    let (tx, rx) = unbounded();

    if !config.no_api {
        tokio::spawn(api::run(tx));
    }

    let bootstrap = config.bootstrap_nodes.into_iter().map(|v| libp2p::multiaddr::Multiaddr::from_str(&v).unwrap()).collect();
    let listen_on = config.listen_on.into_iter().map(|v| libp2p::multiaddr::Multiaddr::from_str(&v).unwrap()).collect();

    tokio::spawn(net::run(listen_on, bootstrap, rx)).await.unwrap()
}
