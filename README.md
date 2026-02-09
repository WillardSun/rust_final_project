# Chatski

## Multi-Room WebSocket Chat Server

A real-time chat server built with Rust, Axum, and WebSockets supporting multiple chat rooms and concurrent users.

---

## Features

- **WebSocket-based communication** - Real-time bidirectional messaging
- **Multiple chat rooms** - Users can create and switch between rooms dynamically
- **User management** - Unique usernames with conflict prevention
- **Room management** - List, join, rename, and auto-cleanup empty rooms
- **Web client** - Browser-based HTML/JavaScript client interface
- **Concurrent handling** - Tokio async runtime with broadcast channels

---

## Architecture

### Core Components

**`chat-server.rs`** - Main server application
- Listens on `0.0.0.0:6142` (WebSocket endpoint: `/ws`)
- Uses Axum web framework for HTTP/WebSocket routing
- Manages shared state: `Rooms` and `Names`

**`Names`** - Thread-safe username registry
- `Arc<Mutex<HashSet<String>>>` for concurrent access
- Auto-generates unique random names on connection
- Prevents duplicate usernames

**`Rooms`** - Thread-safe room collection
- `Arc<RwLock<HashMap<String, Room>>>` for read-heavy operations
- Each `Room` contains:
  - `tx: Sender<String>` - Tokio broadcast channel for messages
  - `users: HashSet<String>` - Active users in the room
- Auto-cleanup: removes rooms when last user leaves

**Message Flow**
```
WebSocket Client → process() loop → tokio::select! {
    ├─ socket.recv() → parse command/message → broadcast via tx.send()
    └─ rx.recv()     → receive broadcast      → socket.send()
}
```

---

## Commands

| Command | Description |
|---------|-------------|
| `/name [NAME]` | Change your username |
| `/join [ROOM]` | Switch to a different room (creates if doesn't exist) |
| `/renameroom [NAME]` | Rename the current room |
| `/users` | List users in current room |
| `/allusers` | List all connected users |
| `/rooms` | List all active rooms with user counts |
| `/help` | Display help message |
| `/quit` | Disconnect from server |

---

## Running the Server

```bash
# Build and run the chat server
cargo run --bin chat-server

# Server starts on ws://localhost:6142/ws
```

---

## Client Usage

### Web Browser Client

1. Open `index.html` in a browser:
   ```bash
   # Option 1: Direct file access
   xdg-open index.html
   
   # Option 2: Use a local HTTP server (recommended)
   python3 -m http.server 8000
   # Then visit: http://localhost:8000/index.html
   ```

2. The client connects to `ws://localhost:6142/ws` by default
3. Edit `WS_URL` in `index.html` to connect to remote servers

Note: `index.html` is now implemented to auto-select the correct WebSocket protocol (wss/ws) based on the page's protocol. You do not need to manually edit `WS_URL`.

---

## Running with Cloudflared tunnels (quick)

If you want to expose your local chat server and static site to the public internet for testing, you can use `cloudflared`.

I included a helper script: `scripts/start_tunnels.sh` which does the following:

- Starts a local static file server on port `8000` (if not already running).
- Launches two ephemeral Cloudflared tunnels:
  - one to `http://localhost:6142` (chat WebSocket server)
  - one to `http://localhost:8000` (static files)
- Parses the ephemeral `trycloudflare.com` URLs printed by Cloudflared and updates `index.html` to use the chat tunnel as the `WS_URL` (`wss://<host>/ws`).

Usage:

```bash
chmod +x scripts/start_tunnels.sh
./scripts/start_tunnels.sh
```

The script prints the public static URL (open this in your browser) and the WebSocket (WSS) URL it set in `index.html`.

WSL note

- If you run the repo inside WSL but installed `cloudflared` on Windows, either install `cloudflared` inside WSL (recommended) or call the Windows `cloudflared.exe` from WSL using the mounted path (e.g. `/mnt/c/Users/You/Downloads/cloudflared.exe`). See the script and earlier notes for details.

---

## Project Structure

```
rust-final-project/
├── src/
│   ├── lib.rs              # random_name() utility
│   ├── people.rs           # (unused module)
│   └── bin/
│       ├── chat-server.rs  # Main WebSocket server
│       ├── client.rs       # (alternative client)
│       ├── main.rs         # (alternative entry point)
│       └── help.txt        # Command reference
├── index.html              # Web client UI
├── scripts/
│   └── start_tunnels.sh    # helper to expose ports via cloudflared
├── Cargo.toml              # Dependencies
└── README.md               # This file
```

---

## Dependencies

- **tokio** - Async runtime (with "full" features)
- **axum** - Web framework with WebSocket support
- **anyhow** - Error handling
- **fastrand** - Random name generation
- **futures-util** - Stream/sink utilities

---

## Development Notes

- Uses Rust 2024 edition (preview)
- Error handling via `anyhow` and custom `b!` macro
- Broadcast channels handle fan-out to multiple clients
- RwLock allows concurrent reads for room lookups
- WebSocket messages use `axum::extract::ws::Message` enum

---

## Future Enhancements

- Coming soon
