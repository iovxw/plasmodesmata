use std::net::SocketAddr;
use std::sync::Arc;

use futures::prelude::*;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use bytes::Bytes;
use h2::{self, server as h2s};
use http::{Request, Response, Method, StatusCode};
use rustls::{self, Session};
use tokio_rustls::ServerConfigExt;

use io::{copy_from_h2, copy_to_h2, Socket};

use super::ALPN_H2;

pub fn server(
    listen_addr: SocketAddr,
    tls_config: Arc<rustls::ServerConfig>,
    server_addr: SocketAddr,
) {
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let done = listener.incoming().for_each(move |(client, client_addr)| {
        let handle2 = handle.clone();
        println!("new client: {}", client_addr);
        let connection = tls_config
            .accept_async(client)
            .map_err(Into::into)
            .and_then(move |client| {
                let negotiated_protcol = {
                    let (_, session) = client.get_ref();
                    session.get_alpn_protocol()
                };
                if let Some(ALPN_H2) = negotiated_protcol.as_ref().map(|x| &**x) {
                } else {
                    println!("not a http2 client!");
                }
                h2s::Server::handshake(client).and_then(move |server| {
                    let handle = handle2.clone();
                    server.for_each(move |(request, respond)| {
                        let s = server_handle(handle.clone(), request, respond, server_addr)
                            .map(move |(client_to_server, server_to_client)| {
                                println!(
                                    "{:?}: {}, {}",
                                    client_addr,
                                    client_to_server,
                                    server_to_client
                                );
                            })
                            .or_else(|e: h2::Error| {
                                println!("{:?}", e);
                                Ok(())
                            });

                        handle.spawn(s);
                        Ok(())
                    }).and_then(move |_| {
                            println!("Connection close: {}", client_addr);
                            Ok(())
                        })
                })
            })
            .map_err(|e| println!("{:?}", e));

        handle.spawn(connection);
        Ok(())
    });
    lp.run(done).unwrap();
}

#[async]
fn server_handle(
    handle: Handle,
    request: Request<h2::RecvStream>,
    mut respond: h2s::Respond<Bytes>,
    server_addr: SocketAddr,
) -> Result<(usize, usize), h2::Error> {
    let (parts, body) = request.into_parts();
    if parts.method != Method::CONNECT {
        unimplemented!();
    }
    let server = await!(TcpStream::connect(&server_addr, &handle))?;
    let server = Socket::new(server);
    let response = Response::builder().status(StatusCode::OK).body(()).unwrap();
    let sendstream = respond.send_response(response, false)?;
    let (server_reader, server_writer) = (server.clone(), server);
    let server_to_client = copy_to_h2(server_reader, sendstream);
    let client_to_server = copy_from_h2(body, server_writer);
    await!(client_to_server.join(server_to_client))
}
