use std::net::SocketAddr;

use futures::prelude::*;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use bytes::Bytes;
use h2::{self, server as h2s};
use http::{Request, Response, Method, StatusCode};

use io::{copy_from_h2, copy_to_h2, Socket};

pub fn server(listen_addr: SocketAddr, server_addr: SocketAddr) {
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let done = listener.incoming().for_each(move |(client, client_addr)| {
        let handle2 = handle.clone();
        let connection = h2s::Server::handshake(client)
            .and_then(move |conn| {
                let handle = handle2.clone();
                println!("S: handshake done");
                conn.for_each(move |(request, stream)| {
                    println!("SSSS");
                    let s = server_handle(handle.clone(), request, stream)
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
            .map_err(|e| println!("{:?}", e));

        handle.spawn(connection);
        Ok(())
    });
    lp.run(done).unwrap();
}

#[async]
fn server_handle(
    handle: Handle,
    request: Request<h2s::Body<Bytes>>,
    mut stream: h2s::Stream<Bytes>,
) -> Result<(usize, usize), h2::Error> {
    println!("S0");
    let (parts, body) = request.into_parts();
    if parts.method != Method::CONNECT {
        unimplemented!();
    }
    let addr = "127.0.0.1:8080".parse().unwrap();
    let server = await!(TcpStream::connect(&addr, &handle))?;
    let server = Socket::new(server);
    let response = Response::builder().status(StatusCode::OK).body(()).unwrap();
    stream.send_response(response, false)?;
    println!("S1");
    let (server_reader, server_writer) = (server.clone(), server);
    let server_to_client = copy_to_h2(server_reader, stream);
    let client_to_server = copy_from_h2(body, server_writer);
    println!("S2");
    await!(
        client_to_server
            .map(|x| {
                println!("S: c-s done");
                x
            })
            .join(server_to_client.map(|x| {
                println!("S: s-c done");
                x
            }))
    )
}
