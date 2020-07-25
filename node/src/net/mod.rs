use libp2p::{
    Multiaddr, PeerId, NetworkBehaviour, identity, gossipsub::{Topic},
    mdns::{Mdns, MdnsEvent},
    ping::{Ping, PingConfig, PingEvent, PingSuccess},
    identify::{Identify, IdentifyEvent, IdentifyInfo},
    request_response::{codec, RequestResponse, RequestResponseEvent},
    swarm::{SwarmBuilder, NetworkBehaviourEventProcess, NetworkBehaviour},
    Swarm,
};
use crate::net::workswap::{WorkswapEvent, ExecutionResult};
use ipfsapi::IpfsApi;
use std::sync::Arc;
use futures::{stream::{Stream, StreamExt}, channel::mpsc::UnboundedReceiver, Future};
use futures::SinkExt;
use futures::future::Either;
use std::task::Poll;
use tokio::macros::support::Pin;

mod workswap;

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    #[behaviour(ignore)]
    ipfs: Arc<IpfsApi>,

    mdns: Mdns,
    ping: Ping,
    identify: Identify,
    workswap: workswap::Workswap,
}

impl NetworkBehaviourEventProcess<()> for Behaviour {
    fn inject_event(&mut self, event: ()) {}
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
                if let Some(ret) = self.workswap.return_channels.remove(&id) {
                    tokio::spawn(async move { ret.send(Ok(res)).unwrap(); });
                }
            }
            WorkswapEvent::WantCalc(peer, id, method, arg) => {
                self.workswap.accept(&peer, id.clone());
                let ipfs = self.ipfs.clone();
                let mut fin = self.workswap.queued_local_execs.clone();
                tokio::spawn(async move {
                    let args = arg.iter().map(AsRef::as_ref).collect::<Vec<_>>();
                    let res = crate::exec(&ipfs, &method, &args).await.unwrap();
                    let res = ExecutionResult {
                        id,
                        hash: res,
                    };
                    fin.send((peer, res)).await.unwrap();
                });
            }
            WorkswapEvent::Accepted(peer, id) => {
                panic!("Accepted")
            }
            WorkswapEvent::Rejected(peer, id) => {
                panic!("Rejected")
            }
        }
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for Behaviour {
    fn inject_event(&mut self, event: PingEvent) {
        use libp2p::ping::handler::{PingFailure, PingSuccess};
        match event {
            PingEvent {
                peer,
                result: Result::Ok(PingSuccess::Ping { rtt }),
            } => {
                log::info!(
                    "ping: rtt to {} is {} ms",
                    peer.to_base58(),
                    rtt.as_millis()
                );
                self.set_rtt(&peer, rtt);
            }
            PingEvent {
                peer,
                result: Result::Ok(PingSuccess::Pong),
            } => {
                log::info!("ping: pong from {}", peer);
            }
            PingEvent {
                peer,
                result: Result::Err(PingFailure::Timeout),
            } => {
                log::info!("ping: timeout to {}", peer);
                self.remove_peer(&peer);
            }
            PingEvent {
                peer,
                result: Result::Err(PingFailure::Other { error }),
            } => {
                log::error!("ping: failure with {}: {}", peer.to_base58(), error);
            }
        }
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for Behaviour {
    fn inject_event(&mut self, event: MdnsEvent) {
        match event {
            MdnsEvent::Discovered(list) => {
                for (peer, addr) in list {
                    log::info!("mdns: Discovered peer {} on {:?}", peer.to_base58(), &addr);
                    self.add_peer(peer, addr);
                }
            }
            MdnsEvent::Expired(list) => {
                for (peer, _) in list {
                    log::info!("mdns: Expired peer {}", peer.to_base58());
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
    let transport = libp2p::build_development_transport(local_key
        .clone()).unwrap();

    let mut swarm = {
        let behavior = Behaviour {
            ipfs,
            mdns: Mdns::new().unwrap(),
            ping: Ping::new(PingConfig::new()),
            identify: Identify::new(
                "/ipcs/0.0.0".into(),
                "rust-ipcs".into(),
                local_key.clone().public()),
            workswap: workswap::Workswap::new(),
        };
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
    use futures::StreamExt;

    loop {
        futures::future::poll_fn(|ctx| -> Poll<Option<()>>{
            match swarm.poll_next_unpin(ctx) {
                Poll::Ready(Some(item)) => {
                    log::info!("Event: {:?}", item);
                }
                Poll::Ready(None) => {
                    panic!("Swarm closed")
                }
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
        }).await;
    }
}