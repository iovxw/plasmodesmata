use std::io;
use std::net::SocketAddr;
use std::rc::Rc;

use rustls::ServerConfig;
use futures::{Future, Stream as FutureStream, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

mod uint;
mod frame;
mod error;

/// A transport-layer connection between two endpoints.
pub struct Connection<S>
where
    S: AsyncRead + AsyncWrite,
{
    socket: S,
    counter: u32,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite,
{
    pub fn new(socket: S) -> Connection<S> {
        Connection {
            socket: socket,
            counter: 0,
        }
    }

    pub fn handle(&self) -> Handle {
        Handle {}
    }

    pub fn incoming(self) -> Incoming<S> {
        Incoming { conn: self }
    }
}

pub struct Handle {}

impl Handle {
    pub fn send_client_magic(&self) -> Result<(), io::Error> {
        unimplemented!()
    }
}

pub struct Incoming<S>
where
    S: AsyncRead + AsyncWrite,
{
    conn: Connection<S>,
}

impl<S> FutureStream for Incoming<S>
where
    S: AsyncRead + AsyncWrite,
{
    type Item = (Stream, SocketAddr);
    type Error = io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, io::Error> {
        unimplemented!()
    }
}

/// A bidirectional flow of frames within the HTTP/2 connection.
pub struct Stream {}
// TODO: impl Read Write AsyncRead AsyncWrite
