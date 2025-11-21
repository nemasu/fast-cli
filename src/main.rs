use anyhow::{Context, Result};
use clap::Parser;
use futures::stream::StreamExt;
use mimalloc::MiMalloc;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT, HOST};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::time::sleep;
use url::Url;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const CACHE_FILE: &str = "server_cache.json";
const UPDATE_BATCH_SIZE: u64 = 1024 * 1024; 

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of parallel streams per server
    #[arg(short, long, default_value_t = 8)]
    streams: usize,

    /// Number of servers to test against
    #[arg(short, long, default_value_t = 4)]
    count: usize,

    /// Download chunk size in MB (Increase this for higher speeds)
    #[arg(long, default_value_t = 512)]
    chunk_size: u64,

    /// Duration of the test in seconds
    #[arg(short, long, default_value_t = 10)]
    time: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq, Hash)]
struct ServerInfo {
    url: String,
}

#[derive(Debug, Clone)]
struct ValidatedServer {
    url: String,
    ipv6: IpAddr,
    latency_ms: u128,
}

struct SpeedTest;

impl SpeedTest {
    fn create_client() -> Client {
        Client::builder()
            .danger_accept_invalid_certs(true)
            .http1_only()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(100)
            .no_brotli()
            .no_gzip()
            .default_headers(Self::get_headers())
            .build()
            .unwrap()
    }

    fn get_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36"));
        headers.insert("Referer", HeaderValue::from_static("https://fast.com/"));
        headers.insert("Origin", HeaderValue::from_static("https://fast.com"));
        headers
    }

    async fn get_token(client: &Client) -> Result<String> {
        println!("[-] Fetching Fast.com main page...");
        let body = client.get("https://fast.com/").send().await?.text().await?;
        
        let js_regex = Regex::new(r#"<script src="([^"]+app-[^"]+\.js)""#)?;
        let js_path = js_regex.captures(&body).context("Could not find app.js")?.get(1).unwrap().as_str();
        let js_url = format!("https://fast.com{}", js_path);
        let js_body = client.get(&js_url).send().await?.text().await?;
        let token_regex = Regex::new(r#"token:"([^"]+)""#)?;
        let token = token_regex.captures(&js_body).context("Could not find token")?.get(1).unwrap().as_str().to_string();
        Ok(token)
    }

    async fn resolve_ipv6(url_str: &str) -> Option<ValidatedServer> {
        let url = Url::parse(url_str).ok()?;
        let host = url.host_str()?;
        let port = 443;
        let addrs = tokio::net::lookup_host(format!("{}:{}", host, port)).await.ok()?;
        
        for addr in addrs {
            if let SocketAddr::V6(sa) = addr {
                let start = Instant::now();
                if let Ok(stream) = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await {
                    if stream.is_ok() {
                        let latency = start.elapsed().as_millis();
                        return Some(ValidatedServer {
                            url: url_str.to_string(),
                            ipv6: IpAddr::V6(sa.ip().clone()),
                            latency_ms: latency,
                        });
                    }
                }
            }
        }
        None
    }

    async fn get_servers(client: &Client, token: &str, count: usize) -> Result<Vec<ValidatedServer>> {
        let mut unique_urls: HashSet<String> = HashSet::new();
        // 1. Load Cache
        if let Ok(content) = tokio::fs::read_to_string(CACHE_FILE).await {
            if let Ok(cached) = serde_json::from_str::<Vec<String>>(&content) {
                println!("[-] Loaded {} servers from cache.", cached.len());
                unique_urls.extend(cached);
            }
        }

        // 2. Fetch New from API
        println!("[-] Fetching new server list from API...");
        for _ in 0..3 {
            let url = format!("https://api.fast.com/netflix/speedtest?https=true&token={}&urlCount=30", token);
            if let Ok(resp) = client.get(&url).send().await {
                if let Ok(servers) = resp.json::<Vec<ServerInfo>>().await {
                    for s in servers { unique_urls.insert(s.url); }
                }
            }
            sleep(Duration::from_millis(200)).await;
        }

        println!("[-] Resolving IPv6 for {} unique candidates...", unique_urls.len());
        let futures = unique_urls.into_iter().map(|u| tokio::spawn(async move { Self::resolve_ipv6(&u).await }));
        let results = futures::future::join_all(futures).await;
        let mut valid_servers: Vec<ValidatedServer> = results.into_iter().filter_map(|r| r.ok().flatten()).collect();

        if valid_servers.is_empty() { anyhow::bail!("No reachable IPv6 servers found!"); }
        
        // Sort by latency
        valid_servers.sort_by_key(|s| s.latency_ms);

        // 3. Save Top 100 Cache
        let top_100_urls: Vec<String> = valid_servers.iter()
            .take(100)
            .map(|s| s.url.clone())
            .collect();

        if let Ok(json) = serde_json::to_string_pretty(&top_100_urls) {
            println!("[-] Optimized cache: Found {} reachable. Keeping top {}.", valid_servers.len(), top_100_urls.len());
            let _ = tokio::fs::write(CACHE_FILE, json).await;
        }

        println!("\n[-] Selected Top IPv6 Servers:");
        let selected = valid_servers.into_iter().take(count).collect::<Vec<_>>();
        for s in &selected {
            let host = Url::parse(&s.url).unwrap().host_str().unwrap().to_string();
            println!("    {}ms | {} ({})", s.latency_ms, s.ipv6, host);
        }
        println!("");
        Ok(selected)
    }

    async fn run_download_worker(
        server: ValidatedServer, 
        counter: Arc<AtomicU64>, 
        _notify: Arc<Notify>,
        range_bytes: u64,
    ) {
        let client = Self::create_client();
        let url_parsed = Url::parse(&server.url).unwrap();
        let hostname = url_parsed.host_str().unwrap();
        
        let target_url = server.url
            .replace(hostname, &format!("[{}]", server.ipv6))
            .replace("/speedtest", &format!("/speedtest/range/0-{}", range_bytes));

        loop {
            let request = client.get(&target_url).header(HOST, hostname);
            match request.send().await {
                Ok(response) => {
                    let mut stream = response.bytes_stream();
                    let mut local_bytes: u64 = 0;
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(chunk) => {
                                local_bytes += chunk.len() as u64;
                                if local_bytes > UPDATE_BATCH_SIZE {
                                    counter.fetch_add(local_bytes, Ordering::Relaxed);
                                    local_bytes = 0;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    if local_bytes > 0 { counter.fetch_add(local_bytes, Ordering::Relaxed); }
                }
                Err(_) => { sleep(Duration::from_millis(100)).await; }
            }
        }
    }

    async fn monitor(total_bytes: Arc<AtomicU64>, duration_secs: u64) {
        println!("[+] Starting 10Gbps Speed Test. Duration: {}s. Press Ctrl+C to stop.\n", duration_secs);
        let mut last_bytes = 0;
        let mut last_time = Instant::now();
        let start_of_test = Instant::now();
        let mut max_bps = 0.0;

        loop {
            sleep(Duration::from_secs(1)).await;
            
            if start_of_test.elapsed().as_secs() >= duration_secs {
                break;
            }

            let current_bytes = total_bytes.load(Ordering::Relaxed);
            let now = Instant::now();
            let elapsed = now.duration_since(last_time).as_secs_f64();
            
            if elapsed > 0.0 {
                let delta_bytes = current_bytes - last_bytes;
                let bps = (delta_bytes as f64 * 8.0) / elapsed;
                
                if bps > max_bps {
                    max_bps = bps;
                }

                let format_speed = |s: f64| {
                    if s > 1_000_000_000.0 {
                        format!("{:.2} Gbps", s / 1_000_000_000.0)
                    } else {
                        format!("{:.2} Mbps", s / 1_000_000.0)
                    }
                };

                print!("\rCurrent: {} | Max: {}   ", format_speed(bps), format_speed(max_bps));
                use std::io::Write;
                std::io::stdout().flush().unwrap();

                last_bytes = current_bytes;
                last_time = now;
            }
        }
        println!("\n\n[+] Test finished.");
    }

    pub async fn run(args: Args) -> Result<()> {
        let main_client = Self::create_client();
        let token = Self::get_token(&main_client).await?;
        let servers = Self::get_servers(&main_client, &token, args.count).await?;
        let total_bytes = Arc::new(AtomicU64::new(0));
        let shutdown_notify = Arc::new(Notify::new());
        let range_size_bytes = args.chunk_size * 1024 * 1024;

        println!("[-] Launching {} async tasks per server ({} total)...", args.streams, args.streams * servers.len());
        println!("[-] Chunk Size: {} MB", args.chunk_size);

        for server in servers {
            for _ in 0..args.streams {
                let srv = server.clone();
                let ctr = total_bytes.clone();
                let not = shutdown_notify.clone();
                tokio::spawn(async move { Self::run_download_worker(srv, ctr, not, range_size_bytes).await; });
            }
        }

        let monitor_fut = Self::monitor(total_bytes.clone(), args.time);
        tokio::select! { _ = monitor_fut => {}, _ = tokio::signal::ctrl_c() => { println!("\n\n[+] Test stopped by user."); } }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    if let Err(e) = SpeedTest::run(args).await { eprintln!("Error: {}", e); }
}