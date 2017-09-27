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
extern crate webpki_roots;

use std::fs::File;
use std::io::{Read, BufReader};

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
    let target_addr = "127.0.0.1:8080".parse().unwrap();

    if std::env::args().count() == 1 {
        client(client_addr, "localhost.iovxw.net".into(), server_addr)
    } else {
        let mut server_tls_config = rustls::ServerConfig::new();
        let certs = load_certs("domain.cer");
        let privkey = load_private_key("domain.key");
        let ocsp = load_ocsp(&None);
        server_tls_config.set_single_cert_with_ocsp_and_sct(certs, privkey, ocsp, vec![]);
        server(server_addr, server_tls_config, target_addr)
    }
}

fn load_certs(filename: &str) -> Vec<rustls::Certificate> {
    let certfile = File::open(filename).expect("cannot open certificate file");
    let mut reader = BufReader::new(certfile);
    rustls::internal::pemfile::certs(&mut reader).unwrap()
}

fn load_private_key(filename: &str) -> rustls::PrivateKey {
    let rsa_keys = {
        let keyfile = File::open(filename).expect("cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        rustls::internal::pemfile::rsa_private_keys(&mut reader)
            .expect("file contains invalid rsa private key")
    };

    let pkcs8_keys = {
        let keyfile = File::open(filename).expect("cannot open private key file");
        let mut reader = BufReader::new(keyfile);
        rustls::internal::pemfile::pkcs8_private_keys(&mut reader).expect(
            "file contains invalid pkcs8 private key (encrypted keys not supported)",
        )
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
            .expect("cannot open ocsp file")
            .read_to_end(&mut ret)
            .unwrap();
    }

    ret
}
