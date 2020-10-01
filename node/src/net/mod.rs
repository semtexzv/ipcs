use crate::prelude::*;

use futures::{future::poll_fn, Future, SinkExt, StreamExt};

use libp2p::{
    identify::{Identify, IdentifyEvent},
    identity,
    mdns::{TokioMdns, MdnsEvent},
    ping::{Ping, PingConfig, PingEvent},
    kad::{Kademlia, KademliaEvent, QueryResult, QueryId, GetClosestPeersResult, KademliaConfig, store::MemoryStore},
    swarm::{NetworkBehaviourEventProcess, SwarmBuilder},
    Multiaddr, NetworkBehaviour, PeerId, Swarm,
};
use bitswap::{Bitswap, BitswapError, BitswapEvent};


mod workswap;

use workswap::{ExecutionResult, Workswap, WorkswapEvent};

use multihash::MultihashDigest;
use cid::Cid;
use crate::net::workswap::{ExecutionID};
use crate::block::Block;
use libp2p::swarm::NetworkBehaviourAction;

pub type WorkReturner = OneSender<Result<Cid, String>>;

pub struct WorkInfo {
    peer: PeerId,
    res_chan: WorkReturner,
    state: WorkState,
}

pub enum WorkState {
    Empty,
    Waiting(Cid, Vec<Cid>),
    Performing,
}

pub enum QueryType {
    /// DHT query for execution purposes
    Workswap(ExecutionID, Cid, Vec<Cid>)
}

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    FinishedLocalExec(ExecutionID, Block),
}

/// Network behavior describing general properties of this node
#[derive(NetworkBehaviour)]
#[behaviour(custom_poll = "poll_custom", out_event = "NetworkEvent")]
pub struct Behaviour {
    #[behaviour(ignore)]
    events: UnboundedReceiver<NetworkEvent>,
    #[behaviour(ignore)]
    events_sender: UnboundedSender<NetworkEvent>,
    #[behaviour(ignore)]
    local_works: HashMap<workswap::ExecutionID, WorkInfo>,
    #[behaviour(ignore)]
    queries: HashMap<QueryId, QueryType>,
    #[behaviour(ignore)]
    blocks: HashMap<Cid, Arc<[u8]>>,

    /// Used to find peers on local network
    mdns: TokioMdns,
    kad: Kademlia<MemoryStore>,
    ping: Ping,
    identify: Identify,
    /// Behavior responsible for actually sending work to other nodes
    workswap: Workswap,
    bitswap: Bitswap,
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
                    self.kad.add_address(&peer, addr);
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
                let info = if let Some(w) = self.local_works.remove(&id) { w } else { return; };
                info.res_chan.send(res.map_err(|e| format!("Network failed to process function: {}", e)))
                    .unwrap()
            }
            WorkswapEvent::WantCalc(peer, id, method, args) => {
                log::debug!("peer {} wants us to work on {} ({}) applied on {:?}", peer, id, method, args);
                self.workswap.accept(&peer, id.clone());
                self.bitswap.want_block(method, 0);
                for arg in args {
                    self.bitswap.want_block(arg, 0);
                }
            }
            WorkswapEvent::LocalErr(id, err) => {
                if let Some(info) = self.local_works.remove(&id) {
                    info.res_chan.send(Err(err)).unwrap();
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

impl NetworkBehaviourEventProcess<BitswapEvent> for Behaviour {
    fn inject_event(&mut self, event: BitswapEvent) {
        match event {
            BitswapEvent::ReceivedBlock(_, cid, data) => {
                log::info!("Received block: {:?}", cid);
                self.blocks.insert(cid, data.into());
            }
            BitswapEvent::ReceivedWant(_, cid, _) => {
                log::info!("Received want: {:?}", cid);
                if let Some(data) = self.blocks.get(&cid) {
                    self.bitswap.send_block_all(&cid, data);
                }
            }
            BitswapEvent::ReceivedCancel(_, _) => {}
        }

        self.poll_unblocked_work();
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
            (Some(QueryType::Workswap(eid, method, args)), QueryResult::GetClosestPeers(Ok(_peers))) => {
                self.workswap.want_exec(eid, method, args)
            }
            (_, QueryResult::GetClosestPeers(Err(_timeout))) => {
                log::error!("Timed our resolving kademlia nodes");
            }
            (None, _v) => {}
            (_, v) => {
                log::info!("Unknown kademlia query resolved : {:?}", v)
            }
        }
    }

    pub fn poll_unblocked_work(&mut self) {
        'outer: for (id, state) in self.local_works.iter_mut() {
            let (method, args, ret) = if let WorkState::Waiting(method, args, ret) = state { (method, args, ret) } else { continue; };

            let method_data = if let Some(m) = self.blocks.get(&method) { m.clone() } else { continue; };
            let mut arg_data = vec![];

            for arg in args.iter() {
                if let Some(arg) = self.blocks.get(&arg) { arg_data.push(arg.clone()) } else { continue 'outer; }
            }

            println!("{:?} is unblocked, starting", id);
            let id = id.to_string();

            let arg_vals = arg_data.iter().map(|v| v.as_ref()).collect::<Vec<_>>();
            let res = executor::exec(method_data.as_ref(), &arg_vals).map_err(|e| anyhow::Error::msg(format!("{:?}", e)));
            let res = res.unwrap();
            let block = crate::block::create_block(res);
            self.workswap.finish_work(id, Ok(block.cid.clone()));
            //self.bitswap.send_block(peer, block.cid, block.data)

            //sender.send(NetworkEvent::FinishedLocalExec(id, block)).unwrap();
        }
    }
    pub fn want_exec(&mut self, id: ExecutionID, method: Cid, args: Vec<Cid>, ret: WorkReturner) {
        // TODO: Doing string-hash compare here, fix this
        let query = self.kad.get_closest_peers(id.as_bytes());
        self.queries.insert(query, QueryType::Workswap(id.clone(), method.clone(), args.clone()));
        self.local_works.insert(id, WorkState::Waiting(method, args, ret));
    }
    pub fn poll_custom(&mut self, ctx: &mut Context) -> Poll<NetworkBehaviourAction<(), NetworkEvent>> {
        while let Poll::Ready(Some(event)) = self.events.poll_next_unpin(ctx) {
            return Poll::Ready(NetworkBehaviourAction::GenerateEvent(event));
        }
        Poll::Pending
    }
}

pub async fn run(listen_on: Vec<Multiaddr>, bootstrap: Vec<Multiaddr>, mut control: UnboundedReceiver<crate::IPCSCommand>) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer id: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex and Yamux protocols
    let transport = libp2p::build_development_transport(local_key.clone()).unwrap();

    let mut swarm = {
        let kad_cfg = KademliaConfig::default();
        //kad_cfg.set_protocol_name(&b"/ipfs/kad/1.0.0"[..]);
        let kad = Kademlia::with_config(local_peer_id.clone(), MemoryStore::new(local_peer_id.clone()), kad_cfg);

        let (tx, rx) = unbounded();
        let behavior = Behaviour {
            local_works: HashMap::new(),
            queries: HashMap::new(),
            blocks: HashMap::new(),

            events_sender: tx,
            events: rx,

            mdns: TokioMdns::new().unwrap(),
            kad,
            ping: Ping::new(PingConfig::new()),
            identify: Identify::new("/ipcs/0.0.0".into(), "rust-ipcs/0.0.0".into(), local_key.clone().public()),
            workswap: workswap::Workswap::new(),
            bitswap: Bitswap::new(),
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
    for l in listen_on {
        Swarm::listen_on(&mut swarm, l).unwrap();
    }


    for a in bootstrap {
        Swarm::dial_addr(&mut swarm, a).unwrap();
    }


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
        }).await;
        panic!("Closed");
    }
}
