# High-Performance Ipv6 10Gbps Fast.com Speed Test

This is a network bandwidth testing tool written in Rust, designed specifically to saturate 10Gbps XGS-PON/10G-EPON fiber internet connections.

Standard browser-based speed tests and HTTP/2 clients often fail to saturate 10Gbps connections due to high CPU overhead, TLS processing limits, and protocol multiplexing bottlenecks.

## Technologies Used

This tool addresses these bottlenecks through the following architectural decisions:

* **Rust & Tokio:** Utilizes the Tokio asynchronous runtime to handle thousands of concurrent tasks with minimal memory overhead.
* **Rustls TLS Engine:** Implements the `rustls` library for user-space TLS processing, avoiding OS-level contention locks and scaling linearly with CPU cores.
* **Forced HTTP/1.1:** Deliberately disables HTTP/2 to force the operating system to open distinct, parallel TCP sockets for every download stream.
* **Mimalloc:** Integrates the Microsoft Mimalloc memory allocator to reduce heap fragmentation and locking during high-frequency buffer writes.
* **Zero-Copy Semantics:** Optimized buffer handling to minimize memory copying between the network stack and userspace.

## Prerequisites

* **Rust Toolchain:** A working installation of the Rust compiler and Cargo.
* **IPv6 Connectivity:** This tool requires IPv6 to be enabled and working on your system.
* **Windows Users:** This tool is highly optimized for Windows but works on Linux/macOS. On Windows, ensure your "Receive Window Auto-Tuning Level" is set to `normal`.
    * Check current status:
      `netsh interface tcp show global`
    * Enable if disabled:
      `netsh interface tcp set global autotuninglevel=normal`

## Usage

1.  Build (or download pre-built) the project in release mode (debug builds will significantly limit throughput).

```bash
cargo build --release
```

2. Run the tool directly or by using `cargo`. Tuning parameters will vary based on your hardware and network latency.

```bash
cargo run --release -- --streams 4 --count 8 --time 10
```

### Command Line Arguments

| Argument | Default | Description |
| :--- | :--- | :--- |
| `-s`, `--streams` | 8 | The number of parallel download streams **per server**. |
| `-c`, `--count` | 4 | The number of unique Netflix OCA servers to test against simultaneously. |
| `--chunk-size` | 512 | The size of each download request in Megabytes. |
| `--time` | 10 | The duration of the test in seconds. |

### Server Caching

To consistently find the lowest-latency servers (which is critical for 10Gbps throughput), the tool maintains a local cache file named `server_cache.json`.

* Accumulation: Every time you run the tool, it fetches fresh server URLs from Netflix and adds them to this cache (100 limit).

* Optimization: Over time, this builds a large pool of candidate servers. On each run, the tool checks latency against all cached servers (plus new ones) and picks the absolute fastest top N servers for the test.

* Benefit: Successive runs may yield better speeds as the tool discovers and remembers the specific servers that have the best peering with your ISP.

## Results

I used the following configuration to reach __7.99__ Gbps on a residential 10Gbps 10G-EPON connection. This effectively saturates the physical limit of both XGS-PON and 10G-EPON standards after protocol overhead.

`cargo run --release -- --streams 4 --count 8`

__Note on Stability:__ At these extreme speeds, you may observe a rapid drop-off after the peak. This is typical behavior caused by __TCP Congestion Control__ (reacting to a dropped packet) or NIC Thermal Throttling. Ensure your 10G network card has adequate cooling.

## Disclaimer & Terms of Use

This tool is an unofficial client and is not affiliated with or endorsed by Netflix. It accesses the Fast.com API using automated means.

__Use with caution:__ This tool is capable of opening hundreds of concurrent high-bandwidth connections. Running this tool aggressively or repeatedly may be interpreted as a Denial of Service (DoS) attack by network providers or Netflix, potentially leading to IP bans or ISP service termination. The author is not responsible for any consequences resulting from the use of this tool.

## License

MIT License