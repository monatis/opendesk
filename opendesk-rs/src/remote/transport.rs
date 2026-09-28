//! WebSocket transport implementation for opendesk protocol and handshake.

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    tungstenite::Message,
    MaybeTlsStream, WebSocketStream,
};

use crate::protocol::handshake::Transport;

pub enum WsStream {
    Plain(WebSocketStream<TcpStream>),
    MaybeTls(WebSocketStream<MaybeTlsStream<TcpStream>>),
}

pub struct WebSocketTransport {
    inner: WsStream,
}

impl WebSocketTransport {
    pub fn new_plain(ws: WebSocketStream<TcpStream>) -> Self {
        Self {
            inner: WsStream::Plain(ws),
        }
    }

    pub fn new_tls(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self {
            inner: WsStream::MaybeTls(ws),
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        match &mut self.inner {
            WsStream::Plain(ws) => ws.close(None).await.map_err(|e| anyhow!(e)),
            WsStream::MaybeTls(ws) => ws.close(None).await.map_err(|e| anyhow!(e)),
        }
    }

    pub async fn send_text(&mut self, text: &str) -> Result<()> {
        match &mut self.inner {
            WsStream::Plain(ws) => ws
                .send(Message::Text(text.to_string().into()))
                .await
                .map_err(|e| anyhow!(e)),
            WsStream::MaybeTls(ws) => ws
                .send(Message::Text(text.to_string().into()))
                .await
                .map_err(|e| anyhow!(e)),
        }
    }

    pub async fn recv_text(&mut self) -> Result<String> {
        loop {
            let msg = match &mut self.inner {
                WsStream::Plain(ws) => ws.next().await,
                WsStream::MaybeTls(ws) => ws.next().await,
            };

            match msg {
                Some(Ok(Message::Text(t))) => return Ok(t.to_string()),
                Some(Ok(Message::Ping(p))) => {
                    let _ = match &mut self.inner {
                        WsStream::Plain(ws) => ws.send(Message::Pong(p)).await,
                        WsStream::MaybeTls(ws) => ws.send(Message::Pong(p)).await,
                    };
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) | None => {
                    return Err(anyhow!("websocket closed"));
                }
                Some(Ok(Message::Binary(_))) => {
                    return Err(anyhow!("expected text frame, got binary"));
                }
                Some(Err(e)) => return Err(anyhow!("websocket read error: {}", e)),
                _ => continue,
            }
        }
    }
}

impl Transport for WebSocketTransport {
    async fn send(&mut self, data: &[u8]) -> Result<()> {
        match &mut self.inner {
            WsStream::Plain(ws) => ws
                .send(Message::Binary(data.to_vec().into()))
                .await
                .map_err(|e| anyhow!("websocket send error: {}", e)),
            WsStream::MaybeTls(ws) => ws
                .send(Message::Binary(data.to_vec().into()))
                .await
                .map_err(|e| anyhow!("websocket send error: {}", e)),
        }
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        loop {
            let msg = match &mut self.inner {
                WsStream::Plain(ws) => ws.next().await,
                WsStream::MaybeTls(ws) => ws.next().await,
            };

            match msg {
                Some(Ok(Message::Binary(b))) => return Ok(b.to_vec()),
                Some(Ok(Message::Ping(p))) => {
                    let _ = match &mut self.inner {
                        WsStream::Plain(ws) => ws.send(Message::Pong(p)).await,
                        WsStream::MaybeTls(ws) => ws.send(Message::Pong(p)).await,
                    };
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) | None => {
                    return Err(anyhow!("websocket closed by peer"));
                }
                Some(Ok(Message::Text(t))) => {
                    return Err(anyhow!("unexpected text message: {}", t));
                }
                Some(Err(e)) => return Err(anyhow!("websocket read error: {}", e)),
                _ => continue,
            }
        }
    }
}
