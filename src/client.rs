use std::net::SocketAddr;
use std::sync::Arc;

use http::{Request, Method, StatusCode};
use futures::prelude::*;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::Core;
use h2;
use rustls;

use pool::H2ClientPool;
use io::{copy_from_h2, copy_to_h2, Socket};

pub fn client(
    listen_addr: SocketAddr,
    tls_config: Arc<rustls::ClientConfig>,
    server_domain: String,
    server_addr: SocketAddr,
) {
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let pool = H2ClientPool::new(lp.handle(), tls_config, server_domain.clone(), server_addr);
    let done = listener.incoming().for_each(move |(client, client_addr)| {
        let c = client_handle(client, &server_domain, pool.clone())
            .map(move |(client_to_server, server_to_client)| {
                println!(
                    "[{}]: SEND: {}, RECV: {}",
                    client_addr,
                    client_to_server,
                    server_to_client
                );
            })
            .or_else(move |e| {
                println!("[{}] ERROR: {}", client_addr, e);
                Ok(())
            });

        handle.spawn(c);
        Ok(())
    });
    lp.run(done).unwrap();
}

fn client_handle<'a>(
    client: TcpStream,
    server_domain: &str,
    pool: H2ClientPool,
) -> impl Future<Item = (usize, usize), Error = h2::Error> + 'a {
    let req = Request::builder()
        .method(Method::CONNECT)
        .uri(format!("https://{}/", server_domain).as_str())
        .body(())
        .unwrap();
    async_block! {
        let client = Socket::new(client);
        let (client_reader, client_writer) = (client.clone(), client);
        let (response, stream) = await!(pool.send_request(req, false))?;
        let (parts, recvstream) = await!(response)?.into_parts();
        if parts.status != StatusCode::OK {
            unimplemented!();
        }
        let server_to_client = copy_from_h2(recvstream, client_writer);
        let client_to_server = copy_to_h2(client_reader, stream);
        await!(client_to_server.join(server_to_client))
    }
}
