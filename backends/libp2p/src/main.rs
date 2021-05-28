#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(unused_variables)]

use async_std::task;
use env_logger::{Builder, Env};
use futures::prelude::*;
use libp2p::gossipsub::MessageId;
use libp2p::gossipsub::{
    GossipsubEvent, GossipsubMessage, IdentTopic as Topic, MessageAuthenticity, ValidationMode,
};
use libp2p::{gossipsub, identity, PeerId};
use rmp_serde::Deserializer;
use rmp_serde::Serializer;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::time::Duration;
use std::{
    error::Error,
    task::{Context, Poll},
};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
enum MsgType {
    Control,
    Notification,
    Set,
    Get,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct Message {
    id: usize,
    msgtype: MsgType,
    payload: Vec<u8>,
}

impl Default for Message {
    fn default() -> Self {
        Message {
            id: 0,
            msgtype: MsgType::Notification,
            payload: br#"Hello"#.to_vec(),
        }
    }
}

struct Client {
    pub id: usize,
    pub wants: bool,
    pub wanted: HashMap<String, String>,
    pub mapping: HashMap<usize, String>,
}

impl Client {
    pub fn default() -> Self {
        Client {
            id: 0,
            wants: true,
            wanted: [
                ("configservice.address".into(), "".into()),
                ("configservice.port".into(), "".into()),
                ("configservice.user".into(), "".into()),
            ]
            .iter()
            .cloned()
            .collect(),
            mapping: HashMap::new(),
        }
    }
    fn poll(self: &mut Self) -> Poll<()> {
        self.wants = false;
        for (_, value) in self.wanted.iter() {
            if value == &"".to_string() {
                self.wants = true;
            }
        }
        match self.wants {
            true => Poll::Pending, //pending only on seeding
            false => Poll::Ready(()),
        }
    }
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // let client = Arc::new(Mutex::new(Client::default()));
    let mut client = Client::default();
    Builder::from_env(Env::default().default_filter_or("info")).init();

    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public()); //todo
    println!("Local peer id: {:?}", local_peer_id);

    let transport = libp2p::development_transport(local_key.clone()).await?;
    let topic = Topic::new("default");
    let mut swarm = {
        let message_id_fn = |message: &GossipsubMessage| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            MessageId::from(s.finish().to_string())
        };
        let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(30))
            .validation_mode(ValidationMode::Permissive)
            .message_id_fn(message_id_fn)
            .mesh_n(2)
            .mesh_n_low(2)
            .mesh_n_high(36)
            .gossip_lazy(2)
            .history_gossip(3)
            .mesh_outbound_min(1)
            .build()
            .expect("Valid config");
        let mut gossipsub: gossipsub::Gossipsub =
            gossipsub::Gossipsub::new(MessageAuthenticity::Signed(local_key), gossipsub_config)
                .expect("Correct configuration");
        gossipsub.subscribe(&topic).unwrap();
        if let Some(explicit) = std::env::args().nth(2) {
            let explicit = explicit.clone();
            match explicit.parse() {
                Ok(id) => gossipsub.add_explicit_peer(&id),
                Err(err) => println!("Failed to parse explicit peer id: {:?}", err),
            }
        }
        libp2p::Swarm::new(transport, gossipsub, local_peer_id)
    };
    swarm
        .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .unwrap(); //any port
    if let Some(to_dial) = std::env::args().nth(1) {
        let dialing = to_dial.clone();
        match to_dial.parse() {
            Ok(to_dial) => match swarm.dial_addr(to_dial) {
                Ok(_) => println!("Dialed {:?}", dialing),
                Err(e) => println!("Dial {:?} failed: {:?}", dialing, e),
            },
            Err(err) => println!("Failed to parse address to dial: {:?}", err),
        }
    }
    let mut listening = false;
    task::block_on(future::poll_fn(move |cx: &mut Context<'_>| {
        loop {
            println!("loop1");
            if swarm
                .behaviour()
                .all_mesh_peers()
                .collect::<Vec<&PeerId>>()
                .len()
                < 2
            {
                break;
            } else {
                match client.poll() {
                    Poll::Pending => ask(&mut client, swarm.behaviour_mut()),
                    _ => break,
                }
            }
        }

        loop {
            println!("loop2");
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(gossip_event)) => match gossip_event {
                    GossipsubEvent::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    } => {
                        process(&message, &mut client);
                        println!(
                            "Got message: {} with id: {} from peer: {:?}",
                            String::from_utf8_lossy(&message.data),
                            id,
                            peer_id
                        );
                    }
                    _ => {}
                },
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        if !listening {
            for addr in libp2p::Swarm::listeners(&swarm) {
                println!("Listening on {:?}", addr);
                listening = true;
            }
        }

        Poll::Pending
    }))
}

fn ask(
    client: &mut Client,
    core: &mut libp2p::gossipsub::Gossipsub<
        libp2p::gossipsub::IdentityTransform,
        libp2p::gossipsub::subscription_filter::AllowAllSubscriptionFilter,
    >,
) {
    println!("ask");
    let topic = Topic::new("general");

    for (key, _) in client.wanted.clone().iter() {
        let request = Message {
            id: client.id,
            msgtype: MsgType::Get,
            payload: key.as_bytes().to_vec(),
        };
        let mut buf = Vec::new();
        request.serialize(&mut Serializer::new(&mut buf)).unwrap();
        core.publish(topic.clone(), buf).unwrap();
        client.mapping.insert(client.id, key.clone());
        client.id += 1;
    }
}

fn process(what: &GossipsubMessage, client: &mut Client) {
    println!("process");
    let cur = Cursor::new(&what.data);
    let mut de = Deserializer::new(cur);
    let message_raw: Message = Deserialize::deserialize(&mut de).unwrap();

    match message_raw.msgtype {
        MsgType::Control => { /*nothingyet*/ }
        MsgType::Get => { /*expliciteignore*/ }
        MsgType::Set => { /*expliciteignore*/ }
        MsgType::Notification => {
            let payload: String = String::from_utf8(message_raw.payload.clone()).unwrap();
            let key: String = client.mapping[&message_raw.id].clone();
            client.wanted.insert(key, payload);
        }
    }
}
