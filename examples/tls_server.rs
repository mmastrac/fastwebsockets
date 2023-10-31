// Copyright 2023 Divy Srivastava <dj.srivastava23@gmail.com>
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use anyhow::Result;
use fastwebsockets::upgrade;
use fastwebsockets::OpCode;
use hyper::server::conn::Http;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use rustls_tokio_stream::rustls;
use rustls_tokio_stream::rustls::Certificate;
use rustls_tokio_stream::rustls::PrivateKey;
use rustls_tokio_stream::rustls::ServerConfig;
use rustls_tokio_stream::TlsStream;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn handle_client(fut: upgrade::UpgradeFut) -> Result<()> {
  let mut ws = fut.await?;
  ws.set_writev(false);
  let mut ws = fastwebsockets::FragmentCollector::new(ws);

  loop {
    let frame = ws.read_frame().await?;
    match frame.opcode {
      OpCode::Close => break,
      OpCode::Text | OpCode::Binary => {
        ws.write_frame(frame).await?;
      }
      _ => {}
    }
  }

  Ok(())
}

async fn server_upgrade(mut req: Request<Body>) -> Result<Response<Body>> {
  let (response, fut) = upgrade::upgrade(&mut req)?;

  tokio::spawn(async move {
    if let Err(e) = handle_client(fut).await {
      eprintln!("Error in websocket connection: {}", e);
    }
  });

  Ok(response)
}

fn server_config() -> Result<ServerConfig> {
  static KEY: &[u8] = include_bytes!("./localhost.key");
  static CERT: &[u8] = include_bytes!("./localhost.crt");

  let mut keys: Vec<PrivateKey> =
    rustls_pemfile::pkcs8_private_keys(&mut &*KEY)
      .map(|mut certs| certs.drain(..).map(PrivateKey).collect())
      .unwrap();
  let certs = rustls_pemfile::certs(&mut &*CERT)
    .map(|mut certs| certs.drain(..).map(Certificate).collect())
    .unwrap();
  dbg!(&certs);
  let config = rustls::ServerConfig::builder()
    .with_safe_defaults()
    .with_no_client_auth()
    .with_single_cert(certs, keys.remove(0))?;
  Ok(config)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;
  println!("Server started, listening on {}", "127.0.0.1:8080");
  let server_config = Arc::new(server_config()?);
  loop {
    let (stream, _) = listener.accept().await?;
    println!("Client connected");
    let server_config = server_config.clone();
    tokio::spawn(async move {
      let stream = TlsStream::new_server_side(stream, server_config, None);
      let conn_fut = Http::new()
        .serve_connection(stream, service_fn(server_upgrade))
        .with_upgrades();
      if let Err(e) = conn_fut.await {
        println!("An error occurred: {:?}", e);
      }
    });
  }
}
