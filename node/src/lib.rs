#![deny(unused_must_use)]
#![allow(unused_imports)]


mod api;

use executor::Result;

use futures::future::{join, join_all};
use futures::channel::{mpsc::unbounded, oneshot::Sender};

use std::sync::Arc;
use std::str::FromStr;
use ipfs::{IpfsOptions, UninitializedIpfs, Cid};

//pub mod api;
//pub mod net;

pub type Ipfs = ipfs::Ipfs<ipfs::TestTypes>;

#[derive(Debug)]
pub struct ExecCommand {
    method: String,
    args: Vec<String>,
    ret: Sender<Result<String, String>>,
}

#[derive(Debug)]
pub enum IPCSCommand {
    Exec(ExecCommand),
}

/// Executes function identified by [arg] has against arguments identified
/// by [arg] hashes
pub async fn exec(ipfs: &Ipfs, method: &str, args: &[&str]) -> Result<String> {
    let method = Cid::from_str(method).unwrap();
    let method = ipfs.get_block(&method);
    let args = args.iter()
        .map(move |hash| {
            let cid = Cid::from_str(hash).unwrap();
            async move { ipfs.clone().get_block(&cid).await }
        })
        .collect::<Vec<_>>();

    let (wasm, args) = join(method, join_all(args)).await;

    let res = astd::task::spawn_blocking(move || {
        // TODO: Do not pre-download whole args, use file-like API for streaming
        let args = args.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        let args = args.iter().map(|v| v.data.as_ref()).collect::<Vec<_>>();

        executor::exec(wasm.unwrap().data.as_ref(), &args).unwrap()
    }).await;

    return Ok(unimplemented!());
    //let hash = api.add(bytes::Bytes::from(res)).await?;
    //return Ok(hash);
}

/// Configuration of IPCS node.
pub struct NodeConfig {
    /// Disable the provided HTTP API
    pub no_api: bool,
    /// Bootstrap multiaddrs
    pub bootstrap_nodes: Vec<String>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            no_api: false,
            bootstrap_nodes: vec![],
        }
    }
}

/// Run the node with provided config. Future should never resolve
pub async fn run(config: NodeConfig) {
    let ipfs = IpfsOptions::inmemory_with_generated_keys();
    let (ipfs, worker): (Ipfs, _) = UninitializedIpfs::new(ipfs).start().await.unwrap();
    astd::task::spawn(worker);
    api::run(ipfs.clone()).await.unwrap();
}
