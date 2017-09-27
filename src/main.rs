#![feature(proc_macro)]
#![feature(conservative_impl_trait)]
#![feature(generators)]

extern crate bytes;
extern crate rustls;
extern crate futures_await as futures;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_rustls;
extern crate http;
extern crate h2;

#[macro_use]
mod macros;
mod client;
use client::client;
mod server;
use server::server;
mod pool;
mod io;

fn main() {
    let client_addr = "127.0.0.1:3345".parse().unwrap();
    let server_addr = "127.0.0.1:3346".parse().unwrap();
    let s = ::std::thread::spawn(move || server(server_addr, server_addr));
    let c = ::std::thread::spawn(move || client(client_addr, server_addr));
    s.join().unwrap();
    c.join().unwrap();
}
