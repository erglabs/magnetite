use futures::StreamExt;
use libp2p::{
    Multiaddr,
    NetworkBehaviour,
    PeerId,
    Transport,
    core::upgrade,
    identity,
    floodsub::{self, Floodsub, FloodsubEvent},
    mdns::{Mdns, MdnsEvent},
    mplex,
    noise,
    swarm::{NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent},
    // `TokioTcpConfig` is available through the `tcp-tokio` feature.
    tcp::TokioTcpConfig,
};

pub mod probe;
use probe::*;

use async_trait::async_trait;
use futures::{channel::mpsc, executor::LocalPool, prelude::*, task::SpawnExt, AsyncWriteExt};
use rand::{self, Rng};
use std::io;
use std::iter;
use libp2p::{
    core::{
        Multiaddr,
        PeerId,
        identity,
        muxing::StreamMuxerBox,
        transport::{self, Transport},
        upgrade,
    },
    yamux::{YamuxConfig,self},
    tcp::{TcpConfig,self},
    swarm::{Swarm, SwarmEvent},
    noise::{NoiseConfig, X25519Spec, Keypair},
};


use std::error::Error;
use tokio::io::{self, AsyncBufReadExt};

/// The `tokio::main` attribute sets up a tokio runtime.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    // ======================
    // start req-resp config
    let query = ProbeQuery("query".to_string().into_bytes());
    let response = ProbeResponse("response".to_string().into_bytes());

    let protocols = iter::once((ProbeProtocol(), ProtocolSupport::Full));
    let cfg = RequestResponseConfig::default();

    let probeBehavior = RequestResponse::new(ProbeCodec(), protocols.clone(), cfg.clone());
    // ======================
    // end reqresp config

    // ======================
    // start id config
    // Create a random PeerId
    let id_keys = identity::Keypair::generate_ed25519();
    let peer_id = PeerId::from(id_keys.public());
    println!("Local peer id: {:?}", peer_id);

    // Create a keypair for authenticated encryption of the transport.
    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&id_keys)
        .expect("Signing libp2p-noise static DH keypair failed.");

    // Create a tokio-based TCP transport use noise for authenticated
    // encryption and Mplex for multiplexing of substreams on a TCP stream.
    let transport = TokioTcpConfig::new().nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();
    // end id config
    // ======================


    // ======================
    // begin floodsub impl

    // Create a behavior topic
    let floodsub_topic = floodsub::Topic::new("general");

    #[derive(NetworkBehaviour)]
    struct MagnetitedBehavior {
        floodsub: Floodsub,
        mdns: Mdns,
        probe: probeBehavior,
    }

    impl NetworkBehaviourEventProcess<FloodsubEvent> for MagnetitedBehavior {
        // Called when `floodsub` produces an event.
        fn inject_event(&mut self, message: FloodsubEvent) {
            if let FloodsubEvent::Message(message) = message {
                println!("Received: '{:?}' from {:?}", String::from_utf8_lossy(&message.data), message.source);
                println!("sending probe to target");
                self.probe.send_request(&message.source, "checking in".to_owned());
            }
        }
    }

    impl NetworkBehaviourEventProcess<MdnsEvent> for MagnetitedBehavior {
        // Called when `mdns` produces an event.
        fn inject_event(&mut self, event: MdnsEvent) {
            match event {
                MdnsEvent::Discovered(list) =>
                    for (peer, _) in list {
                        self.floodsub.add_node_to_partial_view(peer);
                        println!("sending hello probe to target");
                        self.probe.send_request(&peer, "hello peer".to_owned());
                    }
                MdnsEvent::Expired(list) =>
                    for (peer, _) in list {
                        if !self.mdns.has_node(&peer) {
                            self.floodsub.remove_node_from_partial_view(&peer);
                        }
                    }
            }
        }
    }

    // only response part ? **(comment)
    impl NetworkBehaviourEventProcess<RequestResponseEvent> for MagnetitedBehavior {
        // Called when `probe` produces an event.
        fn inject_event(&mut self, event: RequestResponseEvent) {
            match event {
                SwarmEvent::Behaviour(RequestResponseEvent::Message {
                    peer,
                    message: RequestResponseMessage::Request { request, channel, .. }
                }) => {
                    println!("rr::request::got");
                    self.probe.send_response(channel, response.clone()).unwrap();
                },

                // ** from above
                // SwarmEvent::Behaviour(RequestResponseEvent::Message {
                //     peer,
                //     message: RequestResponseMessage::Response { request_id, response }
                // }) => {
                //     println!("rr::request::id::{}", request_id);{
                //         req_id = self.probe.send_request(&peer1_id, query.clone());
                //     }
                // }
            
                SwarmEvent::Behaviour(RequestResponseEvent::ResponseSent {
                    peer, ..
                }) => {
                    println!("rr::response::sent");
                }
            
                SwarmEvent::Behaviour(e) =>panic!("rr: Unexpected event: {:?}", e),
                _ => {}
            }
        }
    }
    // end behavior inpl
    // ======================

    // Create a Swarm to manage peers and events.
    let mut swarm = {
        let mdns = Mdns::new(Default::default()).await?;
        let mut behaviour = MagnetitedBehavior {
            floodsub: Floodsub::new(peer_id.clone()),
            mdns,
        };

        behaviour.floodsub.subscribe(floodsub_topic.clone());

        SwarmBuilder::new(transport, behaviour, peer_id)
            // We want the connection background tasks to be spawned
            // onto the tokio runtime.
            .executor(Box::new(|fut| { tokio::spawn(fut); }))
            .build()
    };

    // Reach out to another node if specified
    if let Some(to_dial) = std::env::args().nth(1) {
        let addr: Multiaddr = to_dial.parse()?;
        swarm.dial_addr(addr)?;
        println!("Dialed {:?}", to_dial)
    }

    // Read full lines from stdin
    let mut stdin = io::BufReader::new(io::stdin()).lines();

    // Listen on all interfaces and whatever port the OS assigns
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Kick it off
    loop {
        tokio::select! {
            line = stdin.next_line() => {
                let line = line?.expect("stdin closed");
                swarm.behaviour_mut().floodsub.publish(floodsub_topic.clone(), line.as_bytes());
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::NewListenAddr { address, .. } = event {
                    println!("Listening on {:?}", address);
                }
            }
        }
    }
}