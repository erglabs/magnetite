use async_trait::async_trait;
use futures::{prelude::*, AsyncWrite, AsyncRead};
use std::io;
use libp2p::{
    core::{
        upgrade::{self, read_length_prefixed, write_length_prefixed}
    },
    request_response::*,
};

pub use libp2p::request_response::*;
use serde::{Serialize, Deserialize};
use anyhow::{Context, Result};
use bytes::{Buf, BufMut, Bytes, BytesMut};

// warning
// you need to add following iterator to your code, 
// its the mapping between protocol definition and actual protocol codec
//    let protocols = iter::once((ProbeProtocol(), ProtocolSupport::Full));
//    let cfg = RequestResponseConfig::default();

// the only thing that this protocol does is wrapping request-response inside something usable.
// it does not care about payload nor the behacior 
// you have to add your own logic to make it react to certain events,
// i would advice doing routing-like handler and then branch out the possiblites, 
// it will be the most flexible way of aprocahing it. 
// otherwise you will have to change the code here to do some magic. Your choice of course, but
// please remember that  modified code will not be able to talk to this generic wrapper, so
// best idea would be to change protocol uri name. 
//  god speed,
//   esavier

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct ConfigurationRequest {
    key : String,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
pub struct ConfigurationResponse {
    key : String,
    value : String,
}

#[derive(Debug, Clone)]
pub struct ProbeProtocol();

#[derive(Clone)]
pub struct ProbeCodec();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeQuery(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResponse(pub Vec<u8>);

impl ProtocolName for ProbeProtocol {
    fn protocol_name(&self) -> &[u8] {
        "/probe/1".as_bytes()
    }
}

#[async_trait]
impl RequestResponseCodec for ProbeCodec {
    type Protocol = ProbeProtocol;
    type Request = ProbeQuery;
    type Response = ProbeResponse;

    async fn read_request<T>(&mut self, _: &ProbeProtocol, io: &mut T)
        -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 1024).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(ProbeQuery(vec))
    }

    async fn read_response<T>(&mut self, _: &ProbeProtocol, io: &mut T)
        -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 1024).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(ProbeResponse(vec))
    }

    async fn write_request<T>(&mut self, _: &ProbeProtocol, io: &mut T, ProbeQuery(data): ProbeQuery)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(&mut self, _: &ProbeProtocol, io: &mut T, ProbeResponse(data): ProbeResponse)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }
}
