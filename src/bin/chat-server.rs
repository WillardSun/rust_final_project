use futures::{SinkExt, StreamExt};
use tokio::net::{TcpStream, TcpListener};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};
use tokio::sync::broadcast::{self, Sender};
use std::collections::{HashSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};

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

#[derive(Clone)]
struct Names {
    existing: Arc<Mutex<HashSet<String>>>
}

impl Names {
    fn new () -> Self {
        return Names { existing: Arc::new(Mutex::new(HashSet::new())) }
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
    tx: Sender<String>,
    users: HashSet<String>
}

impl Room {
    fn new() -> Self {
        let (tx, _) = broadcast::channel(32);
        let users = HashSet::new();
        return Self {
            tx: tx,
            users: users
        }
    }
}

#[derive(Clone)]
struct Rooms(Arc<RwLock<HashMap<String, Room>>>);
impl Rooms{
    fn new() -> Self {
        return Self(Arc::new(RwLock::new(HashMap::new())));
    }
    fn join(&self, room_name: &str, user_name: &str) -> Sender<String> {
        let mut write_guard = self.0.write().unwrap();
        let room = write_guard.entry(room_name.to_owned()).or_insert(Room::new());
        room.users.insert(user_name.to_owned());
        return room.tx.clone();
    }
    fn leave(&self, room_name: &str, user_name: &str) {
        let mut write_guard = self.0.write().unwrap();
        let mut delete_room = false;
        if let Some(room) = write_guard.get_mut(room_name){
            room.users.remove(user_name);
            delete_room = room.tx.receiver_count() <= 1;
        }
        if delete_room {
            write_guard.remove(room_name);
        }
    }
    fn change(&self, prev_room: &str, next_room: &str,  user_name: &str) -> Sender<String> {
        self.leave(prev_room, user_name);
        return self.join(next_room, user_name);
    }
    fn change_name(&self, room_name: &str, old_name: &str, new_name: &str) {
        let mut write_guard = self.0.write().unwrap();
        if let Some(room) = write_guard.get_mut(room_name) {
            room.users.remove(old_name);
            room.users.insert(new_name.to_owned());
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
    let listener = TcpListener::bind("127.0.0.1:6142").await?;
    let rooms = Rooms::new();
    let names = Names::new();
    loop {
        let (socket, _) = listener.accept().await?;
        println!("New socket connected: {:?}", socket.peer_addr()?);
        let names_clone = names.clone();
        let rooms_clone = rooms.clone();
        tokio::spawn (async move {
            if let Err(e) = process(socket, rooms_clone, names_clone).await {
                println!("Connction error: {}", e);
            }
        });
    }
}

async fn process(mut socket: TcpStream, rooms: Rooms, existing: Names) -> anyhow::Result<()> {
    let (rd, wr) = socket.split();
    let mut stream = FramedRead::new(rd, LinesCodec::new());
    let mut sink = FramedWrite::new(wr, LinesCodec::new());

    let mut user_name = existing.get_unique();

    let mut room_name = MAIN.to_owned();
    let mut tx = rooms.join(&room_name, &user_name);
    let mut rx = tx.subscribe();

    let _ = tx.send(format!("{:?} has joined the chat.", user_name));
    sink.send(HELP_MSG).await?;
    let result: anyhow::Result<()> = loop {
        tokio::select! {
            user_msg = stream.next() => {
                let user_msg = match user_msg {
                    Some(msg) => b!(msg),
                    None => break Ok(())
                };
                if user_msg.starts_with("/join") {
                    let mut itr = user_msg.split_ascii_whitespace();
                    itr.next();
                    let new_room = itr
                        .collect::<Vec<&str>>()
                        .join(" ")
                        .to_owned();
                    if new_room == room_name {
                        b!(sink.send("You are already in this room.").await);
                        continue;
                    }
                    b!(tx.send(format!("{user_name} has left {room_name}.")));
                    tx = rooms.change(&room_name, &new_room, &user_name);
                    rx = tx.subscribe();
                    room_name = new_room;
                    b!(tx.send(format!("{user_name} has joined {room_name}.")));
                }
                else if user_msg.starts_with("/name") {
                    let mut itr = user_msg.split_ascii_whitespace();
                    itr.next();
                    let new_name = itr
                        .collect::<Vec<&str>>()
                        .join(" ")
                        .to_owned();
                    let changed_name = existing.insert(new_name.clone());
                    if changed_name {
                        existing.remove(&user_name);
                        rooms.change_name(&room_name, &user_name, &new_name);
                        b!(tx.send(format!("{user_name} is now {new_name}")));
                        b!(tx.send(format!("Current names in room: {:?}", rooms.list_users(&room_name))));
                        user_name = new_name;
                    }
                    else {
                        b!(sink.send("Sorry, that name is taken.").await);
                    }
                }
                else if user_msg.starts_with("/allusers") {
                    b!(sink.send(format!("All users: {:?}", existing.get_existing())).await);
                }
                else if user_msg.starts_with("/users") {
                    b!(sink.send(format!("Users in current room: {:?}", rooms.list_users(&room_name))).await)
                }
                else if user_msg.starts_with("/rooms") {
                    let rooms_list = rooms.get_existing()
                        .into_iter()
                        .map(|(name, count)| format!("{name} ({count})"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    b!(sink.send(format!("Current rooms: {rooms_list}")).await);
                }
                else if user_msg.starts_with("/help") {
                    b!(sink.send(HELP_MSG).await);
                }
                else if user_msg.starts_with("/quit") {
                    break Ok(());
                }
                else {
                    b!(tx.send(format!("{user_name}: {user_msg}")));
                }
            },
            peer_msg = rx.recv() => {
                let peer_msg = b!(peer_msg);
                b!(sink.send(peer_msg).await);
            },
        }
    };
    let _ = tx.send(format!("{:?} has left the chat.", user_name));
    existing.remove(&user_name);
    rooms.leave(&room_name, &user_name);
    result
}