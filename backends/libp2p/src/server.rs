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
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::time::Duration;
use std::{
    error::Error,
    task::{Context, Poll},
};

use rmp_serde::Deserializer;
use serde::{Deserialize, Serialize};
// use rmp_serde::Serializer;

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

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // initial db ?
    let mut coredb: HashMap<String, String> = [
        ("configservice.address".into(), "localhost".into()),
        ("configservice.port".into(), "61250".into()),
        ("configservice.user".into(), "public".into()),
    ]
    .iter()
    .cloned()
    .collect();

    Builder::from_env(Env::default().default_filter_or("info")).init();
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
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
            .heartbeat_interval(Duration::from_secs(2))
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

        // sign with my identity key
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
        .listen_on("/ip4/127.0.0.1/tcp/61250".parse().unwrap())
        .unwrap();
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
            match swarm.poll_next_unpin(cx) {
                Poll::Ready(Some(gossip_event)) => match gossip_event {
                    GossipsubEvent::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    } => {
                        process(&message, &mut coredb, swarm.behaviour_mut());
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

fn process(
    what: &GossipsubMessage,
    db: &mut HashMap<String, String>,
    core: &mut libp2p::gossipsub::Gossipsub<
        libp2p::gossipsub::IdentityTransform,
        libp2p::gossipsub::subscription_filter::AllowAllSubscriptionFilter,
    >,
) {
    let topic = Topic::new("general");
    let cur = Cursor::new(&what.data);
    let mut de = Deserializer::new(cur);
    let message_raw: Message = Deserialize::deserialize(&mut de).unwrap();
    let responsemsg: Message;
    match message_raw.msgtype {
        MsgType::Control => { /*nothingyet*/ }
        MsgType::Notification => { /*expliciteignore*/ }
        MsgType::Get => {
            let value: String = String::from_utf8(message_raw.payload.clone()).unwrap();
            let out: String = db.get(&value).cloned().unwrap_or("NONE".to_owned());
            responsemsg = Message {
                id: message_raw.id,
                msgtype: MsgType::Notification,
                payload: out.as_bytes().to_vec(),
            };
            let mut buf: Vec<u8> = Vec::new();
            responsemsg
                .serialize(&mut rmp_serde::Serializer::new(&mut buf))
                .unwrap();
            core.publish(topic, buf).unwrap();
        }
        MsgType::Set => {
            let value: String = String::from_utf8(message_raw.payload.clone()).unwrap();
            let out: String = db.get(&value).cloned().unwrap_or("NONE".to_owned());
            db.remove(&out);
            db.insert(
                out,
                String::from_utf8(message_raw.payload).unwrap_or("".to_owned()),
            );
            responsemsg = Message {
                id: message_raw.id, //response with the same id
                msgtype: MsgType::Notification,
                payload: "Ok".as_bytes().to_vec(),
            };
            let mut buf: Vec<u8> = Vec::new();
            responsemsg
                .serialize(&mut rmp_serde::Serializer::new(&mut buf))
                .unwrap();
            core.publish(topic, buf).unwrap();
        }
    }
}
