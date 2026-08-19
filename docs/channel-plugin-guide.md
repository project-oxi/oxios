# Channel Plugin Guide

Oxios supports multiple communication channels (web, CLI, Telegram, etc.) through a plugin architecture. This guide covers how to integrate with Oxios via REST API, implement custom channels, subscribe to SSE event streams, and set up Telegram webhooks.

---

## Table of Contents

1. [REST API Integration](#rest-api-integration)
2. [Gateway Channel Trait](#gateway-channel-trait)
3. [SSE Event Stream Subscription](#sse-event-stream-subscription)
4. [Telegram Webhook Example](#telegram-webhook-example)

---

## REST API Integration

Oxios exposes a REST API on the configured host and port (default `127.0.0.1:4200`).

### Base URL

```
http://127.0.0.1:4200
```

### Health Check

```sh
curl http://127.0.0.1:4200/health
```

Response:
```json
{
  "status": "ok",
  "version": "0.2.0-alpha"
}
```

### Send a Message

Send a message to the Oxios gateway for processing:

```sh
curl -X POST http://127.0.0.1:4200/api/chat \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Build a TODO app with React and SQLite",
    "session_id": null
  }'
```

Response:
```json
{
  "response": "I will help you build a TODO app...",
  "evaluation_passed": true,
  "output": null
}
```


```sh
```

Response:
```json
[
  {
    "name": "my-project",
    "running": true,
  }
]
```


```sh
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-project"
  }'
```


```sh
  -H "Content-Type: application/json" \
  -d '{
    "command": ["cargo", "test"]
  }'
```

Response:
```json
{
  "stdout": "running 3 tests...\ntest result: ok. 3 passed",
  "stderr": "",
  "exit_code": 0
}
```

### Manage Programs

```sh
# List installed programs
curl http://127.0.0.1:4200/api/programs

# Install a program
curl -X POST http://127.0.0.1:4200/api/programs \
  -H "Content-Type: application/json" \
  -d '{
    "source": "https://example.com/my-program.tar.gz"
  }'

# Uninstall a program
curl -X DELETE http://127.0.0.1:4200/api/programs/my-program
```

### List Skills

```sh
curl http://127.0.0.1:4200/api/skills
```

---

## Gateway Channel Trait

All channels implement the `oxios_gateway::Channel` trait. To create a custom channel:

### Trait Definition

```rust
use async_trait::async_trait;
use oxios_gateway::{Channel, ChannelMessage, ChannelResponse};

#[derive(Debug)]
pub struct MyChannel {
    sender: tokio::sync::mpsc::Sender<ChannelMessage>,
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<ChannelMessage>>,
    capacity: usize,
}

impl MyChannel {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
        Self {
            sender,
            receiver: tokio::sync::Mutex::new(receiver),
            capacity,
        }
    }
}

#[async_trait]
impl Channel for MyChannel {
    fn name(&self) -> &str {
        "my-channel"
    }

    async fn send(&self, message: ChannelMessage) -> anyhow::Result<()> {
        self.sender.send(message).await
            .map_err(|e| anyhow::anyhow!("Channel send failed: {}", e))
    }

    async fn recv(&self) -> anyhow::Result<ChannelMessage> {
        let mut receiver = self.receiver.lock().await;
        receiver.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("Channel closed"))
    }

    fn capacity(&self) -> usize {
        self.capacity
    }
}
```

### Streaming Capability

The `Channel` trait exposes one capability flag channels must be aware of:

```rust
fn supports_streaming(&self) -> bool {
    false // default
}
```

When `true`, the gateway spawns a streaming-sink collector for your
channel's turns and forwards every LLM delta (text fragments, thinking
fragments, `stream_kind` control markers) as an individual
`OutgoingMessage` with `partial = Some(true)`. Only opt in if your surface
can render those fragments incrementally (e.g. a WebSocket UI appending to
an in-flight message). With the default `false`, your channel receives
exactly one message per turn — the complete terminal response. `web` opts
in; `telegram`, `cli`, and `remote` do not.

### Registration

Register your channel with the gateway at startup:

```rust
let my_channel = MyChannel::new(256);
kernel.gateway.register(Box::new(my_channel)).await;
```

### Channel Handle Pattern

For bidirectional communication, create a handle struct:

```rust
#[derive(Clone)]
pub struct MyChannelHandle {
    sender: tokio::sync::mpsc::Sender<ChannelMessage>,
}

impl MyChannelHandle {
    /// Send a message into the channel (from external source → Oxios).
    pub async fn send_message(
        &self,
        message: String,
        session_id: Option<String>,
    ) -> anyhow::Result<ChannelResponse> {
        let msg = ChannelMessage {
            channel: "my-channel".into(),
            message,
            session_id,
        };
        self.sender.send(msg).await?;
        // In a real implementation, wait for response via a reply channel
        Ok(ChannelResponse::default())
    }
}
```

---

## SSE Event Stream Subscription

Oxios exposes a Server-Sent Events (SSE) endpoint for real-time event streaming.

### Subscribe via curl

```sh
curl -N http://127.0.0.1:4200/api/events
```

### Event Format

Events are JSON envelopes streamed as SSE. Each carries `id`, `timestamp`, and a `type` discriminator:

```
event: agent_started
data: {"id":"<uuid>","timestamp":"2026-08-07T12:00:00Z","type":"agent_started","agent_id":"abc-123"}

event: agent_stopped
data: {"id":"<uuid>","timestamp":"...","type":"agent_stopped","agent_id":"abc-123","success":true}

event: agent_output
data: {"id":"<uuid>","timestamp":"...","type":"agent_output","session_id":"<sid>","agent_id":"abc-123"}
```

Event `type`s: `agent_created`, `agent_started`, `agent_stopped`, `agent_failed`, `message_received`, `agent_output`. Message *content* is omitted from `message_received` / `agent_output` (may be sensitive).

### Subscribe via JavaScript

```javascript
const eventSource = new EventSource('http://127.0.0.1:4200/api/events');

eventSource.addEventListener('agent.started', (event) => {
    const data = JSON.parse(event.data);
    console.log(`Agent ${data.agent_id} started: ${data.goal}`);
});

eventSource.addEventListener('agent.progress', (event) => {
    const data = JSON.parse(event.data);
    console.log(`Step ${data.step}: ${data.message}`);
});

eventSource.addEventListener('agent.completed', (event) => {
    const data = JSON.parse(event.data);
    console.log(`Agent completed: success=${data.success}`);
    eventSource.close();
});

eventSource.addEventListener('evaluation.result', (event) => {
    const data = JSON.parse(event.data);
    console.log(`Eval: score=${data.score}, passed=${data.mechanical_pass}`);
});

eventSource.onerror = () => {
    console.error('SSE connection error');
    eventSource.close();
};
```

### Subscribe via Python

```python
import sseclient
import requests
import json

response = requests.get(
    'http://127.0.0.1:4200/api/events',
    stream=True,
    headers={'Accept': 'text/event-stream'}
)

client = sseclient.SSEClient(response)

for event in client.events():
    if event.event == 'agent.completed':
        data = json.loads(event.data)
        print(f"Agent done: {data}")
        break
    elif event.event == 'agent.progress':
        data = json.loads(event.data)
        print(f"  Progress: {data.get('message', '...')}")
    else:
        print(f"  [{event.event}] {event.data}")
```

---

## Telegram Webhook Example

This example shows how to build a Telegram bot that forwards messages to Oxios and replies with the agent's response.

### Architecture

```
Telegram → HTTPS webhook → Your bot server → REST API → Oxios
                                                          ↓
Telegram ← HTTPS POST ← Your bot server ← REST API ← Oxios
```

### Prerequisites

1. A Telegram Bot Token (from [@BotFather](https://t.me/BotFather))
2. A publicly reachable HTTPS endpoint (e.g., via ngrok)
3. Oxios running on `127.0.0.1:4200`

### Server Implementation (Rust with Axum)

```rust
use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: u64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    chat: TelegramChat,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Serialize)]
struct TelegramReply {
    chat_id: i64,
    text: String,
}

#[derive(Debug, Serialize)]
struct OxiosMessage {
    channel: String,
    message: String,
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OxiosResponse {
    response: String,
}

async fn handle_telegram_webhook(
    State(state): State<Arc<AppState>>,
    Json(update): Json<TelegramUpdate>,
) -> Json<Option<TelegramReply>> {
    let msg = match update.message {
        Some(m) if m.text.is_some() => m,
        _ => return Json(None),
    };

    let text = msg.text.unwrap();

    // Forward to Oxios
    let client = reqwest::Client::new();
    let oxios_resp = client
        .post(format!("{}/api/chat", state.oxios_url))
        .json(&OxiosMessage {
            channel: "telegram".into(),
            message: text.clone(),
            session_id: Some(format!("telegram-{}", msg.chat.id)),
        })
        .send()
        .await;

    let reply_text = match oxios_resp {
        Ok(resp) => {
            if let Ok(oxios) = resp.json::<OxiosResponse>().await {
                oxios.response
            } else {
                "Oxios returned an unexpected response.".into()
            }
        }
        Err(e) => format!("Failed to reach Oxios: {}", e),
    };

    Json(Some(TelegramReply {
        chat_id: msg.chat.id,
        text: reply_text,
    }))
}

struct AppState {
    oxios_url: String,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        oxios_url: "http://127.0.0.1:4200".into(),
    });

    let app = Router::new()
        .route("/webhook/telegram", post(handle_telegram_webhook))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

### Register the Webhook

```sh
# Set the webhook URL (replace TOKEN and YOUR_DOMAIN)
curl -X POST "https://api.telegram.org/bot<TOKEN>/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://YOUR_DOMAIN/webhook/telegram"}'
```

### Verify the Webhook

```sh
curl "https://api.telegram.org/bot<TOKEN>/getWebhookInfo"
```

### Sending Replies

For longer-running tasks, you may want to send progress updates:

```sh
# Send an intermediate "thinking" message
curl -X POST "https://api.telegram.org/bot<TOKEN>/sendMessage" \
  -H "Content-Type: application/json" \
  -d '{
    "chat_id": 123456789,
    "text": "🤔 Processing your request..."
  }'
```

### SSE Integration for Progress Updates

Combine the Telegram bot with the SSE event stream to relay agent progress:

```python
# Example: Forward agent progress to Telegram
import asyncio
import aiohttp

BOT_TOKEN = "YOUR_BOT_TOKEN"
CHAT_ID = 123456789
OXIOS_URL = "http://127.0.0.1:4200"

async def send_telegram_message(text):
    url = f"https://api.telegram.org/bot{BOT_TOKEN}/sendMessage"
    async with aiohttp.ClientSession() as session:
        await session.post(url, json={"chat_id": CHAT_ID, "text": text})

async def watch_events():
    async with aiohttp.ClientSession() as session:
        async with session.get(f"{OXIOS_URL}/api/events") as resp:
            async for line in resp.content:
                decoded = line.decode().strip()
                if decoded.startswith("data:"):
                    data = json.loads(decoded[5:].strip())
                    event_type = decoded.get("event", "unknown")
                    if event_type == "agent.progress":
                        await send_telegram_message(f"⏳ {data.get('message', '...')}")
                    elif event_type == "agent.completed":
                        await send_telegram_message(f"✅ Task completed!")
```

---

## Summary

| Integration Method | Use Case |
|---|---|
| REST API | Simple request/response, scripting, CI/CD |
| Channel Trait | Full bidirectional custom channels |
| SSE Events | Real-time progress monitoring, dashboards |
| Telegram Webhook | Chat-based interaction with LLM agent |

For more details, see the [API reference](../surface/oxios-web/src/routes.rs) and [gateway crate](../crates/oxios-gateway/src/lib.rs).
