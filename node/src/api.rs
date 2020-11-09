use apidefs::{ExecReq, ExecResp};

#[derive(Clone)]
pub struct State {
    ipfs: crate::Ipfs,
}

pub async fn exec(mut req: tide::Request<State>) -> tide::Result {
    let data = req.body_json::<ExecReq>().await?;
    let args = data.args.iter().map(|v| v.as_ref()).collect::<Vec<_>>();
    let res = crate::exec(&req.state().ipfs.clone(), &data.method, &args).await?;

    let resp = ExecResp {
        hash: res.to_string(),
    };

    Ok(tide::Response::from(serde_json::to_value(&resp)?))
}

pub async fn run(ipfs: crate::Ipfs) -> tide::Result<()> {
    let mut app = tide::with_state(State {
        ipfs,
    });
    app.at("/api/v0/exec").post(exec);
    app.listen(vec!["0.0.0.0:3030"]).await?;
    Ok(())
}