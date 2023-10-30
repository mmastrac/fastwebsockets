use std::future::Future;

use anyhow::Result;
use fastwebsockets::FragmentCollector;
use fastwebsockets::Frame;
use fastwebsockets::OpCode;
use hyper::header::CONNECTION;
use hyper::header::UPGRADE;
use hyper::upgrade::Upgraded;
use hyper::Body;
use hyper::Request;
use rustls_tokio_stream::rustls;
use rustls_tokio_stream::rustls::ClientConfig;
use rustls_tokio_stream::rustls::OwnedTrustAnchor;
use rustls_tokio_stream::TlsStream;
use tokio::net::TcpStream;

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

fn client_config() -> Result<ClientConfig> {
  let mut root_store = rustls::RootCertStore::empty();

  root_store.add_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(
    |ta| {
      OwnedTrustAnchor::from_subject_spki_name_constraints(
        ta.subject,
        ta.spki,
        ta.name_constraints,
      )
    },
  ));

  let config = ClientConfig::builder()
    .with_safe_defaults()
    .with_root_certificates(root_store)
    .with_no_client_auth();

  Ok(config)
}

async fn connect(domain: &str) -> Result<FragmentCollector<Upgraded>> {
  let mut addr = String::from(domain);
  addr.push_str(":9443"); // Port number for binance stream

  let tcp_stream = TcpStream::connect(&addr).await?;
  let domain = rustls::ServerName::try_from(domain).map_err(|_| {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid dnsname")
  })?;

  let tls_stream = TlsStream::new_client_side(
    tcp_stream,
    client_config()?.into(),
    domain,
    None,
  );

  let req = Request::builder()
    .method("GET")
    .uri(format!("wss://{}/ws/btcusdt@bookTicker", &addr)) //stream we want to subscribe to
    .header("Host", &addr)
    .header(UPGRADE, "websocket")
    .header(CONNECTION, "upgrade")
    .header(
      "Sec-WebSocket-Key",
      fastwebsockets::handshake::generate_key(),
    )
    .header("Sec-WebSocket-Version", "13")
    .body(Body::empty())?;

  let (ws, _) =
    fastwebsockets::handshake::client(&SpawnExecutor, req, tls_stream).await?;
  Ok(FragmentCollector::new(ws))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  let domain = "data-stream.binance.com";
  let mut ws = connect(domain).await?;

  loop {
    let msg = match ws.read_frame().await {
      Ok(msg) => msg,
      Err(e) => {
        println!("Error: {}", e);
        ws.write_frame(Frame::close_raw(vec![].into())).await?;
        break;
      }
    };

    match msg.opcode {
      OpCode::Text => {
        let payload =
          String::from_utf8(msg.payload.to_vec()).expect("Invalid UTF-8 data");
        // Normally deserialise from json here, print just to show it works
        println!("{:?}", payload);
      }
      OpCode::Close => {
        break;
      }
      _ => {}
    }
  }
  Ok(())
}
