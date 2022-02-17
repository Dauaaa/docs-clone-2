#[allow(unused)]
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, AsyncWrite, BufReader},
    net::TcpListener,
    sync::{broadcast, mpsc, oneshot},
};

#[allow(unused)]
use std::{
    net::SocketAddr,
    sync::Arc
};
use parking_lot::RwLock;

#[derive(Debug)]
struct State {
    doc: String,
    commits: Vec<String>,
}

enum Instructions {
    PushCommit(String, SocketAddr),
    GetCommitSince(oneshot::Sender<Vec<String>>, usize),
    GetDocument(oneshot::Sender<(String, usize)>),
}
impl State {
    fn new() -> Self { State { doc: String::from("default arquivo\n\n\n\ndrip o.O"), commits: vec!["".to_string()]} }
    fn apply_patch(&mut self, patch: &str) {
        //TODO apply patch
        self.doc.push_str(patch);
    }
    fn push_commit(&mut self, commit: &str) {
        self.commits.push(commit.to_string());
        self.apply_patch(commit);
    }
    fn get_version(&self) -> usize { self.commits.len() }
    fn get_doc(&self) -> &str { &self.doc }
    fn update_from(&self, current: usize) -> Vec<String> {
        println!("CURRENT: {}, LEN {}", current, self.commits.len());
        self.commits[current..].to_vec()
    }
    // fn get_commit(&self, idx: usize) -> Result<&str> { &self.commits[idx]? }
}

// struct Client {
//     synced: bool,
//     commits: Vec<usize>,
// }

#[allow(unused)]
#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("localhost:8080").await.unwrap();

    let document: RwLock<State> = RwLock::new(State::new());

    // this channel allows server to send new commits to user
    let (tx_update, mut rx_update) = broadcast::channel::<String>(250);
    // this channel allows users to commit changes to server
    let (tx_edit, mut rx_edit) = mpsc::channel::<Instructions>(250);

    let tx_update_backup = tx_update.clone();
    // this process will takle all state change 
    // and requests
    // this will help by making arc unecessary
    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = rx_edit.recv() => {
                    match result.unwrap() {
                        Instructions::GetDocument(tx) => {
                            println!("GET DOCUMENT");
                            tx.send((String::from(document.read().get_doc()), document.read().get_version()));
                        }
                        Instructions::GetCommitSince(tx, ver) => {
                            println!("GET COMMIT SINCE");
                            tx.send(document.read().update_from(ver));
                        }
                        Instructions::PushCommit(commit, addr) => {
                            println!("PUSH COMMIT");
                            document.write().push_commit(&commit);
                            tx_update.send(commit.clone()).unwrap();
                        }
                    }
                }
            }
        }
    });

    // loop where all clients will be instantiated
    loop {
        let (mut socket, addr) = listener.accept().await.unwrap();
        println!("Copy edit tx");
        // allow user to send commits
        let tx_edit = tx_edit.clone();
        println!("Copy update rx");
        // subscribe user to doc state channel
        let mut rx_update = tx_update_backup.subscribe();

        tokio::spawn(async move {
            let (read, mut writer) = socket.split();

            // reader variable is an observable state that 
            // gets socket incoming stream
            let mut reader = BufReader::new(read);
            // this is a "client state" variable
            // will be used to register incoming 
            // stream from reader
            let mut cmd = String::new();
            
            // we need to sync the server's doc
            // with the client's local doc. This is the first
            // message written to the client.
            let (tx_first, mut rx_first) = oneshot::channel::<(String, usize)>();
            tx_edit.send(Instructions::GetDocument(tx_first)).await;
            let (doc, mut ver) = rx_first.await.unwrap();
            writer.write_all(doc.as_bytes()).await.unwrap();
            loop {
                tokio::select! {
                    result = reader.read_line(&mut cmd) => {
                        if result.unwrap() == 0 {
                            println!("A client disconnected!");
                            break;
                        }
                        match &cmd[..3] {
                            // sync from version
                            "SYV" => {
                                println!("SYV instruction! {}", ver);
                                let (tx_since, mut rx_since) = oneshot::channel::<Vec<String>>();
                                tx_edit.send(Instructions::GetCommitSince(tx_since, ver)).await;
                                //TODO learn how to do with iterator
                                for commit in rx_since.await.unwrap() {
                                    writer.write_all(commit.as_bytes()).await.unwrap();
                                }
                            },
                            // sync using doc
                            "SYN" => {
                                println!("SYN instruction {}", &cmd);
                                let (tx_first, mut rx_first) = oneshot::channel::<(String, usize)>();
                                tx_edit.send(Instructions::GetDocument(tx_first)).await;
                                writer.write_all(rx_first.await.unwrap().0.as_bytes()).await.unwrap();
                            }
                            // try to commit
                            "EDT" => {
                                println!("EDT instruction! {}", &cmd);
                                tx_edit.send(Instructions::PushCommit(String::from(&cmd[3..]), addr)).await;
                            }
                            _ => { eprintln!("WTF!"); }
                        }
                        cmd.clear();
                    },
                    result = rx_update.recv() => {
                        println!("rx_updated!");
                        writer.write_all(result.unwrap().as_bytes()).await.unwrap();
                    }
                }
            }
        });
    }
}