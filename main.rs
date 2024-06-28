use tokio::net::{TcpListener, TcpStream};
use tokio::io::{copy, AsyncReadExt, AsyncWriteExt};
use tokio::spawn;
use tokio::sync::mpsc;
use std::net::SocketAddr;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::SystemTime;
use hyper::Request;
use tokio_socks::tcp::Socks5Stream;

async fn transfer(
    mut inbound: TcpStream,
    response_sender: mpsc::Sender<io::Result<()>>,
) {
    match Socks5Stream::connect("127.0.0.1:9050", "destination.address:port").await {
        Ok(mut outbound) => {
            let (mut ri, mut wi) = inbound.split();
            let (mut ro, mut wo) = outbound.split();

            // Read request to extract URL
            let mut buffer = Vec::new();
            if let Ok(_) = ri.read_to_end(&mut buffer).await {
                if let Ok(req) = Request::from_slice(&buffer) {
                    let uri = req.uri();
                    // Log URL and timestamp
                    let timestamp = SystemTime::now();
                    let log_entry = format!("{:?} - {}\n", timestamp, uri);
                    let mut file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("data/urls.log")
                        .unwrap();
                    file.write_all(log_entry.as_bytes()).unwrap();

                    // Save request content
                    let path = format!("data/{}.html", uri.to_string().replace("/", "_"));
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .unwrap();
                    file.write_all(&buffer).unwrap();
                }
            }

            let client_to_proxy = async {
                copy(&mut ri, &mut wo).await
            };

            let proxy_to_client = async {
                copy(&mut ro, &mut wi).await
            };

            let result = tokio::select! {
                res1 = client_to_proxy => res1,
                res2 = proxy_to_client => res2,
            };

            // Convert the Result<u64, Error> to Result<(), Error>
            let result = result.map(|_| ());

            let _ = response_sender.send(result).await;
        },
        Err(e) => {
            let _ = response_sender.send(Err(e)).await;
        }
    }
}

async fn handle_client(inbound: TcpStream) {
    let (sender, mut receiver) = mpsc::channel(1);
    spawn(transfer(inbound, sender));

    if let Some(result) = receiver.recv().await {
        if let Err(e) = result {
            eprintln!("Failed to transfer data: {}", e);
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Explicitly define the type of addr to avoid type inference issues
    let addr: SocketAddr = "0.0.0.0:3030".parse().unwrap();
    let listener = TcpListener::bind(addr).await?;

    println!("Listening on: {}", addr);

    loop {
        let (socket, _) = listener.accept().await?;
        spawn(handle_client(socket));
    }
}
