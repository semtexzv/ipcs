#![deny(unused_must_use)]

use futures::future::{join, join_all};
use ipfsapi::IpfsApi;

use futures::channel::{mpsc::unbounded, oneshot::Sender};

use executor::Result;
use std::sync::Arc;

pub mod api;
pub mod net;

#[derive(Debug)]
pub enum IPCSCommand {
    Exec(String, Vec<String>, Sender<Result<String, String>>),
}

/// Executes function identified by [arg] has against arguments identified
/// by [arg] hashes
pub async fn exec(api: &IpfsApi, method: &str, args: &[&str]) -> Result<String> {
    let method = api.cat(method);
    let args = args.iter().map(|hash| api.cat(hash)).collect::<Vec<_>>();

    let (wasm, args) = join(method, join_all(args)).await;

    let res = tokio::task::spawn_blocking(move || {
        // TODO: Do not pre-download whole args, use file-like API for streaming
        let args = args.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        let args = args.iter().map(|v| v.as_ref()).collect::<Vec<_>>();

        executor::exec(wasm.unwrap().as_ref(), &args).unwrap()
    })
    .await
    .unwrap();

    let hash = api.add(bytes::Bytes::from(res)).await?;
    return Ok(hash);
}

/// Configuration of IPCS node.
pub struct NodeConfig {
    /// Disable the provided HTTP API
    pub no_api: bool,
    /// URL pointing to IPFS node
    pub ipfs_url: String,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            no_api: false,
            ipfs_url: format!("http://localhost:5001"),
        }
    }
}

/// Run the node with provided config. Future should never resolve
pub async fn run(config: NodeConfig) {
    let api = IpfsApi::new(&config.ipfs_url).unwrap();
    let api = Arc::new(api);

    let (tx, rx) = unbounded();

    if !config.no_api {
        tokio::spawn(api::run(api.clone(), tx));
    }

    tokio::spawn(net::run(api.clone(), rx)).await.unwrap()
}
