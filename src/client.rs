use std::net::SocketAddr;

use http::{Request, Method, StatusCode};
use futures::prelude::*;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::Core;
use h2;

use pool::{H2ClientPool, PoolHandle};
use io::{copy_from_h2, copy_to_h2, Socket};

pub fn client(listen_addr: SocketAddr, server_addr: SocketAddr) {
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let pool = H2ClientPool::new(lp.handle(), server_addr);
    let pool_handle = pool.handle();
    let done = listener.incoming().for_each(move |(client, client_addr)| {
        let c = client_handle(client, pool_handle.clone())
            .map(move |(client_to_server, server_to_client)| {
                println!(
                    "{:?}: {}, {}",
                    client_addr,
                    client_to_server,
                    server_to_client
                );
            })
            .or_else(|e| {
                println!("{:?}", e);
                Ok(())
            });

        handle.spawn(c);
        Ok(())
    });
    lp.handle().spawn(pool);
    lp.run(done).unwrap();
}

#[async]
fn client_handle(client: TcpStream, pool_handle: PoolHandle) -> Result<(usize, usize), h2::Error> {
    let req = Request::builder()
        .method(Method::CONNECT)
        .uri("https://iovxw.net/")
        .body(())
        .unwrap();
    let client = Socket::new(client);
    let (client_reader, client_writer) = (client.clone(), client);
    println!("C: request");
    let mut stream = await!(pool_handle.send_request(req, false))?;
    println!("C: request done");
    let (parts, body) = poll!(stream.poll_response())?.into_parts();
    println!("CCCCC");
    if parts.status != StatusCode::OK {
        unimplemented!();
    }
    let server_to_client = copy_from_h2(body, client_writer);
    let client_to_server = copy_to_h2(client_reader, stream);
    await!(
        client_to_server
            .map(|x| {
                println!("C: c-s done");
                x
            })
            .join(server_to_client.map(|x| {
                println!("C: s-c done");
                x
            }))
    )
}
