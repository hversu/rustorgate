use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::spawn;
use std::net::SocketAddr;
use tokio::io;
use url::Url;
use select::document::Document;
use select::predicate::Name;
use chrono::Local;
use std::fs::OpenOptions;
use std::io::Write;
use std::error::Error;
use reqwest::Proxy;

async fn log_and_scrape(data: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Ok(request) = String::from_utf8(data.to_vec()) {
        let lines: Vec<&str> = request.lines().collect();
        if !lines.is_empty() {
            let first_line = lines[0];
            let full_url = if first_line.starts_with("CONNECT ") {
                // HTTPS connection
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                if parts.len() >= 2 {
                    format!("https://{}", parts[1])
                } else {
                    return Ok(());
                }
            } else if first_line.starts_with("GET ") || first_line.starts_with("POST ") {
                // HTTP connection
                let parts: Vec<&str> = first_line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let path = parts[1];
                    let host = lines.iter()
                        .find(|line| line.to_lowercase().starts_with("host:"))
                        .and_then(|line| line.split_once(":"))
                        .map(|(_, host)| host.trim())
                        .unwrap_or("unknown");
                    format!("http://{}{}", host, path)
                } else {
                    return Ok(());
                }
            } else {
                return Ok(());
            };

            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            
            // Log URL and timestamp
            let log_entry = format!("{} - {}\n", timestamp, full_url);
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open("access_log.txt")?;
            file.write_all(log_entry.as_bytes())?;

            // Scrape and save page content (this part is done asynchronously)
            if let Ok(parsed_url) = Url::parse(&full_url) {
                spawn(async move {
                    let proxy = Proxy::all("socks5h://127.0.0.1:9050").unwrap();
                    let client = reqwest::Client::builder()
                        .proxy(proxy)
                        .danger_accept_invalid_certs(true)
                        .build()
                        .unwrap();

                    match client.get(parsed_url).send().await {
                        Ok(response) => {
                            if let Ok(content) = response.text().await {
                                let document = Document::from(content.as_str());
                                let mut scraped_content = String::new();

                                // Extract headers
                                for i in 1..=6 {
                                    let header_tag = format!("h{}", i);
                                    for node in document.find(Name(header_tag.as_str())) {
                                        scraped_content.push_str(&format!("{}: {}\n", header_tag.to_uppercase(), node.text().trim()));
                                    }
                                }

                                // Extract paragraphs
                                for node in document.find(Name("p")) {
                                    scraped_content.push_str(&format!("P: {}\n", node.text().trim()));
                                }

                                // Extract links
                                for node in document.find(Name("a")) {
                                    if let Some(href) = node.attr("href") {
                                        scraped_content.push_str(&format!("LINK: {} ({})\n", node.text().trim(), href));
                                    }
                                }

                                // Extract tables
                                for table in document.find(Name("table")) {
                                    scraped_content.push_str("TABLE:\n");
                                    for row in table.find(Name("tr")) {
                                        let cells: Vec<String> = row.find(Name("td")).map(|cell| cell.text().trim().to_string()).collect();
                                        scraped_content.push_str(&format!("  {}\n", cells.join(" | ")));
                                    }
                                }

                                if let Ok(mut file) = OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open("scraped_content.txt") {
                                    let _ = writeln!(file, "====================");
                                    let _ = writeln!(file, "URL: {}", full_url);
                                    let _ = writeln!(file, "Timestamp: {}", timestamp);
                                    let _ = writeln!(file, "====================\n");
                                    let _ = writeln!(file, "{}\n\n", scraped_content);
                                }
                            }
                        },
                        Err(e) => eprintln!("Failed to fetch URL: {}", e),
                    }
                });
            }
        }
    }
    Ok(())
}

async fn transfer(mut inbound: TcpStream, mut outbound: TcpStream) -> io::Result<()> {
    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let client_to_server = async {
        let mut buffer = [0; 8192];
        loop {
            let n = ri.read(&mut buffer).await?;
            if n == 0 {
                return Ok(());
            }
            if let Err(e) = log_and_scrape(&buffer[..n]).await {
                eprintln!("Error logging and scraping: {}", e);
            }
            wo.write_all(&buffer[..n]).await?;
        }
    };

    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await?;
        Ok(())
    };

    tokio::select! {
        result = client_to_server => result,
        result = server_to_client => result,
    }
}

async fn handle_client(inbound: TcpStream) {
    let proxy_addr: SocketAddr = "127.0.0.1:9050".parse().unwrap();
    
    match TcpStream::connect(proxy_addr).await {
        Ok(outbound) => {
            if let Err(e) = transfer(inbound, outbound).await {
                eprintln!("Failed to transfer: {}", e);
            }
        },
        Err(e) => eprintln!("Failed to connect to proxy: {}", e),
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let addr: SocketAddr = "0.0.0.0:3030".parse().unwrap();
    let listener = TcpListener::bind(addr).await?;

    println!("Listening on: {}", addr);

    loop {
        let (socket, _) = listener.accept().await?;
        spawn(handle_client(socket));
    }
}
