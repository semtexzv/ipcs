use warp::{Filter, path};
use serde::{Deserialize};
use ipfsapi::IpfsApi;
use std::sync::Arc;

use apidefs::{ExecReq, ExecResp};

#[derive(Debug)]
pub struct Error(warp::http::StatusCode, String);

impl warp::reject::Reject for Error {}

pub async fn convert_err(reject: warp::reject::Rejection) -> Result<warp::reply::WithStatus<String>, warp::reject::Rejection> {
    if let Some(err) = reject.find::<Error>() {
        return Ok(warp::reply::with_status(err.1.clone(), err.0));
    }
    return Err(reject);
}

pub async fn run(api: Arc<IpfsApi>) {
    let with_ipfs = warp::any().map(move || api.clone());
    let root = warp::path("api")
        .and(with_ipfs);

    let v0 = root.and(warp::path("v0"));

    let exec = v0.and(warp::path("exec"))
        .and(warp::post())
        .and(warp::body::json::<ExecReq>())
        .and_then(exec)
        .recover(convert_err);

    let routes = exec;
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}


pub async fn exec(ipfs: Arc<IpfsApi>, body: ExecReq) -> Result<impl warp::Reply, warp::reject::Rejection> {
    let args = body.args.iter().map(|v| v.as_ref()).collect::<Vec<_>>();
    let res = crate::apply(ipfs.as_ref(), &body.method, &args).await;
    match res {
        Ok(r) => {
            Ok(warp::reply::json(&ExecResp {
                hash: r
            }))
        }
        Err(e) => {
            Err(warp::reject::custom(Error(warp::http::StatusCode::BAD_REQUEST, e.to_string())))
        }
    }
}