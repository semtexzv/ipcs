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
    core::{
        connection::ConnectionId, upgrade, InboundUpgrade, Multiaddr, OutboundUpgrade, PeerId,
        UpgradeInfo,
    },
    swarm::{
        DialPeerCondition, NetworkBehaviour, NetworkBehaviourAction, NotifyHandler, PollParameters,
    },
};
use multihash::MultihashDigest;

type FutureResult<T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send>>;

type Error = Box<dyn std::error::Error + Send + 'static>;

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Message {
    // Persistent field,
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
            let packet = upgrade::read_one(&mut socket, 1024 * 1024).await.unwrap();
            let message = Message::try_from(packet.as_ref())?;
            Ok(message)
        }
        .boxed()
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
        }
        .boxed()
    }
}

#[derive(Default)]
pub struct PeerStats {
    running_there: HashSet<ExecutionID>,
    running_here: HashSet<ExecutionID>,

    message: Message,
}

impl PeerStats {
    pub fn accept(&mut self, _id: ExecutionID) {
        self.message.accepts.push(_id);
    }
    pub fn reject(&mut self, id: ExecutionID) {
        self.message.rejects.push(id);
    }

    pub fn add_request(&mut self, req: ExecutionRequest) {
        self.running_there.insert(req.id.clone());
        self.message.requests.push(req)
    }

    pub fn add_result(&mut self, execution: ExecutionResult) {
        self.running_there.remove(&execution.id);
        self.message.results.push(execution);
    }

    pub fn ready_to_send(&mut self) -> Option<Message> {
        if self.message.is_empty() {
            return None;
        }

        self.message.running = self.running_here.clone().into_iter().collect();
        Some(std::mem::take(&mut self.message))
    }
}

pub struct Workswap {
    events: VecDeque<NetworkBehaviourAction<Message, WorkswapEvent>>,
    pub peers: HashMap<PeerId, PeerStats>,

    pub return_channels: HashMap<ExecutionID, OneSender<Result<String, String>>>,

    pub queued_local_execs: UnboundedSender<(PeerId, ExecutionResult)>,
    pub finished_local_execs: UnboundedReceiver<(PeerId, ExecutionResult)>,
}

impl Workswap {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();

        Self {
            peers: HashMap::new(),
            events: VecDeque::new(),

            return_channels: HashMap::new(),

            queued_local_execs: tx,
            finished_local_execs: rx,
        }
    }

    pub fn connect(&mut self, peer_id: PeerId) {
        if !self.peers.contains_key(&peer_id) {
            let ev = NetworkBehaviourAction::DialPeer {
                peer_id,
                condition: DialPeerCondition::Disconnected,
            };
            self.events.push_back(ev);
        }
    }

    fn resolve_execution(&mut self, peer_id: &PeerId, exec: ExecutionResult) {
        let stats = self.peers.get_mut(peer_id).expect("Peer not found");
        stats.add_result(exec);
    }

    pub fn reject(&mut self, peer_id: &PeerId, id: ExecutionID) {
        self.peers
            .get_mut(peer_id)
            .expect("Peer not found")
            .reject(id);
    }

    pub fn accept(&mut self, peer_id: &PeerId, id: ExecutionID) {
        self.peers
            .get_mut(peer_id)
            .expect("Peer not found")
            .accept(id);
    }

    /// We want to execute a function, and pick first usable peer.
    pub fn want_exec(
        &mut self,
        method: Cid,
        args: Vec<Cid>,
        ret: OneSender<Result<String, String>>,
    ) {
        let mut id = multihash::Sha2_256::default();
        id.input(method.as_bytes());
        for i in &args {
            id.input(i.as_bytes());
        }
        let id = bs58::encode(&id.digest(&[])).into_string();

        // TODO: Some proper search structure here
        // Some randomness factor. find by hamming distance between work hash and node id
        let mut peers = self.peers.iter_mut().collect::<Vec<_>>();

        peers.sort_by_key(|(f, _)| hamming::distance(f.as_ref(), id.as_ref()));

        if peers.len() == 0 {
            return ret.send(Err("No peers".to_string())).unwrap();
        }
        let (peer, stats) = &mut peers[0];
        log::info!("Sending {} to {}", method, peer);
        // TODO: Use proper multihash, not hashing of strings here

        let req = ExecutionRequest {
            // TODO: Adopt multihashes
            id: id.clone(),
            method: method,
            args: args,
        };
        stats.add_request(req);
        self.return_channels.insert(id, ret);
    }

    pub fn handle_control_mesage(&mut self, msg: crate::IPCSCommand) {
        match msg {
            crate::IPCSCommand::Exec(method, args, ret) => self.want_exec(method, args, ret),
        }
    }
}

type ExecutionID = String;
type Cid = String;

#[derive(Debug)]
pub enum WorkswapEvent {
    WantCalc(PeerId, ExecutionID, Cid, Vec<Cid>),
    Accepted(PeerId, ExecutionID),
    Rejected(PeerId, ExecutionID),
    Completed(PeerId, ExecutionID, Cid),
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
    pub hash: Cid,
}

impl NetworkBehaviour for Workswap {
    type ProtocolsHandler = libp2p::swarm::OneShotHandler<WorkswapConfig, Message, Message>;
    type OutEvent = WorkswapEvent;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        Default::default()
    }

    fn addresses_of_peer(&mut self, _peer_id: &PeerId) -> Vec<Multiaddr> {
        Vec::new()
    }

    fn inject_connected(&mut self, peer_id: &PeerId) {
        log::info!("workswap: inject connected {:?}", peer_id);
        self.peers.insert(peer_id.clone(), PeerStats::default());
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId) {
        log::info!("Workswap: disconnected {:?}", peer_id);
        self.peers.remove(peer_id);
    }

    fn inject_event(&mut self, peer_id: PeerId, _connection: ConnectionId, message: Message) {
        log::info!("message: {:?}", message);
        let stats = self.peers.get_mut(&peer_id).expect("Peer not found");

        for id in message.rejects {
            log::info!("Peer {:?} rejected execution {:?}", &peer_id, id);
            stats.running_there.remove(&id);
            self.events.push_back(NetworkBehaviourAction::GenerateEvent(
                WorkswapEvent::Rejected(peer_id.clone(), id),
            ));
        }

        for req in message.requests {
            log::info!("Peer {:?} requested execution of {:#?}", &peer_id, req);
            stats.running_here.insert(req.id.clone());
            let ev = WorkswapEvent::WantCalc(peer_id.clone(), req.id, req.method, req.args);
            self.events
                .push_back(NetworkBehaviourAction::GenerateEvent(ev));
        }

        for res in message.results {
            log::info!("Peer {:?} finished execution {:?}", peer_id, res);
            stats.running_there.remove(&res.id);
            let ev = WorkswapEvent::Completed(peer_id.clone(), res.id, res.hash);
            self.events
                .push_back(NetworkBehaviourAction::GenerateEvent(ev));
        }

        let reported = message.running.into_iter().collect();
        let diff = stats.running_there.difference(&reported);

        for id in diff {
            log::info!(
                "Peer {:?} expected to run {:?} but not reported",
                peer_id,
                id
            );
        }
    }

    fn poll(
        &mut self,
        ctx: &mut Context,
        _: &mut impl PollParameters,
    ) -> Poll<NetworkBehaviourAction<Message, Self::OutEvent>> {
        while let Poll::Ready(Some((peer, exec))) = self.finished_local_execs.poll_next_unpin(ctx) {
            log::info!("send exec res: {:?} {:?}", peer, exec);
            self.resolve_execution(&peer, exec);
        }

        if let Some(event) = self.events.pop_front() {
            log::info!("Event pop: {:?}", event);
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
