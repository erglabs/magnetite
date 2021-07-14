#![allow(unused_imports)]

use async_trait::async_trait;
use futures::{channel::mpsc, executor::LocalPool, prelude::*, task::SpawnExt, AsyncWriteExt};
use rand::{self, Rng};
use std::{io, iter};
use std::{collections::HashSet, num::NonZeroU16};
use libp2p::{
    core::{
        Multiaddr,
        PeerId,
        identity,
        muxing::StreamMuxerBox,
        transport::{self, Transport},
        upgrade::{self, read_length_prefixed, write_length_prefixed}
    },
    yamux::{YamuxConfig,self},
    tcp::{TcpConfig,self},
    swarm::{Swarm, SwarmEvent},
    noise::{NoiseConfig, X25519Spec, Keypair},
    request_response::*,
};

fn main() {
    let query = ScaffoldQuery("query".to_string().into_bytes());
    let response = ScaffoldResponse("response".to_string().into_bytes());

    // very important to make protocols an iterable list
    // and make sure that the protocols are bound to codecs
    // and direction hints
    let protocols = iter::once((ScaffoldProtocol(), ProtocolSupport::Full));
    let cfg = RequestResponseConfig::default();

    // magicaly bind peer to channel
    // basically creates a peer with stream bound to it
    // actual transprot does not matter
    // this is what client/server should do to use scaffold
    let (peer1_id, trans) = mk_transport();
    let queryprotoA = RequestResponse::new(ScaffoldCodec(), protocols.clone(), cfg.clone());
    let mut swarm1 = Swarm::new(trans, queryprotoA, peer1_id);
    
    // same as above
    let (peer2_id, trans) = mk_transport();
    let queryprotoB = RequestResponse::new(ScaffoldCodec(), protocols, cfg);
    let mut swarm2 = Swarm::new(trans, queryprotoB, peer2_id);

    // bind comm channel between peers to run in background,
    // i will use it to transfer commands to peers
    let (mut tx, mut rx) = mpsc::channel::<Multiaddr>(1);

    let addr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    swarm1.listen_on(addr).unwrap();

    let expected_query = query.clone();
    let expected_response = response.clone();

    let peer1 = async move {
        loop {
            match swarm1.select_next_some().await {
                SwarmEvent::NewListenAddr { address, .. }=> tx.send(address).await.unwrap(),
                SwarmEvent::Behaviour(RequestResponseEvent::Message {
                    peer,
                    message: RequestResponseMessage::Request { request, channel, .. }
                }) => {
                    print!("peer1::response");
                    assert_eq!(&request, &expected_query);
                    assert_eq!(&peer, &peer2_id);
                    swarm1.behaviour_mut().send_response(channel, response.clone()).unwrap();
                },
                SwarmEvent::Behaviour(RequestResponseEvent::ResponseSent {
                    peer, ..
                }) => {
                    println!("peer1::request");
                    assert_eq!(&peer, &peer2_id);
                }
                SwarmEvent::Behaviour(e) => panic!("Peer1: Unexpected event: {:?}", e),
                _ => {}
            }
        }
    };

    let num_pings: u8 = rand::thread_rng().gen_range(1, 100);

    let peer2 = async move {
        let mut count = 0;
        let addr = rx.next().await.unwrap();
        swarm2.behaviour_mut().add_address(&peer1_id, addr.clone());
        let mut req_id = swarm2.behaviour_mut().send_request(&peer1_id, query.clone());
        assert!(swarm2.behaviour().is_pending_outbound(&peer1_id, &req_id));

        loop {
            match swarm2.select_next_some().await {
                SwarmEvent::Behaviour(RequestResponseEvent::Message {
                    peer,
                    message: RequestResponseMessage::Response { request_id, response }
                }) => {
                    println!("peer2::request::id::{}", request_id);
                    count += 1;
                    assert_eq!(&response, &expected_response);
                    assert_eq!(&peer, &peer1_id);
                    assert_eq!(req_id, request_id);
                    if count >= num_pings {
                        return
                    } else {
                        req_id = swarm2.behaviour_mut().send_request(&peer1_id, query.clone());
                    }

                }
                SwarmEvent::Behaviour(e) =>panic!("Peer2: Unexpected event: {:?}", e),
                _ => {}
            }
        }
    };

    async_std::task::spawn(Box::pin(peer1));
    let () = async_std::task::block_on(peer2);
}

fn mk_transport() -> (PeerId, transport::Boxed<(PeerId, StreamMuxerBox)>) {
    let id_keys = identity::Keypair::generate_ed25519();
    let peer_id = id_keys.public().into_peer_id();
    let noise_keys = Keypair::<X25519Spec>::new().into_authentic(&id_keys).unwrap();
    (peer_id, TcpConfig::new()
        .nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(libp2p::yamux::YamuxConfig::default())
        .boxed())
}

// Simple Ping-Pong Protocol

#[derive(Debug, Clone)]
struct ScaffoldProtocol();

#[derive(Clone)]
struct ScaffoldCodec();

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScaffoldQuery(Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScaffoldResponse(Vec<u8>);

impl ProtocolName for ScaffoldProtocol {
    fn protocol_name(&self) -> &[u8] {
        "/scaffold/1".as_bytes()
    }
}

#[async_trait]
impl RequestResponseCodec for ScaffoldCodec {
    type Protocol = ScaffoldProtocol;
    type Request = ScaffoldQuery;
    type Response = ScaffoldResponse;

    async fn read_request<T>(&mut self, _: &ScaffoldProtocol, io: &mut T)
        -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 1024).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(ScaffoldQuery(vec))
    }

    async fn read_response<T>(&mut self, _: &ScaffoldProtocol, io: &mut T)
        -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 1024).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(ScaffoldResponse(vec))
    }

    async fn write_request<T>(&mut self, _: &ScaffoldProtocol, io: &mut T, ScaffoldQuery(data): ScaffoldQuery)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(&mut self, _: &ScaffoldProtocol, io: &mut T, ScaffoldResponse(data): ScaffoldResponse)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }
}
