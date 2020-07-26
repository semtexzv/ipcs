use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::TryFrom;
use std::pin::Pin;

use futures::channel::{
    mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    oneshot::Sender as OneSender,
};
use futures::task::{Context, Poll};
use futures::{AsyncRead, AsyncWrite, Future, FutureExt, StreamExt};
use libp2p::{
    core::{connection::ConnectionId, upgrade, InboundUpgrade, Multiaddr, OutboundUpgrade, PeerId, UpgradeInfo},
    swarm::{NetworkBehaviour, NetworkBehaviourAction, OneShotHandler, NotifyHandler, PollParameters, DialPeerCondition},
};
use multihash::MultihashDigest;
use std::cmp;

type FutureResult<T, E> = Pin<Box<dyn Future<Output=Result<T, E>> + Send>>;

type Error = anyhow::Error;

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Message {
    // Persistent fiel
    running: Vec<ExecutionID>,

    // Transient fields
    accepts: Vec<ExecutionID>,
    rejects: Vec<ExecutionID>,
    requests: Vec<ExecutionRequest>,
    results: Vec<ExecutionResult>,
}

impl Into<Vec<u8>> for &Message {
    fn into(self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}

impl TryFrom<&[u8]> for Message {
    type Error = Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(serde_json::from_slice(value).unwrap())
    }
}

impl From<()> for Message {
    fn from(_: ()) -> Self {
        Default::default()
    }
}

impl Message {
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.results.is_empty() && self.rejects.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkswapConfig;

impl UpgradeInfo for WorkswapConfig {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(b"/ipcs/workswap/0.0.0")
    }
}

impl UpgradeInfo for Message {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(b"/ipcs/workswap/0.0.0")
    }
}

impl<TS> InboundUpgrade<TS> for WorkswapConfig
    where
        TS: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Message;
    type Error = Error;
    type Future = FutureResult<Self::Output, Self::Error>;

    fn upgrade_inbound(self, mut socket: TS, _info: Self::Info) -> Self::Future {
        async move {
            // TODO: proper error types
            let packet = upgrade::read_one(&mut socket, 1024 * 1024).await?;
            let message = Message::try_from(packet.as_ref())?;
            Ok(message)
        }.boxed()
    }
}

impl<TS> OutboundUpgrade<TS> for Message
    where
        TS: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = ();
    type Error = std::io::Error;
    type Future = FutureResult<Self::Output, Self::Error>;

    fn upgrade_outbound(self, mut socket: TS, _info: Self::Info) -> Self::Future {
        async move {
            let bytes: Vec<u8> = (&self).into();
            upgrade::write_one(&mut socket, bytes).await?;
            Ok(())
        }.boxed()
    }
}

#[derive(Default)]
pub struct PeerInfo {
    /// Which work units is peer running
    running_there: HashSet<ExecutionID>,
    /// Which work units are we running for peer
    running_here: HashSet<ExecutionID>,

    // Used here to accumulate state changes, and then send them to peer
    message: Message,
}

impl PeerInfo {
    /// Accept the work unit
    pub fn accept(&mut self, _id: ExecutionID) {
        self.message.accepts.push(_id);
    }
    /// Reject the work unit
    pub fn reject(&mut self, id: ExecutionID) {
        self.message.rejects.push(id);
    }

    /// Request a the peer to perform a work unit
    pub fn add_request(&mut self, req: ExecutionRequest) {
        self.running_there.insert(req.id.clone());
        self.message.requests.push(req)
    }

    /// Send result of local execution to peer
    pub fn add_result(&mut self, execution: ExecutionResult) {
        self.running_there.remove(&execution.id);
        self.message.results.push(execution);
    }

    /// Do we have new information we need to send to this peer ?
    pub fn ready_to_send(&mut self) -> Option<Message> {
        if self.message.is_empty() {
            return None;
        }

        self.message.running = self.running_here.clone().into_iter().collect();
        Some(std::mem::take(&mut self.message))
    }
}

pub type WorkResult = Result<Cid, String>;

/// Struct containing metdata about work that this node has dispatched to the network
pub struct WorkInfo {
    /// When was this work unit started ?
    pub started: std::time::Instant,
    /// Set of peers to which we have sent the work
    pub contacted_peers: HashSet<PeerId>,
    /// Set of peers which have rejected the work
    pub rejected_peers: HashSet<PeerId>,
    /// Peers which have produced a value for us
    pub resolved_peers: HashMap<PeerId, WorkResult>,
}

impl WorkInfo {
    /// Do we have enough information to provide a satisfying result ?
    pub fn is_resolved(&self) -> bool {
        // TODO: Beter heuristic here
        self.resolved_peers.len() > 0
    }

    /// Returns most probable result of this work, if it is resolved
    pub fn resolve_result(self) -> Option<WorkResult> {
        if self.is_resolved() {
            return self.resolved_peers.into_iter().next().map(|v| v.1);
        }
        None
    }

    /// Work unit is stuck, because all the peers rejected it
    pub fn is_stuck(&self) -> bool {
        self.rejected_peers.len() >= self.contacted_peers.len()
    }
}

pub struct Workswap {
    // Allows us to dispatch event from a lot of places
    events: VecDeque<NetworkBehaviourAction<Message, WorkswapEvent>>,
    // Information about connected peers
    pub peers: HashMap<PeerId, PeerInfo>,
    // Information about dispatched work units
    pub works: HashMap<ExecutionID, WorkInfo>,

    // Used here as a queue to send work results from worker tasks to protocol implementation
    pub queued_works: UnboundedSender<(PeerId, ExecutionResult)>,
    pub finished_works: UnboundedReceiver<(PeerId, ExecutionResult)>,
}

impl Workswap {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();

        Self {
            peers: HashMap::new(),
            events: VecDeque::new(),

            works: HashMap::new(),

            queued_works: tx,
            finished_works: rx,
        }
    }

    pub fn connect(&mut self, _peer_id: PeerId) {
        // TODO: Move to separate struct for controlling the swarm
        if !self.peers.contains_key(&_peer_id) {
            let ev = NetworkBehaviourAction::DialPeer {
                peer_id: _peer_id,
                condition: DialPeerCondition::Disconnected,
            };
            self.events.push_back(ev);
        }
    }

    fn finish_work(&mut self, peer_id: &PeerId, exec: ExecutionResult) {
        let stats = self.peers.get_mut(peer_id).expect("Peer not found");
        stats.add_result(exec);
    }


    pub fn reject(&mut self, peer_id: &PeerId, id: ExecutionID) {
        self.peers.get_mut(peer_id).expect("Peer not found").reject(id);
    }

    /// Accept a work request from peer
    pub fn accept(&mut self, peer_id: &PeerId, id: ExecutionID) {
        self.peers.get_mut(peer_id).expect("Peer not found").accept(id);
    }

    /// Request execution of work from a reasonable set of peers
    pub fn want_exec(&mut self, id: ExecutionID, method: Cid, args: Vec<Cid>) {
        // TODO: Disable routing here, route using kademlia

        // TODO: Some proper search structure here
        // Some randomness factor. find by hamming distance between work hash and node id
        let mut peers = self.peers.iter_mut().collect::<Vec<_>>();


        peers.sort_by_key(|(f, _)| hamming::distance(&f.as_ref()[..8], &id.as_bytes()[..8]));

        if peers.len() == 0 {
            self.events.push_back(NetworkBehaviourAction::GenerateEvent(WorkswapEvent::LocalErr(id.clone(), "no_peers".to_string())))
        }

        let req = ExecutionRequest {
            // TODO: Adopt multihashes
            id: id.clone(),
            method: method.clone(),
            args: args.clone(),
        };

        let mut work_unit = WorkInfo {
            started: std::time::Instant::now(),
            contacted_peers: HashSet::new(),
            rejected_peers: HashSet::new(),
            resolved_peers: HashMap::new(),
        };

        let max_peers = cmp::min(3, peers.len());
        for (peer, stats) in &mut peers[0..max_peers] {
            stats.add_request(req.clone());
            work_unit.contacted_peers.insert(peer.clone());
            log::info!("Sending {} to {}", id, peer);
        }
        self.works.insert(id.clone(), work_unit);
    }
}

pub type ExecutionID = String;
pub type Cid = String;

#[derive(Debug)]
pub enum WorkswapEvent {
    WantCalc(PeerId, ExecutionID, Cid, Vec<Cid>),

    Accepted(PeerId, ExecutionID),
    Rejected(PeerId, ExecutionID),

    Resolved(ExecutionID, Result<Cid, String>),
    LocalErr(ExecutionID, String),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionRequest {
    pub id: ExecutionID,
    pub method: Cid,
    pub args: Vec<Cid>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionResult {
    pub id: ExecutionID,
    pub result: Result<Cid, String>,
}

impl NetworkBehaviour for Workswap {
    type ProtocolsHandler = OneShotHandler<WorkswapConfig, Message, Message>;
    type OutEvent = WorkswapEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        Default::default()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        log::info!("workswap: connected {:?}", peer_id);
        self.peers.insert(peer_id.clone(), PeerInfo::default());
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        log::info!("Workswap: disconnected {:?}", peer_id);
        self.peers.remove(peer_id);
    }

    // TODO: Protocol internal handling of multiple workers for single work unit & result heuristics
    fn inject_event(&mut self, peer_id: PeerId, _connection: ConnectionId, message: Message) {
        log::info!("message: {:?}", message);
        let stats = self.peers.get_mut(&peer_id).expect("Peer not found");


        for id in message.rejects {
            log::info!("Peer {:?} rejected execution {:?}", &peer_id, id);
            stats.running_there.remove(&id);
            let work = self.works.get_mut(&id);

            if let Some(work) = work {
                work.rejected_peers.insert(peer_id.clone());
            }

            self.events
                .push_back(NetworkBehaviourAction::GenerateEvent(WorkswapEvent::Rejected(peer_id.clone(), id)));
        }

        for req in message.requests {
            log::info!("Peer {:?} requested execution of {:#?}", &peer_id, req);
            stats.running_here.insert(req.id.clone());
            let ev = WorkswapEvent::WantCalc(peer_id.clone(), req.id, req.method, req.args);
            self.events.push_back(NetworkBehaviourAction::GenerateEvent(ev));
        }

        for received in message.results {
            log::info!("Peer {:?} finished execution {:?}", peer_id, received);

            stats.running_there.remove(&received.id);
            use std::collections::hash_map::Entry;
            if let Entry::Occupied(mut entry) = self.works.entry(received.id.clone()) {
                let work = entry.get_mut();
                work.resolved_peers.insert(peer_id.clone(), received.result);

                log::info!("Work: {} has {} / {} results ({}) rejects", received.id,
                    work.resolved_peers.len(),
                    work.contacted_peers.len(), work.rejected_peers.len());

                if !work.is_resolved() {
                    continue;
                }

                log::info!("Resolving work {}", received.id);
                let (k, work) = entry.remove_entry();
                let event = WorkswapEvent::Resolved(k, work.resolve_result().unwrap());
                self.events.push_back(NetworkBehaviourAction::GenerateEvent(event));
            }
        }

        let reported = message.running.into_iter().collect();
        let diff = stats.running_there.difference(&reported);

        for id in diff {
            log::info!("Peer {:?} expected to run {:?} but not reported", peer_id, id);
        }
    }

    fn poll(&mut self, ctx: &mut Context, _: &mut impl PollParameters) -> Poll<NetworkBehaviourAction<Message, Self::OutEvent>> {
        while let Poll::Ready(Some((peer, exec))) = self.finished_works.poll_next_unpin(ctx) {
            log::info!("send exec res: {:?} {:?}", peer, exec);
            self.finish_work(&peer, exec);
        }

        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        for (peer, stats) in self.peers.iter_mut() {
            if let Some(msg) = stats.ready_to_send() {
                return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                    peer_id: peer.clone(),
                    handler: NotifyHandler::Any.clone(),
                    event: msg,
                });
            }
        }

        Poll::Pending
    }
}
