use ipfsapi::IpfsApi;
use futures::future::{join_all, join};

use executor::Result;
use std::sync::Arc;

pub mod net;
pub mod api;

pub async fn apply(api: &IpfsApi, method: &str, args: &[&str]) -> Result<String> {
    let method = api.cat(method);
    let args = args.iter().map(|hash| {
        api.cat(hash)
    }).collect::<Vec<_>>();

    let (wasm, args) = join(method, join_all(args)).await;
    let args = args.into_iter().collect::<Result<Vec<_>, _>>()?;
    let args = args.iter().map(|v| v.as_ref()).collect::<Vec<_>>();
    let res = executor::exec(wasm?.as_ref(), &args)?;

    let hash = api.add(bytes::Bytes::from(res)).await?;
    return Ok(hash);
}


pub async fn run() {
    let api = IpfsApi::new("localhost", 5001);
    let api = Arc::new(api);
    tokio::spawn(api::run(api));
    tokio::spawn(net::run()).await.unwrap()

    /*
    let api = IpfsApi::new("localhost", 5001);

    let res = apply(
        &api,
        "QmTNrx5m787uBudCyA9eE8iyQFrMPdC1F8VzXB2DTQem38",
        &["QmQbkRGwgdZs31nhrmubSDmyvAyxaBwRSpJ66qwAnzjEpP"]).await.unwrap();

    panic!("{:?} - ", res);

     */
}