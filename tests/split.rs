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
use fastwebsockets::Frame;
use fastwebsockets::OpCode;
use hyper::server::conn::Http;
use hyper::service::service_fn;
use hyper::Body;
use hyper::Request;
use hyper::Response;
use tokio::net::TcpListener;

use fastwebsockets::handshake;
use fastwebsockets::WebSocketRead;
use fastwebsockets::WebSocketWrite;
use hyper::header::CONNECTION;
use hyper::header::UPGRADE;
use hyper::upgrade::Upgraded;

use std::future::Future;

use tokio::net::TcpStream;

use tokio::sync::mpsc::unbounded_channel;

const N_CLIENTS: usize = 20;

async fn handle_client(
  client_id: usize,
  fut: upgrade::UpgradeFut,
) -> Result<()> {
  let mut ws = fut.await?;
  ws.set_writev(false);
  let mut ws = fastwebsockets::FragmentCollector::new(ws);

  ws.write_frame(Frame::binary(client_id.to_ne_bytes().as_ref().into()))
    .await
    .unwrap();

  Ok(())
}

async fn server_upgrade(mut req: Request<Body>) -> Result<Response<Body>> {
  let (response, fut) = upgrade::upgrade(&mut req)?;

  let client_id: usize = req
    .headers()
    .get("CLIENT-ID")
    .unwrap()
    .to_str()
    .unwrap()
    .parse()
    .unwrap();
  tokio::spawn(async move {
    handle_client(client_id, fut).await.unwrap();
  });

  Ok(response)
}

async fn connect(
  client_id: usize,
) -> Result<(
  WebSocketRead<tokio::io::ReadHalf<Upgraded>>,
  WebSocketWrite<tokio::io::WriteHalf<Upgraded>>,
)> {
  let stream = TcpStream::connect("localhost:8080").await?;

  let req = Request::builder()
    .method("GET")
    .uri("http://localhost:8080/")
    .header("Host", "localhost:8080")
    .header(UPGRADE, "websocket")
    .header(CONNECTION, "upgrade")
    .header("CLIENT-ID", &format!("{}", client_id))
    .header(
      "Sec-WebSocket-Key",
      fastwebsockets::handshake::generate_key(),
    )
    .header("Sec-WebSocket-Version", "13")
    .body(Body::empty())?;

  let (ws, _) = handshake::client(&SpawnExecutor, req, stream).await?;
  Ok(ws.split(|s| tokio::io::split(s)))
}

async fn start_client(client_id: usize) -> Result<()> {
  let (mut r, _w) = connect(client_id).await.unwrap();
  let (write_queue_tx, _write_queue_rx) = unbounded_channel();
  let frame = r
    .read_frame(|frame| {
      let res = write_queue_tx
        .send(frame)
        .map_err(|_| Box::<dyn std::error::Error + Send + Sync>::from("Failed to send frame".to_owned()));
      async { res }
    })
    .await?;
  match frame.opcode {
    OpCode::Close => {}
    OpCode::Binary => {
      let n = usize::from_ne_bytes(frame.payload[..].try_into().unwrap());
      assert_eq!(n, client_id);
    }
    _ => {
      panic!("Unexpected");
    }
  }
  Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;
  println!("Server started, listening on {}", "127.0.0.1:8080");
  tokio::spawn(async move {
    loop {
      let (stream, _) = listener.accept().await.unwrap();
      tokio::spawn(async move {
        let conn_fut = Http::new()
          .serve_connection(stream, service_fn(server_upgrade))
          .with_upgrades();
        conn_fut.await.unwrap();
      });
    }
  });
  let mut tasks = Vec::with_capacity(N_CLIENTS);
  for client in 0..N_CLIENTS {
    tasks.push(start_client(client));
  }
  for handle in tasks {
    handle.await.unwrap();
  }
  Ok(())
}

struct SpawnExecutor;

impl<Fut> hyper::rt::Executor<Fut> for SpawnExecutor
where
  Fut: Future + Send + 'static,
  Fut::Output: Send + 'static,
{
  fn execute(&self, fut: Fut) {
    tokio::task::spawn(fut);
  }
}
