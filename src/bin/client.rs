use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use mini_redis::client;

#[derive(Debug)]
enum Command {
    Get {
        key: String,
        responder: Responder<Option<Bytes>>
    },
    Set {
        key: String,
        val: Bytes,
        responder: Responder<()>
    }
}

type Responder<T> = oneshot::Sender<mini_redis::Result<T>>;


#[tokio::main]
async fn main() {
    let (tx, mut rx) = mpsc::channel(32);

    // The `move` keyword is used to **move** ownership of `rx` into the task.
    let manager = tokio::spawn(async move {
        // Establish a connection to the server
        let mut client = client::connect("127.0.0.1:6379").await.unwrap();

        // Start receiving messages
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Get { key, responder } => {
                    let res = client.get(&key).await;
                    // Ignore errors
                    let _ = responder.send(res);
                }
                Command::Set { key, val, responder } => {
                    let res = client.set(&key, val).await;
                    // Ignore errors
                    let _ = responder.send(res);
                }
            }
        }
    });

    // The `Sender` handles are moved into the tasks. As there are two
    // tasks, we need a second `Sender`.
    let tx2 = tx.clone();

    // Spawn two tasks, one gets a key, the other sets a key
    let t1 = tokio::spawn(async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let cmd = Command::Get {
            key: "foo".to_string(),
            responder: resp_tx
        };

        tx.send(cmd).await.unwrap();
        
        let res = resp_rx.await;
        println!("GOT = {:?}", res);
    });

    let t2 = tokio::spawn(async move {
        let (resp_tx, resp_rx) = oneshot::channel();
        let cmd = Command::Set {
            key: "foo".to_string(),
            val: "bar".into(),
            responder: resp_tx,
        };

        // Send the SET request
        tx2.send(cmd).await.unwrap();

        // Await the response
        let res = resp_rx.await;
        println!("GOT = {:?}", res);
    });

    t1.await.unwrap();
    t2.await.unwrap();
    manager.await.unwrap();
}