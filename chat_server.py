import asyncio
import json
import time
import random
import websockets
from collections import defaultdict

HELP_MSG = """
Available commands:
/join <room_name> - Join a different room
/name <new_name> - Change your name
/users - List users in current room
/allusers - List all users
/rooms - List all rooms
/renameroom <new_name> - Rename current room
/help - Show this help message
/quit - Disconnect
"""

class ChatMessage:
    def __init__(self, message):
        self.message = message
        self.timestamp = int(time.time() * 1000)

    def to_json(self):
        return json.dumps({
            "message": self.message,
            "timestamp": self.timestamp
        })

class Names:
    def __init__(self):
        self.existing = set()

    def insert(self, name):
        if name in self.existing:
            return False
        self.existing.add(name)
        return True

    def remove(self, name):
        if name in self.existing:
            self.existing.remove(name)
            return True
        return False

    def get_unique(self):
        while True:
            name = f"User{random.randint(1000, 9999)}"
            if self.insert(name):
                return name

    def get_existing(self):
        return list(self.existing)

class Room:
    def __init__(self):
        self.users = set() # Set of (websocket, name) tuples? Or just names?
                           # We need websockets to broadcast.
        self.connections = set() # Set of websocket objects

    def broadcast(self, message_json):
        # Create tasks for sending to avoid blocking
        # In a real high-perf scenario we might want to batch or use more advanced techniques
        # but this matches the Rust "broadcast" channel concept roughly.
        if not self.connections:
            return
        
        # websockets.broadcast is efficient
        websockets.broadcast(self.connections, message_json)

class Rooms:
    def __init__(self):
        self.rooms = defaultdict(Room) # room_name -> Room

    def join(self, room_name, websocket):
        self.rooms[room_name].connections.add(websocket)

    def leave(self, room_name, websocket):
        if room_name in self.rooms:
            self.rooms[room_name].connections.discard(websocket)
            if not self.rooms[room_name].connections:
                del self.rooms[room_name]

    def change(self, prev_room, next_room, websocket):
        self.leave(prev_room, websocket)
        self.join(next_room, websocket)

    def change_room_name(self, old_name, new_name):
        if old_name in self.rooms:
            self.rooms[new_name] = self.rooms.pop(old_name)
            return True
        return False

    def list_users(self, room_name, user_map):
        # user_map maps websocket -> name
        if room_name not in self.rooms:
            return []
        return [user_map[ws] for ws in self.rooms[room_name].connections if ws in user_map]

    def get_existing(self):
        return [(name, len(r.connections)) for name, r in self.rooms.items()]

# Global state
rooms = Rooms()
names = Names()
# Map websocket -> current_room_name
ws_rooms = {}
# Map websocket -> user_name
ws_names = {}

async def process(websocket):
    user_name = names.get_unique()
    room_name = "main"
    
    ws_names[websocket] = user_name
    ws_rooms[websocket] = room_name
    
    rooms.join(room_name, websocket)
    
    # Notify join
    join_msg = ChatMessage(f"{user_name} has joined the chat.").to_json()
    rooms.rooms[room_name].broadcast(join_msg)
    
    try:
        await websocket.send(HELP_MSG)
        
        async for message in websocket:
            # message is text (or bytes, but we expect text)
            
            # Check for commands
            if message.startswith("/"):
                parts = message.split()
                cmd = parts[0]
                args = parts[1:]
                
                if cmd == "/join":
                    new_room = " ".join(args)
                    if not new_room:
                        continue
                        
                    if new_room == room_name:
                        await websocket.send("You are already in this room.")
                        continue
                        
                    # Notify leave old
                    leave_msg = ChatMessage(f"{user_name} has left {room_name}.").to_json()
                    rooms.rooms[room_name].broadcast(leave_msg)
                    
                    # Switch
                    rooms.change(room_name, new_room, websocket)
                    room_name = new_room
                    ws_rooms[websocket] = room_name
                    
                    # Notify join new
                    join_msg = ChatMessage(f"{user_name} has joined {room_name}.").to_json()
                    rooms.rooms[room_name].broadcast(join_msg)
                    
                elif cmd == "/name":
                    new_name = " ".join(args)
                    if not new_name:
                        continue
                        
                    if names.insert(new_name):
                        names.remove(user_name)
                        old_name = user_name
                        user_name = new_name
                        ws_names[websocket] = user_name
                        
                        rename_msg = ChatMessage(f"{old_name} is now {new_name}").to_json()
                        rooms.rooms[room_name].broadcast(rename_msg)
                        
                        # Send current names list to the user (as per Rust impl)
                        current_users = rooms.list_users(room_name, ws_names)
                        list_msg = ChatMessage(f"Current names in room: {current_users}").to_json()
                        # Rust sends this to the *broadcast channel*? 
                        # "b!(tx.send(ChatMessage::new(format!("Current names in room: {:?}", rooms.list_users(&room_name)))));"
                        # Yes, it broadcasts the list to everyone in the room.
                        rooms.rooms[room_name].broadcast(list_msg)
                        
                    else:
                        await websocket.send("Sorry, that name is taken.")
                        
                elif cmd == "/allusers":
                    all_users = names.get_existing()
                    await websocket.send(f"All users: {all_users}")
                    
                elif cmd == "/users":
                    room_users = rooms.list_users(room_name, ws_names)
                    await websocket.send(f"Users in current room: {room_users}")
                    
                elif cmd == "/rooms":
                    room_list = rooms.get_existing()
                    # Sort by count desc, then name asc
                    room_list.sort(key=lambda x: (-x[1], x[0]))
                    formatted = ", ".join([f"{name} ({count})" for name, count in room_list])
                    await websocket.send(f"Current rooms: {formatted}")
                    
                elif cmd == "/renameroom":
                    new_room_name = " ".join(args)
                    if not new_room_name:
                        continue
                        
                    if new_room_name in rooms.rooms:
                        await websocket.send("Room name already exists.")
                        continue
                        
                    rooms.change_room_name(room_name, new_room_name)
                    # Update room_name for all users in this room?
                    # Rust impl:
                    # "b!(rooms.change_room_name(&room_name, &new_room_name));"
                    # "b!(tx.send(ChatMessage::new(format!("Room {room_name} has been renamed to {new_room_name}."))));"
                    # "room_name = new_room_name;" -> This only updates local variable for the current user loop.
                    # Wait, in Rust `rooms` is shared. `change_room_name` updates the HashMap key.
                    # But other users' `room_name` local variable in their loop is NOT updated.
                    # However, they hold a `tx` (Sender) to the room.
                    # In Rust `Room` struct doesn't store its name. The `HashMap` maps Name -> Room.
                    # If we change the key in HashMap, the `Sender` held by other users is still valid and points to the same channel.
                    # So they keep receiving messages.
                    # BUT, if they try to `/join` or do something using their local `room_name` variable, it might be stale?
                    # In Rust: `room_name` is a local variable in `process`.
                    # If I rename room A to B. My `room_name` becomes B.
                    # Another user in A still has `room_name` = A.
                    # If they send a message, it goes to `tx` (which is still valid).
                    # If they try `/join C`. `rooms.change(A, C, ...)` -> `leave(A)`.
                    # `leave` looks up A in map. A is gone!
                    # So `leave` fails to remove them from the room struct?
                    # Rust `leave`: `if let Some(room) = write_guard.get_mut(room_name)`.
                    # If room_name is stale, it returns None.
                    # So the user is "stuck" in the old room struct (which is now under new name)?
                    # Actually `leave` just removes user from `room.users`.
                    # If `leave` fails, the user remains in `room.users`.
                    # This seems like a bug in the Rust implementation or a subtle behavior.
                    # For Python, we need to replicate this or handle it.
                    # In Python `ws_rooms` map stores the room name.
                    # If we rename, we should probably update `ws_rooms` for all users in that room.
                    
                    rename_msg = ChatMessage(f"Room {room_name} has been renamed to {new_room_name}.").to_json()
                    rooms.rooms[new_room_name].broadcast(rename_msg)
                    
                    # Update all users in this room to know the new name
                    # This fixes the stale name issue for everyone
                    for ws in rooms.rooms[new_room_name].connections:
                        ws_rooms[ws] = new_room_name
                    
                    room_name = new_room_name
                    
                elif cmd == "/help":
                    await websocket.send(HELP_MSG)
                    
                elif cmd == "/quit":
                    break
                    
                else:
                    # Unknown command, treat as message?
                    # Rust: "b!(tx.send(ChatMessage::new(format!("{user_name}: {user_msg}"))));"
                    # It treats everything else as a message.
                    msg = ChatMessage(f"{user_name}: {message}").to_json()
                    rooms.rooms[room_name].broadcast(msg)
            else:
                # Normal message
                msg = ChatMessage(f"{user_name}: {message}").to_json()
                rooms.rooms[room_name].broadcast(msg)
                
    except websockets.exceptions.ConnectionClosed:
        pass
    finally:
        # Cleanup
        if websocket in ws_names:
            names.remove(ws_names[websocket])
            del ws_names[websocket]
        
        if websocket in ws_rooms:
            current_room = ws_rooms[websocket]
            rooms.leave(current_room, websocket)
            
            leave_msg = ChatMessage(f"{user_name} has left the chat.").to_json()
            # If room still exists (might have been deleted if empty)
            if current_room in rooms.rooms:
                rooms.rooms[current_room].broadcast(leave_msg)
            
            del ws_rooms[websocket]

async def main():
    print("Starting Python Chat Server on 0.0.0.0:6142")
    async with websockets.serve(process, "0.0.0.0", 6142):
        await asyncio.Future()  # run forever

if __name__ == "__main__":
    asyncio.run(main())
