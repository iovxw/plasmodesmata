#![feature(proc_macro, conservative_impl_trait, generators)]

extern crate bytes;
extern crate rustls;
extern crate futures_await as futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_rustls;
extern crate http;
extern crate h2;
extern crate io_dump;

use std::net::SocketAddr;

use futures::prelude::*;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::io::shutdown;
use bytes::{BytesMut, Bytes, IntoBuf};
use h2::{server as h2s, client as h2c};

mod pool;
use pool::*;

macro_rules! poll {
    ($e:expr) => ({
        loop {
            match $e {
                ::futures::__rt::Ok(::futures::Async::Ready(e)) => {
                    break ::futures::__rt::Ok(e)
                }
                ::futures::__rt::Ok(::futures::Async::NotReady) => {}
                ::futures::__rt::Err(e) => {
                    break ::futures::__rt::Err(e)
                }
            }
            yield
        }
    })
}

fn client(listen_addr: SocketAddr, server_addr: SocketAddr) {
    let mut lp = Core::new().unwrap();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let pool = pool::H2ClientPool::new(lp.handle(), server_addr);
    let pool_handle = pool.handle();
    let done = listener.incoming().for_each(move |(client, client_addr)| {
        client_handle(client, pool_handle.clone())
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
            })
    });
    lp.handle().spawn(pool);
    lp.run(done).unwrap();
}

#[async]
fn client_handle(client: TcpStream, pool_handle: PoolHandle) -> Result<(usize, usize), h2::Error> {
    let req = http::Request::builder()
        .method(http::Method::CONNECT)
        .uri("https://iovxw.net/")
        .body(())
        .unwrap();
    let (client_reader, client_writer) = client.split();
    println!("C: request");
    let mut stream = await!(pool_handle.send_request(req, false))?;
    println!("C: request done");
    let (parts, body) = poll!(stream.poll_response())?.into_parts();
    println!("CCCCC");
    if parts.status != http::StatusCode::OK {
        unimplemented!();
    }
    let server_to_client = copy_from_h2(body, client_writer);
    let client_to_server = copy_to_h2(client_reader, stream);
    await!(client_to_server.join(server_to_client))
}

#[async]
fn copy_from_h2<W: AsyncWrite + 'static, B: Stream<Item = Bytes, Error = h2::Error> + 'static>(
    src: B,
    mut dst: W,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    #[async]
    for buf in src {
        let mut buf = buf.into_buf();
        let n = poll!(dst.write_buf(&mut buf))?;
        counter += n;
    }
    shutdown(dst);
    Ok(counter)
}

trait SendData<B: IntoBuf> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error>;
}

impl<B: IntoBuf> SendData<B> for h2c::Stream<B> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error> {
        h2c::Stream::send_data(self, data, end_of_stream)
    }
}
impl<B: IntoBuf> SendData<B> for h2s::Stream<B> {
    fn send_data(&mut self, data: B, end_of_stream: bool) -> Result<(), h2::Error> {
        h2s::Stream::send_data(self, data, end_of_stream)
    }
}

#[async]
fn copy_to_h2<R: AsyncRead + 'static, H: SendData<Bytes> + 'static>(
    mut src: R,
    mut dst: H,
) -> Result<usize, h2::Error> {
    let mut counter = 0;
    let mut buf = BytesMut::with_capacity(1024);
    loop {
        let n = poll!(src.read_buf(&mut buf))?;
        if n == 0 {
            dst.send_data(buf.take().freeze(), true)?;
            break;
        } else {
            dst.send_data(buf.take().freeze(), false)?;
        }
        counter += n;
    }
    Ok(counter)
}

fn server(listen_addr: SocketAddr, server_addr: SocketAddr) {
    let mut lp = Core::new().unwrap();
    let handle = lp.handle();

    let listener = TcpListener::bind(&listen_addr, &lp.handle()).unwrap();
    println!("Listening on: {}", listen_addr);
    println!("Proxying to: {}", server_addr);

    let done = listener.incoming().for_each(move |(client, client_addr)| {
        let handle2 = handle.clone();
        let client = io_dump::Dump::to_stdout(client);
        let connection = h2s::Server::handshake(client)
            .and_then(move |conn| {
                let handle = handle2.clone();
                println!("S: handshake done");
                conn.for_each(move |(request, stream)| {
                    println!("SSSS");
                    server_handle(handle.clone(), request, stream)
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
    request: http::Request<h2s::Body<Bytes>>,
    stream: h2s::Stream<Bytes>,
) -> Result<(usize, usize), h2::Error> {
    let (parts, body) = request.into_parts();
    if parts.method != http::Method::CONNECT {
        unimplemented!();
    }
    let addr = "127.0.0.1:8080".parse().unwrap();
    let server = await!(TcpStream::connect(&addr, &handle))?;
    let (server_reader, server_writer) = server.split();
    let server_to_client = copy_to_h2(server_reader, stream);
    let client_to_server = copy_from_h2(body, server_writer);
    await!(client_to_server.join(server_to_client))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn it_works() {
        let client_addr = "127.0.0.1:3345".parse().unwrap();
        let server_addr = "127.0.0.1:3346".parse().unwrap();
        let s = ::std::thread::spawn(move || server(server_addr, server_addr));
        let c = ::std::thread::spawn(move || client(client_addr, server_addr));
        s.join().unwrap();
        c.join().unwrap();
    }
}
