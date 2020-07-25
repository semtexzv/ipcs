use warp::{Filter, path};
use ipfsapi::IpfsApi;
use std::sync::Arc;

use apidefs::{ExecReq, ExecResp};
use futures::channel::mpsc::UnboundedSender;
use futures::SinkExt;

#[derive(Debug)]
pub struct Error(warp::http::StatusCode, String);

impl warp::reject::Reject for Error {}

pub async fn convert_err(reject: warp::reject::Rejection) -> Result<warp::reply::WithStatus<String>, warp::reject::Rejection> {
    if let Some(err) = reject.find::<Error>() {
        return Ok(warp::reply::with_status(err.1.clone(), err.0));
    }
    return Err(reject);
}

type State = (Arc<IpfsApi>, UnboundedSender<crate::IPCSCommand>);

pub async fn run(api: Arc<IpfsApi>, control: UnboundedSender<crate::IPCSCommand>) {
    let with_ipfs = warp::any().map(move || (api.clone(), control.clone()));
    let root = path("api")
        .and(with_ipfs);

    let v0 = root.and(path("v0"));

    let exec = v0.and(path("exec"))
        .and(warp::post())
        .and(warp::body::json::<ExecReq>())
        .and_then(exec)
        .recover(convert_err);

    let routes = exec;
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}


pub async fn exec((_, mut control): State, body: ExecReq) -> Result<impl warp::Reply, warp::reject::Rejection> {
    let (tx, rx) = futures::channel::oneshot::channel();
    control.send(crate::IPCSCommand::Exec(body.method, body.args, tx)).await.unwrap();
    let res = rx.await.unwrap();
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