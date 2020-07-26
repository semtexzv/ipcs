use serde::{Deserialize, Serialize};


/// Request for executing a method
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecReq {
    /// Method being executed in. a base58 multihash of IPFS object
    pub method: String,
    /// Arguments passed to method. base58 hashes of IPFS objects
    pub args: Vec<String>,
}

/// Response provided when succesfully executing a function
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecResp {
    /// base58 multihash describing IPFS object in which the resulting data are stored
    pub hash: String,
}
