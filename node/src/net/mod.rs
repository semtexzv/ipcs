use std::sync::Arc;
use std::task::Poll;

use futures::{channel::mpsc::UnboundedReceiver, future::poll_fn, Future, SinkExt, StreamExt};

use libp2p::{
    identify::{Identify, IdentifyEvent},
    identity,
    mdns::{Mdns, MdnsEvent},
    ping::{Ping, PingConfig, PingEvent},
    kad::{Kademlia, KademliaEvent, QueryResult, QueryId, GetClosestPeersResult, KademliaConfig, store::MemoryStore},
    swarm::{NetworkBehaviourEventProcess, SwarmBuilder},
    Multiaddr, NetworkBehaviour, PeerId, Swarm,
};
use tokio::macros::support::Pin;

use ipfsapi::IpfsApi;

//mod peerstore;
mod workswap;

//use peerstore::*;
use workswap::{ExecutionResult, Workswap, WorkswapEvent};
use std::collections::HashMap;
use futures::channel::oneshot::Sender;
use multihash::MultihashDigest;
use crate::net::workswap::{Cid, ExecutionID};

pub type WorkReturner = Sender<Result<workswap::Cid, String>>;

pub enum WorkStatus {
    PeerLookup(QueryId),
}

pub enum QueryType {
    /// DHT query for execution purposes
    Workswap(ExecutionID, Cid, Vec<Cid>)
}

/// Network behavior describing general properties of this node
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    #[behaviour(ignore)]
    ipfs: Arc<IpfsApi>,
    #[behaviour(ignore)]
    works: HashMap<workswap::ExecutionID, WorkReturner>,
    #[behaviour(ignore)]
    queries: HashMap<QueryId, QueryType>,
    /// Used to find peers on local network
    mdns: Mdns,
    ping: Ping,
    identify: Identify,
    kad: Kademlia<MemoryStore>,
    /// Behavior responsible for actually sending work to other nodes
    workswap: Workswap,
}

impl NetworkBehaviourEventProcess<()> for Behaviour {
    fn inject_event(&mut self, _: ()) {}
}

impl NetworkBehaviourEventProcess<MdnsEvent> for Behaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(list) => {
                for (peer, addr) in list {
                    log::trace!("mdns: Discovered peer {} on {:?}", peer.to_base58(), &addr);
                    // Workswap is core logic behavior, it handles connecting to new peers
                    self.workswap.connect(peer);
                }
            }
            MdnsEvent::Expired(_) => {}
        }
    }
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for Behaviour {
    fn inject_event(&mut self, event: IdentifyEvent) {
        log::trace!("identify: {:?}", event);
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for Behaviour {
    fn inject_event(&mut self, event: PingEvent) {
        log::trace!("ping: {:?}",event)
    }
}

impl NetworkBehaviourEventProcess<workswap::WorkswapEvent> for Behaviour {
    fn inject_event(&mut self, event: WorkswapEvent) {
        log::info!("workswap event: {:?}", event);

        match event {
            WorkswapEvent::Resolved(id, res) => {
                log::debug!("Network resolved work on {} res: {:?}", id, res);
                if let Some(ret) = self.works.remove(&id) {
                    ret.send(res).unwrap();
                }
            }
            WorkswapEvent::WantCalc(peer, id, method, args) => {
                log::debug!("peer {} wants us to work on {} ({}) applied on {:?}", peer, id, method, args);
                self.workswap.accept(&peer, id.clone());
                let ipfs = self.ipfs.clone();
                let mut fin = self.workswap.queued_works.clone();
                tokio::spawn(async move {
                    let args = args.iter().map(AsRef::as_ref).collect::<Vec<_>>();
                    let res = crate::exec(&ipfs, &method, &args).await;

                    // Simple error reporting to remote node
                    // TODO: Fix this crap
                    let res = ExecutionResult { id, result: res.map_err(|e| e.to_string()) };
                    fin.send((peer, res)).await.unwrap();
                });
            }
            WorkswapEvent::LocalErr(id, err) => {
                if let Some(ret) = self.works.remove(&id) {
                    ret.send(Err(err)).unwrap();
                }
            }
            WorkswapEvent::Accepted(peer, id) => {
                log::debug!("Peer {} accepted to work on {}", peer, id);
            }
            WorkswapEvent::Rejected(peer, id) => {
                log::debug!("Peer {} rejected to work on {}", peer, id);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<KademliaEvent> for Behaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        log::info!("kad: {:?}", event);
        match event {
            KademliaEvent::QueryResult { id, result, .. } => {
                self.kademlia_resolved(id, result);
            }
            _ => {}
        }
    }
}

impl Behaviour {
    pub fn add_peer(&mut self, _peer: PeerId, _addr: Multiaddr) {
        self.kad.add_address(&_peer, _addr);
    }

    pub fn remove_peer(&mut self, _peer: &PeerId) {
        self.kad.remove_peer(_peer);
    }

    fn kademlia_resolved(&mut self, id: QueryId, result: QueryResult) {
        match (self.queries.remove(&id), result) {
            (Some(QueryType::Workswap(eid, method, args)), QueryResult::GetClosestPeers(Ok(peers))) => {
                self.workswap.want_exec(eid, method, args)
            }
            (_, QueryResult::GetClosestPeers(Err(timeout))) => {
                log::error!("Timed our resolving kademlia nodes");
            }
            (None, v) => {}
            (_, v) => {
                log::info!("Unknown kademlia query resolved : {:?}", v)
            }
        }
    }

    pub fn want_exec(&mut self, id: ExecutionID, method: Cid, args: Vec<Cid>, ret: WorkReturner) {
        // TODO: Doing string-hash compare here, fix this
        let query = self.kad.get_closest_peers(id.as_bytes());
        self.queries.insert(query, QueryType::Workswap(id.clone(), method, args));
        self.works.insert(id, ret);
    }
}

pub async fn run(ipfs: Arc<IpfsApi>, mut control: UnboundedReceiver<crate::IPCSCommand>) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer id: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex and Yamux protocols
    let transport = libp2p::build_development_transport(local_key.clone()).unwrap();

    let mut swarm = {
        let mut kad_cfg = KademliaConfig::default();
        kad_cfg.set_protocol_name(&b"/ipcs/0.0.0"[..]);
        let kad = Kademlia::with_config(local_peer_id.clone(), MemoryStore::new(local_peer_id.clone()), kad_cfg);
        let behavior = Behaviour {
            ipfs,
            works: HashMap::new(),
            queries: HashMap::new(),
            mdns: Mdns::new().unwrap(),
            ping: Ping::new(PingConfig::new()),
            kad: kad,
            identify: Identify::new("/ipcs/0.0.0".into(), "rust-ipcs/0.0.0".into(), local_key.clone().public()),
            workswap: workswap::Workswap::new(),
        };

        // Use tokio for created tasks
        struct Exec;
        impl libp2p::core::Executor for Exec {
            fn exec(&self, future: Pin<Box<dyn Future<Output=()> + Send>>) {
                tokio::spawn(future);
            }
        }

        SwarmBuilder::new(transport, behavior, local_peer_id)
            .executor(Box::new(Exec))
            .build()
    };

    Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse().unwrap()).unwrap();

    loop {
        poll_fn(|ctx| -> Poll<Option<()>> {
            match swarm.poll_next_unpin(ctx) {
                Poll::Ready(Some(item)) => {
                    log::info!("Event: {:?}", item);
                }
                Poll::Ready(None) => panic!("Swarm closed"),
                Poll::Pending => {}
            }
            match control.poll_next_unpin(ctx) {
                Poll::Ready(Some(item)) => {
                    log::info!("Control: {:?}", item);
                    match item {
                        crate::IPCSCommand::Exec(method, args, ret) => {
                            // TODO: Use proper multihash, not hashing of strings here
                            let id = exec_id(&method, &args);
                            let id = base58(&id);
                            swarm.want_exec(id.clone(), method, args, ret);
                        }
                    }
                }
                Poll::Ready(None) => {
                    return Poll::Ready(None);
                }
                Poll::Pending => {}
            }

            Poll::Pending
        })
            .await;
    }
}

fn base58(d: &[u8]) -> String {
    bs58::encode(&d).into_string()
}

pub fn exec_id(method: &Cid, args: &[Cid]) -> Vec<u8> {
    let mut id = multihash::Sha2_256::default();
    id.input(method.as_bytes());
    for i in args {
        id.input(i.as_bytes());
    }

    return id.digest(&[]).to_vec();
}