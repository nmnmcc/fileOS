// SPDX-License-Identifier: AGPL-3.0-only

//! Port mapping on home routers.
//!
//! A server at home is behind the router's NAT. Many routers let a device
//! open an inbound port with PCP (RFC 6887) or the older NAT-PMP
//! (RFC 6886). Both are short UDP exchanges with the gateway on port 5351.
//! We try PCP first, and use NAT-PMP if PCP does not work. `UPnP` IGD is not
//! supported.
//!
//! The encode and decode functions are pure. [`map_tcp`], [`renew_tcp`] and
//! [`unmap_tcp`] do the network exchange. They use a shorter form of the RFC
//! retransmission schedule (under two seconds for each protocol).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use tokio::net::UdpSocket;

/// The UDP port PCP and NAT-PMP servers listen on.
pub const SERVER_PORT: u16 = 5351;

/// Waits between retransmissions. RFC 6886 starts at 250 ms and doubles the
/// wait each time. We stop after three tries.
const SCHEDULE: [u64; 3] = [250, 500, 1000];

/// IANA protocol number for TCP.
const TCP: u8 = 6;

/// A port mapping error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The gateway did not answer.
    Timeout,
    /// The gateway does not speak this protocol version.
    UnsupportedVersion,
    /// The gateway answered with a non-zero result code.
    Refused(u16),
    /// The answer could not be read, or did not match the request.
    Malformed,
    /// A socket error.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Timeout => f.write_str("port mapping: gateway did not answer"),
            Error::UnsupportedVersion => f.write_str("port mapping: unsupported version"),
            Error::Refused(code) => write!(f, "port mapping: gateway said no, result code {code}"),
            Error::Malformed => f.write_str("port mapping: bad answer"),
            Error::Io(e) => write!(f, "port mapping: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

/// Which protocol created a mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Port Control Protocol, RFC 6887.
    Pcp,
    /// NAT Port Mapping Protocol, RFC 6886.
    NatPmp,
}

/// A port mapping the gateway has given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// The public address, when the gateway told us.
    pub external_ip: Option<IpAddr>,
    /// The public port. It may not be the one we asked for.
    pub external_port: u16,
    /// Seconds until the mapping ends. Renew it at about half of this.
    pub lifetime: u32,
    /// The local port being forwarded.
    pub internal_port: u16,
    /// The protocol that created it.
    pub protocol: Protocol,
    /// The PCP mapping nonce, needed to renew or delete the mapping.
    pub nonce: [u8; 12],
}

// NAT-PMP (RFC 6886)

/// A NAT-PMP external address request (§3.2).
pub fn natpmp_address_request() -> [u8; 2] {
    [0, 0]
}

/// A NAT-PMP TCP mapping request (§3.3). Lifetime 0 deletes the mapping.
pub fn natpmp_map_request(
    internal_port: u16,
    suggested_external: u16,
    lifetime_s: u32,
) -> [u8; 12] {
    let mut b = [0u8; 12];
    b[1] = 2; // Map TCP.
    b[4..6].copy_from_slice(&internal_port.to_be_bytes());
    b[6..8].copy_from_slice(&suggested_external.to_be_bytes());
    b[8..12].copy_from_slice(&lifetime_s.to_be_bytes());
    b
}

/// A decoded NAT-PMP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NatPmpResponse {
    /// The answer to an external address request.
    Address {
        /// Seconds since the gateway last started (or reset) its mapping table.
        epoch: u32,
        /// The gateway's public IPv4 address.
        ip: Ipv4Addr,
    },
    /// The answer to a mapping request.
    Map {
        /// Seconds since the gateway last started (or reset) its mapping table.
        epoch: u32,
        /// 6 for TCP, 17 for UDP.
        protocol: u8,
        /// The local port.
        internal_port: u16,
        /// The public port.
        external_port: u16,
        /// Seconds the mapping lasts.
        lifetime: u32,
    },
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Decode a NAT-PMP response. A non-zero result code is an error.
///
/// # Errors
///
/// Returns an error for a truncated, unsupported or unsuccessful response.
pub fn decode_natpmp(b: &[u8]) -> Result<NatPmpResponse, Error> {
    if b.len() < 4 || b[1] < 128 {
        return Err(Error::Malformed);
    }
    if b[0] != 0 {
        return Err(Error::UnsupportedVersion);
    }
    match u16_at(b, 2) {
        0 => {}
        1 => return Err(Error::UnsupportedVersion),
        code => return Err(Error::Refused(code)),
    }
    match b[1] {
        128 if b.len() >= 12 => Ok(NatPmpResponse::Address {
            epoch: u32_at(b, 4),
            ip: Ipv4Addr::new(b[8], b[9], b[10], b[11]),
        }),
        129 | 130 if b.len() >= 16 => Ok(NatPmpResponse::Map {
            epoch: u32_at(b, 4),
            protocol: if b[1] == 130 { TCP } else { 17 },
            internal_port: u16_at(b, 8),
            external_port: u16_at(b, 10),
            lifetime: u32_at(b, 12),
        }),
        _ => Err(Error::Malformed),
    }
}

// PCP (RFC 6887)

fn to_mapped(ip: IpAddr) -> Ipv6Addr {
    match ip {
        IpAddr::V4(v4) => v4.to_ipv6_mapped(),
        IpAddr::V6(v6) => v6,
    }
}

fn from_mapped(ip: Ipv6Addr) -> IpAddr {
    match ip.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(ip),
    }
}

/// Encode a PCP MAP request for TCP.
///
/// The 24-byte common header comes before the 36-byte MAP opcode body.
/// `client` is our own address as the gateway sees it. If no external
/// address is suggested, we send the all-zeros address of the client's
/// family. Lifetime 0 deletes the mapping.
pub fn pcp_map_request(
    client: IpAddr,
    nonce: &[u8; 12],
    internal_port: u16,
    suggested_external: u16,
    suggested_ip: Option<IpAddr>,
    lifetime_s: u32,
) -> [u8; 60] {
    let mut b = [0u8; 60];
    b[0] = 2; // Version.
    b[1] = 1; // Request, opcode MAP.
    b[4..8].copy_from_slice(&lifetime_s.to_be_bytes());
    b[8..24].copy_from_slice(&to_mapped(client).octets());
    b[24..36].copy_from_slice(nonce);
    b[36] = TCP;
    b[40..42].copy_from_slice(&internal_port.to_be_bytes());
    b[42..44].copy_from_slice(&suggested_external.to_be_bytes());
    let zero = match client {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    };
    b[44..60].copy_from_slice(&to_mapped(suggested_ip.unwrap_or(zero)).octets());
    b
}

/// A decoded PCP MAP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcpMapResponse {
    /// Seconds the mapping lasts.
    pub lifetime: u32,
    /// The server's epoch time.
    pub epoch: u32,
    /// The nonce from the request.
    pub nonce: [u8; 12],
    /// The IANA protocol number.
    pub protocol: u8,
    /// The local port.
    pub internal_port: u16,
    /// The public port.
    pub external_port: u16,
    /// The public address.
    pub external_ip: IpAddr,
}

/// Decode a PCP MAP response. A non-zero result code is an error. When a
/// NAT-PMP server answers a PCP request, the result is
/// [`Error::UnsupportedVersion`].
///
/// # Errors
///
/// Returns an error for a truncated, unsupported or unsuccessful response.
pub fn decode_pcp_map(b: &[u8]) -> Result<PcpMapResponse, Error> {
    if b.len() >= 4 && b[0] == 0 {
        // A NAT-PMP-only gateway (RFC 6887 §9).
        return Err(Error::UnsupportedVersion);
    }
    if b.len() < 24 || b[0] != 2 || b[1] != 0x81 {
        return Err(Error::Malformed);
    }
    match b[3] {
        0 => {}
        1 => return Err(Error::UnsupportedVersion),
        code => return Err(Error::Refused(code.into())),
    }
    if b.len() < 60 {
        return Err(Error::Malformed);
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&b[24..36]);
    let mut ip = [0u8; 16];
    ip.copy_from_slice(&b[44..60]);
    Ok(PcpMapResponse {
        lifetime: u32_at(b, 4),
        epoch: u32_at(b, 8),
        nonce,
        protocol: b[36],
        internal_port: u16_at(b, 40),
        external_port: u16_at(b, 42),
        external_ip: from_mapped(Ipv6Addr::from(ip)),
    })
}

// The exchange.

/// Send `req` again and again, as [`SCHEDULE`] says, until `accept` returns
/// an answer.
/// `accept` returns `None` for packets that are not the answer.
async fn transact<T>(
    sock: &UdpSocket,
    req: &[u8],
    accept: impl Fn(&[u8]) -> Option<Result<T, Error>>,
) -> Result<T, Error> {
    let mut buf = [0u8; 1100];
    for wait in SCHEDULE {
        sock.send(req).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        loop {
            match tokio::time::timeout_at(deadline, sock.recv(&mut buf)).await {
                Err(_) => break,
                Ok(Err(e)) => return Err(e.into()),
                Ok(Ok(n)) => {
                    if let Some(answer) = accept(&buf[..n]) {
                        return answer;
                    }
                }
            }
        }
    }
    Err(Error::Timeout)
}

async fn connect(gateway: SocketAddr) -> Result<UdpSocket, Error> {
    let bind: SocketAddr = match gateway {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let sock = UdpSocket::bind(bind).await?;
    // A connected socket only receives datagrams from the gateway.
    sock.connect(gateway).await?;
    Ok(sock)
}

async fn pcp_map(
    gateway: SocketAddr,
    nonce: [u8; 12],
    internal_port: u16,
    suggested_external: u16,
    lifetime_s: u32,
) -> Result<Mapping, Error> {
    let sock = connect(gateway).await?;
    let client = sock.local_addr()?.ip();
    let req = pcp_map_request(
        client,
        &nonce,
        internal_port,
        suggested_external,
        None,
        lifetime_s,
    );
    let res = transact(&sock, &req, |b| match decode_pcp_map(b) {
        Ok(r) if r.nonce != nonce || r.protocol != TCP || r.internal_port != internal_port => None,
        other => Some(other),
    })
    .await?;
    Ok(Mapping {
        external_ip: (!res.external_ip.is_unspecified()).then_some(res.external_ip),
        external_port: res.external_port,
        lifetime: res.lifetime,
        internal_port,
        protocol: Protocol::Pcp,
        nonce,
    })
}

async fn natpmp_map(
    gateway: SocketAddr,
    internal_port: u16,
    suggested_external: u16,
    lifetime_s: u32,
) -> Result<Mapping, Error> {
    if !gateway.is_ipv4() {
        return Err(Error::UnsupportedVersion);
    }
    let sock = connect(gateway).await?;
    let req = natpmp_map_request(internal_port, suggested_external, lifetime_s);
    let (external_port, lifetime) = transact(&sock, &req, |b| match decode_natpmp(b) {
        Ok(NatPmpResponse::Map {
            protocol: TCP,
            internal_port: p,
            external_port,
            lifetime,
            ..
        }) if p == internal_port => Some(Ok((external_port, lifetime))),
        Ok(_) | Err(Error::Malformed) => None,
        Err(e) => Some(Err(e)),
    })
    .await?;
    let external_ip = if lifetime_s == 0 {
        None
    } else {
        transact(&sock, &natpmp_address_request(), |b| {
            match decode_natpmp(b) {
                Ok(NatPmpResponse::Address { ip, .. }) => Some(Ok(ip)),
                _ => None,
            }
        })
        .await
        .ok()
        .map(IpAddr::V4)
    };
    Ok(Mapping {
        external_ip,
        external_port,
        lifetime,
        internal_port,
        protocol: Protocol::NatPmp,
        nonce: [0; 12],
    })
}

async fn map_at(
    gateway: SocketAddr,
    nonce: [u8; 12],
    internal_port: u16,
    suggested_external: u16,
    lifetime_s: u32,
) -> Result<Mapping, Error> {
    let pcp = pcp_map(
        gateway,
        nonce,
        internal_port,
        suggested_external,
        lifetime_s,
    )
    .await;
    let pcp_err = match pcp {
        Ok(m) => return Ok(m),
        Err(e) => e,
    };
    match natpmp_map(gateway, internal_port, suggested_external, lifetime_s).await {
        Ok(m) => Ok(m),
        // Report the more useful of the two errors.
        Err(Error::Timeout | Error::UnsupportedVersion) => Err(pcp_err),
        Err(e) => Err(e),
    }
}

#[expect(
    clippy::map_err_ignore,
    reason = "ring does not say why the operating system RNG failed"
)]
fn new_nonce() -> Result<[u8; 12], Error> {
    let mut nonce = [0u8; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error::Io("no randomness".into()))?;
    Ok(nonce)
}

/// Ask the gateway to forward a public TCP port to `internal_port` on this
/// machine for `lifetime_s` seconds. It tries PCP, then NAT-PMP.
///
/// # Errors
///
/// Returns an error if no supported protocol can make the mapping.
pub async fn map_tcp(
    gateway: IpAddr,
    internal_port: u16,
    suggested_external: u16,
    lifetime_s: u32,
) -> Result<Mapping, Error> {
    let gw = SocketAddr::new(gateway, SERVER_PORT);
    map_at(
        gw,
        new_nonce()?,
        internal_port,
        suggested_external,
        lifetime_s,
    )
    .await
}

/// Renew a mapping before it ends. The port and nonce stay the same.
///
/// # Errors
///
/// Returns an error if the gateway says no to the renewal or cannot be
/// reached.
pub async fn renew_tcp(
    gateway: IpAddr,
    mapping: &Mapping,
    lifetime_s: u32,
) -> Result<Mapping, Error> {
    let gw = SocketAddr::new(gateway, SERVER_PORT);
    map_at(
        gw,
        mapping.nonce,
        mapping.internal_port,
        mapping.external_port,
        lifetime_s,
    )
    .await
}

/// Delete a mapping (a request with lifetime 0).
///
/// # Errors
///
/// Returns an error if the gateway says no to the removal or cannot be
/// reached.
pub async fn unmap_tcp(gateway: IpAddr, mapping: &Mapping) -> Result<(), Error> {
    unmap_at(SocketAddr::new(gateway, SERVER_PORT), mapping).await
}

async fn unmap_at(gw: SocketAddr, mapping: &Mapping) -> Result<(), Error> {
    match mapping.protocol {
        Protocol::Pcp => pcp_map(gw, mapping.nonce, mapping.internal_port, 0, 0).await,
        Protocol::NatPmp => natpmp_map(gw, mapping.internal_port, 0, 0).await,
    }
    .map(|_| ())
}

// The default gateway.

/// The default IPv4 gateway, if it can be found: from `/proc/net/route` on
/// Linux, or `route -n get default` on macOS and the BSDs. Else `None`.
pub fn default_gateway() -> Option<IpAddr> {
    #[cfg(target_os = "linux")]
    {
        parse_proc_net_route(&std::fs::read_to_string("/proc/net/route").ok()?)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        let out = std::process::Command::new("route")
            .args(["-n", "get", "default"])
            .output()
            .ok()?;
        parse_route_get(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    {
        None
    }
}

/// Find the default route's gateway in Linux `/proc/net/route` text.
#[cfg(any(test, target_os = "linux"))]
fn parse_proc_net_route(text: &str) -> Option<IpAddr> {
    text.lines().skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (dest, gw, flags) = (f.get(1)?, f.get(2)?, f.get(3)?);
        let flags = u16::from_str_radix(flags, 16).ok()?;
        // RTF_UP | RTF_GATEWAY, destination 0.0.0.0.
        if *dest != "00000000" || flags & 3 != 3 {
            return None;
        }
        // The kernel prints the address as a host-order integer.
        let raw = u32::from_str_radix(gw, 16).ok()?;
        Some(IpAddr::V4(Ipv4Addr::from(raw.to_ne_bytes())))
    })
}

/// Find the `gateway:` line in `route -n get default` output.
#[cfg(any(
    test,
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn parse_route_get(text: &str) -> Option<IpAddr> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("gateway:"))
        .and_then(|g| g.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natpmp_layout() {
        assert_eq!(natpmp_address_request(), [0, 0]);
        assert_eq!(
            natpmp_map_request(8443, 443, 7200),
            [0, 2, 0, 0, 0x20, 0xfb, 0x01, 0xbb, 0, 0, 0x1c, 0x20]
        );
        let addr = [0, 128, 0, 0, 0, 0, 0, 9, 203, 0, 113, 7];
        assert_eq!(
            decode_natpmp(&addr),
            Ok(NatPmpResponse::Address {
                epoch: 9,
                ip: Ipv4Addr::new(203, 0, 113, 7)
            })
        );
        let map = [
            0, 130, 0, 0, 0, 0, 1, 0, 0x20, 0xfb, 0x01, 0xbc, 0, 0, 0x0e, 0x10,
        ];
        assert_eq!(
            decode_natpmp(&map),
            Ok(NatPmpResponse::Map {
                epoch: 256,
                protocol: TCP,
                internal_port: 8443,
                external_port: 444,
                lifetime: 3600
            })
        );
        assert_eq!(decode_natpmp(&[0, 130, 0, 2]), Err(Error::Refused(2)));
        assert_eq!(decode_natpmp(&[0, 130, 0, 0, 0]), Err(Error::Malformed));
        assert_eq!(decode_natpmp(&[]), Err(Error::Malformed));
    }

    #[test]
    fn pcp_layout() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let client = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));
        let req = pcp_map_request(client, &nonce, 8443, 443, None, 7200);
        assert_eq!(&req[..4], &[2, 1, 0, 0]);
        assert_eq!(&req[4..8], &7200u32.to_be_bytes());
        assert_eq!(
            &req[8..24],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 168, 1, 20]
        );
        assert_eq!(&req[24..36], &nonce);
        assert_eq!(&req[36..40], &[6, 0, 0, 0]);
        assert_eq!(&req[40..44], &[0x20, 0xfb, 0x01, 0xbb]);
        assert_eq!(
            &req[44..60],
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]
        );

        let mut res = req;
        res[1] = 0x81;
        res[3] = 0;
        res[4..8].copy_from_slice(&3600u32.to_be_bytes());
        res[8..12].copy_from_slice(&77u32.to_be_bytes());
        res[12..24].fill(0);
        res[44..60].copy_from_slice(&Ipv4Addr::new(203, 0, 113, 7).to_ipv6_mapped().octets());
        let r = decode_pcp_map(&res).unwrap();
        assert_eq!(r.lifetime, 3600);
        assert_eq!(r.epoch, 77);
        assert_eq!(r.nonce, nonce);
        assert_eq!(r.external_port, 443);
        assert_eq!(r.external_ip, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)));

        res[3] = 2; // NOT_AUTHORIZED
        assert_eq!(decode_pcp_map(&res), Err(Error::Refused(2)));
        assert_eq!(decode_pcp_map(&res[..30]), Err(Error::Refused(2)));
        res[3] = 0;
        assert_eq!(decode_pcp_map(&res[..30]), Err(Error::Malformed));
        assert_eq!(
            decode_pcp_map(&[0, 129, 0, 1, 0, 0, 0, 0]),
            Err(Error::UnsupportedVersion)
        );
    }

    #[test]
    fn gateways() {
        let linux = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     eth0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
                     eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n";
        let expected = u32::from_ne_bytes([192, 168, 1, 1]);
        let linux = linux.replace("0101A8C0", &format!("{expected:08X}"));
        assert_eq!(
            parse_proc_net_route(&linux),
            Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
        let mac = "   route to: default\ndestination: default\n       mask: default\n    gateway: 10.0.0.1\n  interface: en0\n";
        assert_eq!(
            parse_route_get(mac),
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
        );
        assert_eq!(
            parse_route_get("route: writing to routing socket: not in table"),
            None
        );
    }

    /// A fake gateway. If `pcp`, it answers PCP. Else it answers "unsupported
    /// version" to PCP, and uses NAT-PMP.
    async fn fake_gateway(pcp: bool) -> SocketAddr {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 1100];
            loop {
                let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                    return;
                };
                let req = &buf[..n];
                let reply: Vec<u8> = match (req[0], pcp) {
                    (2, true) => {
                        assert_eq!(n, 60);
                        assert_eq!(&req[8..24], &Ipv4Addr::LOCALHOST.to_ipv6_mapped().octets());
                        let mut r = req.to_vec();
                        r[1] = 0x81;
                        r[12..24].fill(0);
                        let ext = u16_at(req, 42).max(1) + 1;
                        r[42..44].copy_from_slice(&ext.to_be_bytes());
                        r[44..60].copy_from_slice(
                            &Ipv4Addr::new(198, 51, 100, 1).to_ipv6_mapped().octets(),
                        );
                        r
                    }
                    (2, false) => vec![0, 128 + req[1], 0, 1, 0, 0, 0, 5],
                    (0, _) if req[1] == 0 => vec![0, 128, 0, 0, 0, 0, 0, 5, 198, 51, 100, 2],
                    (0, _) => {
                        let mut r = vec![0, 128 + req[1], 0, 0, 0, 0, 0, 5];
                        r.extend_from_slice(&req[4..6]);
                        r.extend_from_slice(&req[6..8]);
                        r.extend_from_slice(&req[8..12]);
                        r
                    }
                    _ => continue,
                };
                sock.send_to(&reply, from).await.unwrap();
            }
        });
        addr
    }

    #[tokio::test]
    async fn loopback_pcp() {
        let gw = fake_gateway(true).await;
        let m = map_at(gw, [9; 12], 8443, 443, 3600).await.unwrap();
        assert_eq!(m.protocol, Protocol::Pcp);
        assert_eq!(m.external_port, 444);
        assert_eq!(m.lifetime, 3600);
        assert_eq!(
            m.external_ip,
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)))
        );
        assert_eq!(m.nonce, [9; 12]);
        unmap_at(gw, &m).await.unwrap();
    }

    #[tokio::test]
    async fn loopback_natpmp_fallback() {
        let gw = fake_gateway(false).await;
        let m = map_at(gw, [9; 12], 8443, 443, 3600).await.unwrap();
        assert_eq!(m.protocol, Protocol::NatPmp);
        assert_eq!(m.external_port, 443);
        assert_eq!(m.lifetime, 3600);
        assert_eq!(
            m.external_ip,
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 2)))
        );
        unmap_at(gw, &m).await.unwrap();
    }
}
