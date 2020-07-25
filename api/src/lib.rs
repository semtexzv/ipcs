pub struct IpcsApi {
    url: url::Url,
    client: reqwest::Client,
}

impl IpcsApi {
    pub fn new(url: &str) -> Result<Self, url::ParseError> {
        Ok(IpcsApi {
            url: url::Url::parse(url)?,
            client: reqwest::Client::new(),
        })
    }

    pub async fn exec(&self, method: &str, args: &[&str]) -> Result<String, reqwest::Error> {
        let mut url = self.url.clone();
        url.set_path("/api/v0/exec");

        let res: apidefs::ExecResp = self
            .client
            .post(url)
            .json(&apidefs::ExecReq {
                method: method.to_string(),
                args: args.iter().map(|v| v.to_string()).collect(),
            })
            .send()
            .await?
            .json()
            .await?;

        Ok(res.hash)
    }
}
