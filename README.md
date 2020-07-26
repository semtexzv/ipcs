# InterPlanetary Computation System
Is a peer-to-peer system for performing distributed computations. 

## How ?
The system uses WebAssembly to create portable, sandboxed binaries which have minimal interface, and uses IPFS 
to store the these binaries and data on which they operate in order to create perfectly secure computing environment.

It uses p2p communication between nodes in order to distribute the work of running individual functions between
multiple nodes.

## How to use this  ?
`cargo install ipcs-cli` and then use the `ipcs` command.
You can either start a node using `ipcs node` command (NOTE: Requires running IPFS deamon).

Or execute functions using `ipcs exec` agains the local node (NOTE: The node against which requests are submitted
does not execute functions, for debugging purposes it forcibly distributes them to connected peers).

```bash
ipcs node # Runs local API+worker node
ipcs node -n # Runs node without API, making it only a worker
ipcs exec QmT8MRDQxey9PVBWVRZCBDgXSg3mSQJ3a9y88pC8rAuNdz QmeMk2xH5DMpqumNn9F7vTYTgx51kSkso9WkQv35Gnn74D
# Runs QmT8MRDQxey9PVBWVRZCBDgXSg3mSQJ3a9y88pC8rAuNdz function on data in QmeMk2xH5DMpqumNn9F7vTYTgx51kSkso9WkQv35Gnn74D
```

### How to create new functions ?
Check out examples. There is a simple [ipcs-runtime](https://crates.io/crates/ipcs-runtime) crate for creating rust
functions (Note: functions need to be compiled using the `wasm32-unknown-unknown` target)
Then upload these functions using 
`ipfs add ./target/debug/<name>.wasm`
Upon uploading the function, you will be presented with ID of IPFS object, which contains function data, and can be used 
to execute the function

