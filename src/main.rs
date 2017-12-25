#![feature(proc_macro)]
#![feature(conservative_impl_trait)]
#![feature(generators)]

extern crate bytes;
extern crate env_logger;
extern crate futures_await as futures;
extern crate h2;
extern crate http;
#[macro_use]
extern crate log;
extern crate rustls;
extern crate structopt;
#[macro_use]
extern crate structopt_derive;
extern crate tokio_core;
extern crate tokio_io;
extern crate tokio_rustls;
extern crate webpki;
extern crate webpki_roots;

use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::Arc;
use std::net::SocketAddr;

use webpki_roots::TLS_SERVER_ROOTS;
use structopt::StructOpt;

#[macro_use]
mod macros;
mod client;
use client::client;
mod server;
use server::server;
mod pool;
mod io;

const ALPN_H2: &str = "h2";

#[derive(StructOpt)]
#[structopt(name = "plasmodesmata", about = "A http2 tunnel")]
enum Command {
    #[structopt(name = "client")]
    Client {
        #[structopt(short = "l", help = "Local address")] local: String,
        #[structopt(short = "r", help = "Remote address")] remote: String,
        #[structopt(short = "d", help = "Remote domain")] domain: String,
    },
    #[structopt(name = "server")]
    Server {
        #[structopt(short = "l", help = "Local address")] local: String,
        #[structopt(short = "r", help = "Remote address")] remote: String,
        #[structopt(short = "c", help = "Specify certificate file")] certificate: String,
        #[structopt(short = "k", help = "Specify certificate private key")] key: String,
    },
}

fn main() {
    env_logger::Builder::new()
        .filter(None, log::LevelFilter::Info)
        .init();
    let cmd = Command::from_args();
    match cmd {
        Command::Client {
            local,
            remote,
            domain,
        } => {
            let local_addr: SocketAddr = local
                .parse()
                .expect("Local address is not a valid IP address");
            // FIXME: resolve DNS
            let server_addr: SocketAddr = remote
                .parse()
                .expect("Server address is not a valid IP address");
            let mut tls_config = rustls::ClientConfig::new();
            tls_config
                .root_store
                .add_server_trust_anchors(&TLS_SERVER_ROOTS);
            tls_config.alpn_protocols.push(ALPN_H2.to_owned());
            let tls_config = Arc::new(tls_config);
            client(local_addr, tls_config, domain, server_addr)
        }
        Command::Server {
            local,
            remote,
            certificate,
            key,
        } => {
            let local_addr: SocketAddr = local
                .parse()
                .expect("Local address is not a valid IP address");
            let server_addr: SocketAddr = remote
                .parse()
                .expect("Server address is not a valid IP address");
            let mut tls_config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
            tls_config.alpn_protocols.push(ALPN_H2.to_owned());
            let certs = load_certs(&certificate);
            let privkey = load_private_key(&key);
            let ocsp = load_ocsp(&None);
            tls_config.set_single_cert_with_ocsp_and_sct(certs, privkey, ocsp, vec![]);
            let tls_config = Arc::new(tls_config);
            server(local_addr, tls_config, server_addr);
        }
    }
}

fn load_certs(filename: &str) -> Vec<rustls::Certificate> {
    let certfile = File::open(filename).expect("Cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    rustls::internal::pemfile::certs(&mut reader).unwrap()
}

fn load_private_key(filename: &str) -> rustls::PrivateKey {
    let rsa_keys = {
        let keyfile = File::open(filename).expect("Cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        rustls::internal::pemfile::rsa_private_keys(&mut reader)
            .expect("File contains invalid rsa private key")
    };

    let pkcs8_keys = {
        let keyfile = File::open(filename).expect("Cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        rustls::internal::pemfile::pkcs8_private_keys(&mut reader)
            .expect("File contains invalid pkcs8 private key (encrypted keys not supported)")
    };

    // prefer to load pkcs8 keys
    if !pkcs8_keys.is_empty() {
        pkcs8_keys[0].clone()
    } else {
        assert!(!rsa_keys.is_empty());
        rsa_keys[0].clone()
    }
}

fn load_ocsp(filename: &Option<String>) -> Vec<u8> {
    let mut ret = Vec::new();

    if let &Some(ref name) = filename {
        File::open(name)
            .expect("Cannot open ocsp file")
            .read_to_end(&mut ret)
            .unwrap();
    }

    ret
}
