use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 8080;
const PROBE_INTERVAL: Duration = Duration::from_secs(1);
const STARTING_PROBE_INTERVAL: Duration = Duration::from_millis(300);
const START_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);

pub struct LocalServer {
    child: Option<Child>,
    listen: SocketAddr,
    lan_ip: Option<Ipv4Addr>,
    reachable: bool,
    starting_since: Option<Instant>,
    last_probe: Option<Instant>,
    last_error: Option<String>,
    log_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct ServerSnapshot {
    pub reachable: bool,
    pub starting: bool,
    pub external: bool,
    pub address: String,
    pub error: Option<String>,
}

pub enum ToggleOutcome {
    Started,
    Stopped,
    External,
    Failed(String),
}

impl LocalServer {
    pub fn new() -> Self {
        Self {
            child: None,
            listen: listen_address(),
            lan_ip: None,
            reachable: false,
            starting_since: None,
            last_probe: None,
            last_error: None,
            log_path: None,
        }
    }

    pub fn refresh(&mut self) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child = None;
                    self.reachable = false;
                    self.starting_since = None;
                    self.last_error =
                        Some(self.with_log(format!("O servidor encerrou ({status}).")));
                }
                Ok(None) => {}
                Err(error) => {
                    self.child = None;
                    self.reachable = false;
                    self.starting_since = None;
                    self.last_error =
                        Some(self.with_log(format!("Falha ao acompanhar o servidor: {error}")));
                }
            }
        }

        let interval = if self.starting_since.is_some() {
            STARTING_PROBE_INTERVAL
        } else {
            PROBE_INTERVAL
        };
        let probe_due = self
            .last_probe
            .map(|probe| probe.elapsed() >= interval)
            .unwrap_or(true);
        if !probe_due {
            return;
        }

        self.last_probe = Some(Instant::now());
        self.reachable = probe_health(self.listen);
        if self.reachable {
            self.starting_since = None;
            self.last_error = None;
        } else if self
            .starting_since
            .map(|started| started.elapsed() >= START_TIMEOUT)
            .unwrap_or(false)
        {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            self.starting_since = None;
            self.last_error = Some(
                self.with_log("O servidor não respondeu em 10 segundos. Verifique o log.".into()),
            );
        }
    }

    pub fn toggle(&mut self) -> ToggleOutcome {
        self.last_error = None;
        if self.child.is_some() {
            self.stop();
            return ToggleOutcome::Stopped;
        }
        if self.reachable {
            return ToggleOutcome::External;
        }

        match self.start() {
            Ok(()) => ToggleOutcome::Started,
            Err(error) => {
                self.last_error = Some(self.with_log(error.clone()));
                ToggleOutcome::Failed(error)
            }
        }
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        let starting = self.child.is_some() && !self.reachable && self.starting_since.is_some();
        ServerSnapshot {
            reachable: self.reachable,
            starting,
            external: self.reachable && self.child.is_none(),
            address: format!("{}:{}", self.host(), self.listen.port()),
            error: self.last_error.clone(),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.snapshot().address)
    }

    pub fn port(&self) -> u16 {
        self.listen.port()
    }

    fn start(&mut self) -> Result<(), String> {
        let executable = server_executable();
        let networks_file = networks_file();
        self.log_path = None;
        if let Some(parent) = networks_file.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "Não foi possível criar a pasta {}: {error}",
                        parent.display()
                    )
                })?;
            }
        }

        let log_path = networks_file.with_file_name("server.log");
        let mut command = Command::new(&executable);
        command
            .stdin(Stdio::null())
            .env("SHADOWVPN_NETWORKS_FILE", &networks_file);
        if let Ok(log) = open_log(&log_path) {
            if let Ok(errors) = log.try_clone() {
                command.stdout(Stdio::from(log)).stderr(Stdio::from(errors));
                self.log_path = Some(log_path);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let child = command.spawn().map_err(|error| {
            format!("Não foi possível iniciar {}: {error}", executable.display())
        })?;
        self.child = Some(child);
        self.reachable = false;
        self.starting_since = Some(Instant::now());
        self.last_probe = None;
        self.lan_ip = lan_ipv4();
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.reachable = false;
        self.starting_since = None;
        self.last_probe = Some(Instant::now());
    }

    fn host(&self) -> Ipv4Addr {
        match self.listen.ip() {
            IpAddr::V4(address) if !address.is_unspecified() => address,
            _ => self.lan_ip.unwrap_or(Ipv4Addr::LOCALHOST),
        }
    }

    fn with_log(&self, message: String) -> String {
        match &self.log_path {
            Some(path) => format!("{message} Log: {}", path.display()),
            None => message,
        }
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn is_loopback_url(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    let value = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(value);
    let host = value.split('/').next().unwrap_or(value);
    host.starts_with("127.")
        || host.starts_with("localhost")
        || host.starts_with("[::1]")
        || host.starts_with("0.0.0.0")
}

fn server_executable() -> PathBuf {
    let name = format!("shadowvpn-server{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .map(|current| current.with_file_name(&name))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

fn listen_address() -> SocketAddr {
    std::env::var("SHADOWVPN_LISTEN")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)))
}

fn probe_health(listen: SocketAddr) -> bool {
    let address = probe_address(listen);
    let Ok(mut stream) = TcpStream::connect_timeout(&address, PROBE_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        address.ip(),
        address.port()
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    if stream.take(2048).read_to_string(&mut response).is_err() {
        return false;
    }
    response.starts_with("HTTP/1.1 200") && response.contains("\"status\":\"ok\"")
}

fn probe_address(listen: SocketAddr) -> SocketAddr {
    if listen.ip().is_unspecified() {
        SocketAddr::new(
            if listen.is_ipv4() {
                Ipv4Addr::LOCALHOST.into()
            } else {
                Ipv6Addr::LOCALHOST.into()
            },
            listen.port(),
        )
    } else {
        listen
    }
}

pub fn relay_port() -> u16 {
    std::env::var("SHADOWVPN_RELAY_LISTEN")
        .ok()
        .and_then(|value| value.parse::<SocketAddr>().ok())
        .map(|address| address.port())
        .unwrap_or(8081)
}

fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(address) if !address.is_loopback() && !address.is_unspecified() => {
            Some(address)
        }
        _ => None,
    }
}

fn open_log(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn networks_file() -> PathBuf {
    if let Some(path) = std::env::var_os("SHADOWVPN_NETWORKS_FILE") {
        return PathBuf::from(path);
    }

    if let Some(base) = data_dir() {
        return base.join("shadowvpn").join("networks.json");
    }
    PathBuf::from("data").join("networks.json")
}

fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("share"))
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn recognizes_loopback_urls() {
        assert!(is_loopback_url(""));
        assert!(is_loopback_url("http://127.0.0.1:8080"));
        assert!(is_loopback_url("http://localhost:8080/"));
        assert!(is_loopback_url("https://[::1]:8080"));
        assert!(!is_loopback_url("http://192.168.1.10:8080"));
        assert!(!is_loopback_url("https://vpn.example.com"));
    }

    #[test]
    fn probes_the_configured_bind_address() {
        let loopback: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        assert_eq!(probe_address(loopback), loopback);

        let bound: SocketAddr = "192.0.2.10:8080".parse().unwrap();
        assert_eq!(probe_address(bound), bound);

        let any: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert_eq!(probe_address(any), "127.0.0.1:8080".parse().unwrap());

        let any_v6: SocketAddr = "[::]:8080".parse().unwrap();
        assert_eq!(probe_address(any_v6), "[::1]:8080".parse().unwrap());
    }

    #[test]
    fn probes_the_health_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}",
                )
                .unwrap();
        });

        assert!(probe_health(address));
        handle.join().unwrap();
    }

    #[test]
    fn rejects_unrelated_http_services() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        });

        assert!(!probe_health(address));
        handle.join().unwrap();
    }
}
