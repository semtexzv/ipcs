use std::sync::Arc;
use std::task::Poll;

use futures::{channel::mpsc::UnboundedReceiver, future::poll_fn, Future, SinkExt, StreamExt};

use libp2p::{
    identify::{Identify, IdentifyEvent},
    identity,
    mdns::{Mdns, MdnsEvent},
    ping::{Ping, PingConfig, PingEvent},
    swarm::{NetworkBehaviourEventProcess, SwarmBuilder},
    Multiaddr, NetworkBehaviour, PeerId, Swarm,
};
use tokio::macros::support::Pin;

use ipfsapi::IpfsApi;

mod workswap;

use workswap::{ExecutionResult, Workswap, WorkswapEvent};

/// Network behavior describing general properties of this node
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    #[behaviour(ignore)]
    ipfs: Arc<IpfsApi>,
    /// Used to find peers on local network
    mdns: Mdns,
    ping: Ping,
    identify: Identify,
    /// Behavior responsible for actually sending work to other nodes
    workswap: Workswap,
}

impl NetworkBehaviourEventProcess<()> for Behaviour {
    fn inject_event(&mut self, _: ()) {}
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for Behaviour {
    fn inject_event(&mut self, event: IdentifyEvent) {
        log::info!("identify: {:?}", event);
    }
}

impl NetworkBehaviourEventProcess<workswap::WorkswapEvent> for Behaviour {
    fn inject_event(&mut self, event: WorkswapEvent) {
        log::info!("workswap event: {:?}", event);
        match event {
            WorkswapEvent::Completed(peer, id, res) => {
                log::debug!("Peer {} finished work on {} res: {}", peer, id, res);
                if let Some(ret) = self.workswap.return_channels.remove(&id) {
                    tokio::spawn(async move {
                        ret.send(Ok(res)).unwrap();
                    });
                }
            }
            WorkswapEvent::WantCalc(peer, id, method, args) => {
                log::debug!("peer {} wants us to work on {} ({}) applied on {:?}", peer, id, method, args);
                self.workswap.accept(&peer, id.clone());
                let ipfs = self.ipfs.clone();
                let mut fin = self.workswap.queued_local_execs.clone();
                tokio::spawn(async move {
                    let args = args.iter().map(AsRef::as_ref).collect::<Vec<_>>();
                    let res = crate::exec(&ipfs, &method, &args).await.unwrap();
                    let res = ExecutionResult { id, hash: res };
                    fin.send((peer, res)).await.unwrap();
                });
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

impl NetworkBehaviourEventProcess<PingEvent> for Behaviour {
    fn inject_event(&mut self, event: PingEvent) {
        use libp2p::ping::handler::{PingFailure, PingSuccess};
        let peer = event.peer;
        match event.result {
            Ok(PingSuccess::Ping { rtt }) => {
                log::trace!("ping: rtt to {} is {} ms", peer.to_base58(), rtt.as_millis());
                self.set_rtt(&peer, rtt);
            }
            Ok(PingSuccess::Pong) => {
                log::trace!("ping: pong from {}", peer);
            }
            Err(PingFailure::Timeout) => {
                log::trace!("ping: timeout to {}", peer);
                self.remove_peer(&peer);
            }
            Err(PingFailure::Other { error }) => {
                log::trace!("ping: failure with {}: {}", peer.to_base58(), error);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for Behaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(list) => {
                for (peer, addr) in list {
                    log::trace!("mdns: Discovered peer {} on {:?}", peer.to_base58(), &addr);
                    self.add_peer(peer, addr);
                }
            }
            MdnsEvent::Expired(list) => {
                for (peer, _) in list {
                    log::debug!("mdns: Expired peer {}", peer.to_base58());
                    self.remove_peer(&peer);
                }
            }
        }
    }
}

impl Behaviour {
    pub fn add_peer(&mut self, peer: PeerId, _addr: Multiaddr) {
        self.workswap.connect(peer);
        //self.kademlia.add_address(&peer, addr);
        // TODO self.bitswap.add_node_to_partial_view(peer);
    }

    pub fn remove_peer(&mut self, _peer: &PeerId) {
        // TODO self.bitswap.remove_peer(&peer);
    }

    pub fn set_rtt(&mut self, _peer: &PeerId, _rtt: std::time::Duration) {}
}

pub async fn run(ipfs: Arc<IpfsApi>, mut control: UnboundedReceiver<crate::IPCSCommand>) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer id: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex and Yamux protocols
    let transport = libp2p::build_development_transport(local_key.clone()).unwrap();

    let mut swarm = {
        let behavior = Behaviour {
            ipfs,
            mdns: Mdns::new().unwrap(),
            ping: Ping::new(PingConfig::new()),
            identify: Identify::new("/ipcs/0.0.0".into(), "rust-ipcs".into(), local_key.clone().public()),
            workswap: workswap::Workswap::new(),
        };
        struct Exec;
        impl libp2p::core::Executor for Exec {
            fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
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
                    swarm.workswap.handle_control_mesage(item);
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
