use libp2p::{
    Multiaddr, PeerId, NetworkBehaviour, identity, gossipsub::{Topic},
    mdns::{Mdns, MdnsEvent},
    ping::{Ping, PingConfig, PingEvent, PingSuccess},
    identify::{Identify, IdentifyEvent, IdentifyInfo},
    request_response::{codec, RequestResponse, RequestResponseEvent},
    swarm::{NetworkBehaviourEventProcess, NetworkBehaviour},
    Swarm,
};

#[derive(NetworkBehaviour)]
pub struct Behaviour {
    mdns: Mdns,
    ping: Ping,
    identify: Identify,
}

impl NetworkBehaviourEventProcess<()> for Behaviour {
    fn inject_event(&mut self, event: ()) {}
}

impl NetworkBehaviourEventProcess<IdentifyEvent> for Behaviour {
    fn inject_event(&mut self, event: IdentifyEvent) {
        log::info!("identify: {:?}", event);
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
                    log::info!("mdns: Discovered peer {}", peer.to_base58());
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
    pub fn add_peer(&mut self, peer: PeerId, addr: Multiaddr) {
        self.identify.inject_connected(&peer);
        //self.kademlia.add_address(&peer, addr);
        // TODO self.bitswap.add_node_to_partial_view(peer);
    }

    pub fn remove_peer(&mut self, peer: &PeerId) {
        // TODO self.bitswap.remove_peer(&peer);
    }
    pub fn set_rtt(&mut self, peer: &PeerId, rtt: std::time::Duration) {}
}

pub async fn run() {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    log::info!("Local peer id: {:?}", local_peer_id);

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex and Yamux protocols
    let transport = libp2p::build_development_transport(local_key
        .clone()).unwrap();

    // Create a Gossipsub topic
    let topic = Topic::new("ipcs".into());

    let mut swarm = {
        let behavior = Behaviour {
            mdns: Mdns::new().unwrap(),
            ping: Ping::new(PingConfig::new()),
            identify: Identify::new(
                "/ipcs/0.0.0".into(),
                "rust-ipcs".into(),
                local_key.clone().public()),
        };
        Swarm::new(transport, behavior, local_peer_id)
    };

    Swarm::listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0".parse().unwrap()).unwrap();
    loop {
        let ev = swarm.next_event().await;
        println!("Event: {:?}", ev);
    }
}