use axum::extract::{
    State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::response::IntoResponse;
use axum::{Router, routing};
use bytes::Bytes;
use chrono::{TimeZone, Utc};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio::net::TcpListener;
use tokio::sync::broadcast::{self, Sender};
use tokio::time;

use rust_final_project::random_name;

macro_rules! b {
    ($result:expr) => {
        match $result {
            Ok(ok) => ok,
            Err(err) => break Err(err.into()),
        }
    };
}

const HELP_MSG: &str = include_str!("help.txt");
const MAIN: &str = "main";

#[derive(Clone, Debug, serde::Serialize)]
struct ChatMessage {
    message: String,
    timestamp: i64,
}

impl ChatMessage {
    fn new(message: String) -> Self {
        ChatMessage {
            message,
            timestamp: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
        }
    }
}

#[derive(Clone, Debug)]
struct Names {
    existing: Arc<Mutex<HashSet<String>>>,
}

impl Names {
    fn new() -> Self {
        return Names {
            existing: Arc::new(Mutex::new(HashSet::new())),
        };
    }
    fn insert(&self, str: String) -> bool {
        return self.existing.lock().unwrap().insert(str);
    }
    fn remove(&self, str: &String) -> bool {
        return self.existing.lock().unwrap().remove(str);
    }
    fn get_unique(&self) -> String {
        let mut new_str = random_name();
        while !self.insert(new_str.clone()) {
            new_str = random_name();
        }
        return new_str;
    }
    fn get_existing(&self) -> Vec<String> {
        let mut names = Vec::new();
        for s in self.existing.lock().unwrap().iter() {
            names.push(s.clone());
        }
        return names;
    }
}

struct Room {
    tx: Sender<ChatMessage>,
    users: HashSet<String>,
}

impl Room {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(32);
        let users = HashSet::new();
        return Self {
            tx: tx,
            users: users,
        };
    }
}

#[derive(Clone)]
struct Rooms(Arc<RwLock<HashMap<String, Room>>>);
impl Rooms {
    fn new() -> Self {
        return Self(Arc::new(RwLock::new(HashMap::new())));
    }
    fn join(&self, room_name: &str, user_name: &str) -> Sender<ChatMessage> {
        let mut write_guard = self.0.write().unwrap();
        let room = write_guard
            .entry(room_name.to_owned())
            .or_insert(Room::new());
        room.users.insert(user_name.to_owned());
        return room.tx.clone();
    }
    fn leave(&self, room_name: &str, user_name: &str) {
        let mut write_guard = self.0.write().unwrap();
        let mut delete_room = false;
        if let Some(room) = write_guard.get_mut(room_name) {
            room.users.remove(user_name);
            delete_room = room.tx.receiver_count() <= 1;
        }
        if delete_room {
            write_guard.remove(room_name);
        }
    }
    fn change(&self, prev_room: &str, next_room: &str, user_name: &str) -> Sender<ChatMessage> {
        self.leave(prev_room, user_name);
        return self.join(next_room, user_name);
    }
    fn change_name(&self, room_name: &str, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        let mut write_guard = self.0.write().unwrap();
        if let Some(room) = write_guard.get_mut(room_name) {
            room.users.remove(old_name);
            room.users.insert(new_name.to_owned());
            Ok(())
        } else {
            Err(anyhow::anyhow!("User not found"))
        }
    }
    fn change_room_name(&self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        let mut write_guard = self.0.write().unwrap();
        if let Some(room) = write_guard.remove(old_name) {
            write_guard.insert(new_name.to_owned(), room);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Room not found"))
        }
    }
    fn list_users(&self, room_name: &str) -> Vec<String> {
        let mut users = Vec::new();
        let read_guard = self.0.read().unwrap();
        for user in read_guard.get(room_name).unwrap().users.iter() {
            users.push(user.to_owned());
        }
        users
    }
    fn get_existing(&self) -> Vec<(String, usize)> {
        let mut rooms = Vec::new();
        for s in self.0.read().unwrap().iter() {
            rooms.push((s.0.clone(), s.1.tx.receiver_count()));
        }
        rooms.sort_by(|a, b| {
            use std::cmp::Ordering::*;
            match b.1.cmp(&a.1) {
                Equal => a.0.cmp(&b.0),
                ordering => ordering,
            }
        });
        return rooms;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("0.0.0.0:6142").await?;
    let rooms = Rooms::new();
    let names = Names::new();

    let app = Router::new()
        .route("/ws", routing::any(ws_handler))
        .with_state((rooms, names));

    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State((rooms, names)): State<(Rooms, Names)>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = process(socket, rooms, names).await {
            eprintln!("connection error: {e}");
        }
    })
}

async fn process(mut socket: WebSocket, rooms: Rooms, existing: Names) -> anyhow::Result<()> {
    let mut user_name = existing.get_unique();
    let mut room_name = MAIN.to_owned();
    let mut tx = rooms.join(&room_name, &user_name);
    let mut rx = tx.subscribe();

    let _ = tx.send(ChatMessage::new(format!(
        "{user_name} has joined the chat."
    )));

    let _ = socket.send(Message::Text(HELP_MSG.into())).await;

    let mut heartbeat = time::interval(time::Duration::from_secs(15));
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    // main loop returns Result so `b!` can break with Err
    let result: anyhow::Result<()> = loop {
        tokio::select! {
            msg = socket.recv() => {
                let msg = match msg {
                    Some(msg) => b!(msg),
                    None => break Ok(()), // client closed
                };

                let user_msg = match msg {
                    Message::Text(t) => t,
                    Message::Binary(_) => continue,
                    Message::Ping(_) => continue,
                    Message::Pong(_) => {
                        b!(socket.send(Message::Text(format!("Received pong from {}", user_name).into())).await);
                        continue;
                    }
                    Message::Close(_) => break Ok(()),
                };

                if user_msg.starts_with("/join") {

                    let mut itr = user_msg.split_ascii_whitespace();
                    itr.next();
                    let new_room = itr.collect::<Vec<&str>>().join(" ");

                    if new_room == room_name {
                        b!(socket.send(Message::Text("You are already in this room.".into())).await);
                        continue;
                    }

                    b!(tx.send(ChatMessage::new(format!("{user_name} has left {room_name}."))));
                    tx = rooms.change(&room_name, &new_room, &user_name);
                    rx = tx.subscribe();
                    room_name = new_room;
                    b!(tx.send(ChatMessage::new(format!("{user_name} has joined {room_name}."))));
                }
                else if user_msg.starts_with("/name") {
                    let mut itr = user_msg.split_ascii_whitespace();
                    itr.next();
                    let new_name = itr.collect::<Vec<&str>>().join(" ");
                    let changed_name = existing.insert(new_name.clone());
                    if changed_name {
                        existing.remove(&user_name);
                        b!(rooms.change_name(&room_name, &user_name, &new_name));
                        b!(tx.send(ChatMessage::new(format!("{user_name} is now {new_name}"))));
                        b!(tx.send(ChatMessage::new(format!("Current names in room: {:?}", rooms.list_users(&room_name)))));
                        user_name = new_name;
                    }
                    else {
                        b!(socket.send(Message::Text("Sorry, that name is taken.".into())).await);
                    }
                }
                else if user_msg.starts_with("/allusers") {
                    let users_str = format!("All users: {:?}", existing.get_existing());
                    b!(socket.send(Message::Text(users_str.into())).await);
                }
                else if user_msg.starts_with("/users") {
                    let users_str = format!("Users in current room: {:?}", rooms.list_users(&room_name));
                    b!(socket.send(Message::Text(users_str.into())).await);
                }
                else if user_msg.starts_with("/rooms") {
                    let rooms_list = rooms
                        .get_existing()
                        .into_iter()
                        .map(|(name, count)| format!("{name} ({count})"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let rooms_str = format!("Current rooms: {rooms_list}");
                    b!(socket.send(Message::Text(rooms_str.into())).await);
                }
                else if user_msg.starts_with("/renameroom ") {
                    let mut itr = user_msg.split_ascii_whitespace();
                    itr.next();
                    let new_room_name = itr.collect::<Vec<&str>>().join(" ");

                    if rooms.0.read().unwrap().contains_key(&new_room_name) {
                        b!(socket.send(Message::Text("Room name already exists.".into())).await);
                        continue;
                    }

                    b!(rooms.change_room_name(&room_name, &new_room_name));
                    b!(tx.send(ChatMessage::new(format!("Room {room_name} has been renamed to {new_room_name}."))));
                    room_name = new_room_name;
                }
                else if user_msg.starts_with("/help") {
                    b!(socket.send(Message::Text(HELP_MSG.into())).await);
                }
                else if user_msg.starts_with("/quit") {
                    break Ok(());
                }
                else {
                    b!(tx.send(ChatMessage::new(format!("{user_name}: {user_msg}"))));
                }
            },

            peer_msg = rx.recv() => {
                let peer_msg = b!(peer_msg);
                // Send machine-readable JSON so load tests can parse timestamps reliably
                match serde_json::to_string(&peer_msg) {
                    Ok(json) => {
                        b!(socket.send(Message::Text(json.into())).await);
                    }
                    Err(_) => {
                        // fallback to formatted text (timestamp is milliseconds)
                        let ts = peer_msg.timestamp as i64;
                        let secs = ts / 1000;
                        let nsecs = ((ts % 1000) * 1_000_000) as u32;
                        let dt = Utc.timestamp_opt(secs, nsecs).single().unwrap();
                        let formatted_date = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                        let millis = (ts % 1000).abs();
                        let formatted_time = format!("{}.{} UTC", formatted_date, format!("{:03}", millis));
                        let output_msg = format!("[{}] {}", formatted_time, peer_msg.message);
                        b!(socket.send(Message::Text(output_msg.into())).await);
                    }
                }
            },
            _ = heartbeat.tick() => {
                b!(socket.send(Message::Ping(Bytes::from("ping"))).await);
            }
        }
    };

    let _ = tx.send(ChatMessage::new(format!("{user_name} has left the chat.")));
    existing.remove(&user_name);
    rooms.leave(&room_name, &user_name);
    result
}
