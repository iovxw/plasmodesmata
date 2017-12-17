use std::net::SocketAddr;
use std::collections::VecDeque;
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::Arc;

use futures::prelude::*;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Handle;
use bytes::Bytes;
use h2::{self, client as h2c};
use http::Request;
use rustls::{self, Session};
use tokio_rustls::ClientConfigExt;

use super::ALPN_H2;

#[derive(Clone)]
pub struct H2ClientPool {
    domain: Rc<String>,
    addr: SocketAddr,
    tls_config: Arc<rustls::ClientConfig>,
    handle: Handle,
    task: Rc<RefCell<Option<::futures::task::Task>>>,
    pool: Rc<RefCell<VecDeque<h2c::SendRequest<Bytes>>>>,
}

impl H2ClientPool {
    pub fn new(
        handle: Handle,
        tls_config: Arc<rustls::ClientConfig>,
        domain: String,
        addr: SocketAddr,
    ) -> H2ClientPool {
        H2ClientPool {
            domain: Rc::new(domain),
            addr: addr,
            handle: handle,
            tls_config: tls_config,
            task: Rc::new(RefCell::new(None)),
            pool: Rc::new(RefCell::new(VecDeque::new())),
        }
    }
}

impl H2ClientPool {
    pub fn send_request<'a>(
        &self,
        request: Request<()>,
        end_of_stream: bool,
    ) -> impl Future<Item = (h2c::ResponseFuture, h2::SendStream<Bytes>), Error = h2::Error> + 'a {
        let s = self.clone();
        async_block! {
            let mut client = await!(s.pop())?;
            let stream = client.send_request(request, end_of_stream)?;
            s.pool.borrow_mut().push_back(client);
            Ok(stream)
        }
    }

    fn new_client<'a>(
        &self,
    ) -> impl Future<Item = h2c::SendRequest<Bytes>, Error = h2::Error> + 'a {
        let task = self.task.clone();
        let domain = self.domain.clone();
        let tls_config = self.tls_config.clone();
        let handle = self.handle.clone();
        TcpStream::connect(&self.addr, &self.handle)
            .map_err(h2::Error::from)
            .and_then(move |tcp| {
                tls_config.connect_async(&domain, tcp).map_err(Into::into)
            })
            .and_then(move |socket| {
                let negotiated_protcol = {
                    let (_, session) = socket.get_ref();
                    session.get_alpn_protocol()
                };
                if let Some(ALPN_H2) = negotiated_protcol.as_ref().map(|x| &**x) {
                } else {
                    println!("not a http2 server!");
                }
                if let Some(ref task) = *task.borrow() {
                    task.notify();
                }
                h2c::Builder::new().handshake(socket)
            })
            .and_then(move |(client, connection)| {
                handle.spawn(connection.map_err(
                    |e| eprintln!("h2 connection error: {}", e),
                ));
                Ok(client)
            })
    }

    fn pop<'a>(&self) -> impl Future<Item = h2c::SendRequest<Bytes>, Error = h2::Error> + 'a {
        let s = self.clone();
        async_block! {
            loop {
                let client = s.pool.borrow_mut().pop_front();
                let mut client = match client {
                    Some(x) => x,
                    None => await!(s.new_client())?,
                };

                if client.poll_ready().is_ok() {
                    return Ok(client);
                }
            }
        }
    }
}
