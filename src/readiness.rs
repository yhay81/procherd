use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use url::Url;

use crate::{
    error::AppError,
    model::{LogStream, ReadinessCondition, ReadinessState, ReadinessStatus},
    store::now_ms,
};

const MAX_CONDITIONS: usize = 16;
const MAX_LOG_LITERAL_BYTES: usize = 1024;
const PROBE_INTERVAL: Duration = Duration::from_millis(50);
const PROBE_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_HTTP_STATUS_BYTES: usize = 1024;

pub fn build_conditions(
    tcp: Vec<String>,
    http: Vec<String>,
    files: Vec<PathBuf>,
    log_literals: Vec<String>,
    ready_ports: Vec<String>,
    leased_ports: &[String],
    working_directory: &Path,
) -> Result<Vec<ReadinessCondition>, AppError> {
    let count = tcp.len() + http.len() + files.len() + log_literals.len() + ready_ports.len();
    if count > MAX_CONDITIONS {
        return Err(AppError::usage(format!(
            "at most {MAX_CONDITIONS} readiness conditions may be configured"
        )));
    }
    let mut conditions = Vec::with_capacity(count);
    for address in tcp {
        local_socket_addresses(&address).ok_or_else(|| {
            AppError::usage(
                "readiness TCP address must be localhost:PORT or a loopback IP socket address",
            )
        })?;
        conditions.push(ReadinessCondition::Tcp { address });
    }
    for value in http {
        validate_http_url(&value)?;
        conditions.push(ReadinessCondition::Http { url: value });
    }
    for path in files {
        let path = readiness_path(working_directory, &path)?;
        conditions.push(ReadinessCondition::File { path });
    }
    for literal in log_literals {
        if literal.is_empty() || literal.len() > MAX_LOG_LITERAL_BYTES {
            return Err(AppError::usage(format!(
                "readiness log literals must contain 1 to {MAX_LOG_LITERAL_BYTES} UTF-8 bytes"
            )));
        }
        conditions.push(ReadinessCondition::Log { literal });
    }
    for name in ready_ports {
        if !leased_ports.contains(&name) {
            return Err(AppError::usage(format!(
                "readiness port {name:?} does not name a requested port lease"
            )));
        }
        conditions.push(ReadinessCondition::PortLease { name });
    }
    Ok(conditions)
}

pub struct ReadinessTracker {
    last_probe: Option<Instant>,
    next_probe_index: usize,
    stdout_tail: Vec<u8>,
    stderr_tail: Vec<u8>,
}

impl Default for ReadinessTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadinessTracker {
    pub fn new() -> Self {
        Self {
            last_probe: None,
            next_probe_index: 0,
            stdout_tail: Vec::new(),
            stderr_tail: Vec::new(),
        }
    }

    pub fn observe_log(
        &mut self,
        state: &mut ReadinessState,
        stream: LogStream,
        bytes: &[u8],
    ) -> bool {
        if state.status != ReadinessStatus::Pending {
            return false;
        }
        let tail = match stream {
            LogStream::Stdout => &mut self.stdout_tail,
            LogStream::Stderr => &mut self.stderr_tail,
        };
        let mut searchable = Vec::with_capacity(tail.len() + bytes.len());
        searchable.extend_from_slice(tail);
        searchable.extend_from_slice(bytes);

        let mut changed = false;
        for check in &mut state.checks {
            if check.ready_at_ms.is_some() {
                continue;
            }
            let ReadinessCondition::Log { literal } = &check.condition else {
                continue;
            };
            if contains_bytes(&searchable, literal.as_bytes()) {
                let observed_at = now_ms();
                check.ready_at_ms = Some(observed_at);
                check.evidence = Some(format!(
                    "{} contained the configured literal at timestamp {}",
                    stream_name(stream),
                    observed_at
                ));
                changed = true;
            }
        }

        let max_tail = state
            .checks
            .iter()
            .filter_map(|check| match &check.condition {
                ReadinessCondition::Log { literal } if check.ready_at_ms.is_none() => {
                    Some(literal.len().saturating_sub(1))
                }
                _ => None,
            })
            .max()
            .unwrap_or(0)
            .min(MAX_LOG_LITERAL_BYTES.saturating_sub(1));
        let keep = searchable.len().min(max_tail);
        tail.clear();
        tail.extend_from_slice(&searchable[searchable.len().saturating_sub(keep)..]);
        let completed = finish_if_ready(state);
        changed || completed
    }

    pub fn poll(&mut self, state: &mut ReadinessState) -> bool {
        if state.status != ReadinessStatus::Pending {
            return false;
        }
        if state
            .deadline_at_ms
            .is_some_and(|deadline| now_ms() >= deadline)
        {
            state.status = ReadinessStatus::TimedOut;
            state.failure_reason = Some("readiness_timeout".to_owned());
            return true;
        }
        if self
            .last_probe
            .is_some_and(|last| last.elapsed() < PROBE_INTERVAL)
        {
            return false;
        }
        self.last_probe = Some(Instant::now());
        let Some(index) = self.next_external_check(state) else {
            return finish_if_ready(state);
        };
        self.next_probe_index = index.saturating_add(1);
        let evidence = match &state.checks[index].condition {
            ReadinessCondition::Tcp { address } => probe_tcp(address),
            ReadinessCondition::Http { url } => probe_http(url),
            ReadinessCondition::File { path } => probe_file(path),
            ReadinessCondition::Log { .. } | ReadinessCondition::PortLease { .. } => None,
        };
        if let Some(evidence) = evidence {
            let observed_at = now_ms();
            state.checks[index].ready_at_ms = Some(observed_at);
            state.checks[index].evidence = Some(evidence);
            finish_if_ready(state);
            return true;
        }
        false
    }

    pub fn process_exited(state: &mut ReadinessState) -> bool {
        if state.status != ReadinessStatus::Pending {
            return false;
        }
        state.status = ReadinessStatus::Failed;
        state.failure_reason = Some("process_exited_before_ready".to_owned());
        true
    }

    fn next_external_check(&self, state: &ReadinessState) -> Option<usize> {
        let count = state.checks.len();
        (0..count)
            .map(|offset| (self.next_probe_index + offset) % count)
            .find(|index| {
                let check = &state.checks[*index];
                check.ready_at_ms.is_none()
                    && !matches!(
                        check.condition,
                        ReadinessCondition::Log { .. } | ReadinessCondition::PortLease { .. }
                    )
            })
    }
}

fn finish_if_ready(state: &mut ReadinessState) -> bool {
    if state.status == ReadinessStatus::Pending
        && state.checks.iter().all(|check| check.ready_at_ms.is_some())
    {
        state.status = ReadinessStatus::Ready;
        state.ready_at_ms = Some(now_ms());
        true
    } else {
        false
    }
}

fn probe_tcp(address: &str) -> Option<String> {
    local_socket_addresses(address)?
        .into_iter()
        .find_map(|address| {
            TcpStream::connect_timeout(&address, PROBE_TIMEOUT)
                .ok()
                .map(|_| format!("connected to {address}"))
        })
}

fn probe_http(value: &str) -> Option<String> {
    let (url, addresses) = validate_http_url(value).ok()?;
    for address in addresses {
        let Ok(mut stream) = TcpStream::connect_timeout(&address, PROBE_TIMEOUT) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
        let mut target = url.path().to_owned();
        if target.is_empty() {
            target.push('/');
        }
        if let Some(query) = url.query() {
            target.push('?');
            target.push_str(query);
        }
        let host = url.host_str()?;
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let host_header = match url.port() {
            Some(port) if port != 80 => format!("{host}:{port}"),
            _ => host,
        };
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n\r\n");
        if stream.write_all(request.as_bytes()).is_err() {
            continue;
        }
        let Some(code) = read_http_status(&mut stream) else {
            continue;
        };
        if (200..400).contains(&code) {
            return Some(format!("HTTP {code} from {value}"));
        }
    }
    None
}

fn read_http_status(stream: &mut TcpStream) -> Option<u16> {
    let mut response = Vec::with_capacity(128);
    while response.len() < MAX_HTTP_STATUS_BYTES {
        let mut bytes = [0_u8; 128];
        let remaining = MAX_HTTP_STATUS_BYTES - response.len();
        let read_limit = remaining.min(bytes.len());
        let count = stream.read(&mut bytes[..read_limit]).ok()?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&bytes[..count]);
        if response.contains(&b'\n') {
            break;
        }
    }
    let line = response
        .split(|byte| *byte == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())?;
    let mut fields = line.split_ascii_whitespace();
    let protocol = fields.next()?;
    if !protocol.starts_with("HTTP/1.") {
        return None;
    }
    fields.next()?.parse::<u16>().ok()
}

fn probe_file(path: &Path) -> Option<String> {
    std::fs::metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map(|metadata| format!("regular file exists ({} bytes)", metadata.len()))
}

fn local_socket_addresses(value: &str) -> Option<Vec<SocketAddr>> {
    if let Ok(address) = value.parse::<SocketAddr>() {
        return address.ip().is_loopback().then(|| vec![address]);
    }
    let port = value
        .strip_prefix("localhost:")
        .and_then(|port| port.parse::<u16>().ok())?;
    Some(vec![
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
    ])
}

fn validate_http_url(value: &str) -> Result<(Url, Vec<SocketAddr>), AppError> {
    let url = Url::parse(value)
        .map_err(|error| AppError::usage(format!("invalid readiness HTTP URL: {error}")))?;
    if url.scheme() != "http" {
        return Err(AppError::usage(
            "version 0.1 readiness HTTP URLs must use http://",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::usage(
            "readiness HTTP URLs must not contain credentials",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| AppError::usage("readiness HTTP URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AppError::usage("readiness HTTP URL has no port"))?;
    let addresses = if host.eq_ignore_ascii_case("localhost") {
        vec![
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
        ]
    } else {
        let ip = host.parse::<IpAddr>().map_err(|_| {
            AppError::usage("readiness HTTP host must be localhost or a loopback IP address")
        })?;
        if !ip.is_loopback() {
            return Err(AppError::usage(
                "readiness HTTP host must be localhost or a loopback IP address",
            ));
        }
        vec![SocketAddr::new(ip, port)]
    };
    Ok((url, addresses))
}

fn readiness_path(working_directory: &Path, path: &Path) -> Result<PathBuf, AppError> {
    if path.as_os_str().is_empty() {
        return Err(AppError::usage("readiness file path must not be empty"));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) && !path.is_absolute()
    {
        return Err(AppError::usage(
            "relative readiness file paths must not contain parent traversal",
        ));
    }
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        working_directory.join(path)
    })
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn stream_name(stream: LogStream) -> &'static str {
    match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
    }
}

#[cfg(test)]
mod tests {
    use super::{build_conditions, contains_bytes};
    use std::path::Path;

    #[test]
    fn byte_search_handles_boundaries() {
        assert!(contains_bytes(b"abc ready def", b"ready"));
        assert!(!contains_bytes(b"abc", b"ready"));
    }

    #[test]
    fn readiness_endpoints_are_local_and_bounded() {
        assert!(
            build_conditions(
                vec!["127.0.0.1:3000".to_owned()],
                vec!["http://localhost:3000/health".to_owned()],
                vec![],
                vec!["ready".to_owned()],
                vec![],
                &[],
                Path::new("."),
            )
            .is_ok()
        );
        assert!(
            build_conditions(
                vec!["8.8.8.8:53".to_owned()],
                vec![],
                vec![],
                vec![],
                vec![],
                &[],
                Path::new("."),
            )
            .is_err()
        );
    }
}
