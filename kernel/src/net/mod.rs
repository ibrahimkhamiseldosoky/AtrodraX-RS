//! TCP/IP stack: Ethernet, ARP, IPv4, ICMP, UDP, DHCP, DNS and minimal TCP.
//!
//! Addresses are kept as host integers in wire order (`make_ip(10,0,2,2)`
//! returns `0x0A000202`), so serialising uses `to_be_bytes`.

use crate::drivers::e1000;
use alloc::string::String;
use alloc::vec::Vec;

pub const AF_INET: i32 = 2;
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;

pub const SOCK_STATE_CLOSED: i32 = 0;
pub const SOCK_STATE_UNCONNECTED: i32 = 1;
pub const SOCK_STATE_CONNECTED: i32 = 2;
pub const SOCK_STATE_LISTENING: i32 = 3;
pub const SOCK_STATE_SYN_SENT: i32 = 4;
pub const SOCK_STATE_SYN_RECV: i32 = 5;
pub const SOCK_STATE_FIN_WAIT: i32 = 6;

const ETH_ARP: u16 = 0x0806;
const ETH_IPV4: u16 = 0x0800;
const IP_ICMP: u8 = 1;
const IP_TCP: u8 = 6;
const IP_UDP: u8 = 17;

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DNS_PORT: u16 = 53;

const MAX_SOCKETS: usize = 16;
const RX_CAP: usize = 32 * 1024;

#[derive(Clone, Copy, Default)]
struct ArpEntry {
    ip: u32,
    mac: [u8; 8],
    valid: bool,
}

struct Socket {
    used: bool,
    kind: i32,
    state: i32,
    local_port: u16,
    remote_ip: u32,
    remote_port: u16,
    rx: Vec<u8>,
    src_ip: u32,
    src_port: u16,
    seq: u32,
    ack: u32,
    pending_syn: Option<(u32, u16, u32)>,
}

impl Socket {
    fn new(kind: i32) -> Self {
        Self {
            used: true,
            kind,
            state: SOCK_STATE_UNCONNECTED,
            local_port: 0,
            remote_ip: 0,
            remote_port: 0,
            rx: Vec::new(),
            src_ip: 0,
            src_port: 0,
            seq: 0x1000_0000,
            ack: 0,
            pending_syn: None,
        }
    }
}

struct NetState {
    ip: u32,
    gateway: u32,
    netmask: u32,
    dns: u32,
    mac: [u8; 6],
    arp: [ArpEntry; 8],
    arp_count: usize,
    dhcp_lease: bool,
    dhcp_txid: u32,
    dhcp_retries: u32,
    last_dhcp_ms: u64,
    ping_target: u32,
    ping_seq: u16,
    last_echo: u16,
    icmp_replies: u32,
    dns_txid: u16,
    dns_ip: u32,
    dns_ready: bool,
    dns_pending: bool,
    rx_packets: u64,
    tx_packets: u64,
    arp_packets: u64,
    udp_packets: u64,
    tcp_packets: u64,
    ip_next_id: u16,
}

static mut STATE: Option<NetState> = None;
static mut SOCKETS: Option<Vec<Option<Socket>>> = None;

fn state() -> &'static mut NetState {
    unsafe { STATE.as_mut().expect("net not initialized") }
}

fn sockets() -> &'static mut Vec<Option<Socket>> {
    unsafe { SOCKETS.as_mut().expect("sockets not initialized") }
}

pub fn present() -> bool {
    e1000::present()
}

pub fn mac() -> [u8; 6] {
    state().mac
}

fn ip_bytes(ip: u32) -> [u8; 4] {
    ip.to_be_bytes()
}

fn ip_from_bytes(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

pub fn local_ip() -> u32 {
    state().ip
}

pub fn gateway() -> u32 {
    state().gateway
}

pub fn dns_server() -> u32 {
    state().dns
}

pub fn ip_string(ip: u32) -> String {
    let b = ip_bytes(ip);
    alloc::format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

pub fn init() {
    let mut mac = [0u8; 6];
    if e1000::present() {
        mac = e1000::mac();
    }
    unsafe {
        STATE = Some(NetState {
            ip: 0x0A00_020F,      // 10.0.2.15 fallback (QEMU slirp)
            gateway: 0x0A00_0202, // 10.0.2.2
            netmask: 0xFFFF_FF00,
            dns: 0x0A00_0203,     // 10.0.2.3
            mac,
            arp: [ArpEntry::default(); 8],
            arp_count: 0,
            dhcp_lease: false,
            dhcp_txid: 0x4154_524F,
            dhcp_retries: 0,
            last_dhcp_ms: 0,
            ping_target: 0,
            ping_seq: 0,
            last_echo: 0,
            icmp_replies: 0,
            dns_txid: 0x1234,
            dns_ip: 0,
            dns_ready: false,
            dns_pending: false,
            rx_packets: 0,
            tx_packets: 0,
            arp_packets: 0,
            udp_packets: 0,
            tcp_packets: 0,
            ip_next_id: 1,
        });
        SOCKETS = Some((0..MAX_SOCKETS).map(|_| None).collect());
    }
    if present() {
        crate::kprintln!(
            "[net] link up, mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, IP {} (DHCP pending)",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5],
            ip_string(0x0A00_020F)
        );
        dhcp_discover();
    } else {
        crate::kprintln!("[net] no NIC; network disabled");
    }
}

pub fn poll() {
    if !present() {
        return;
    }
    e1000::rx_poll();
    let now = crate::sync::uptime_ms();
    let st = state();
    if !st.dhcp_lease && now.saturating_sub(st.last_dhcp_ms) > 2000 && st.dhcp_retries < 5 {
        dhcp_discover();
    }
}

/* ── Checksums ───────────────────────────────────────────────────────────── */

fn checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/* ── ARP ─────────────────────────────────────────────────────────────────── */

fn arp_send_request(target: u32) {
    let st = state();
    let mut frame = Vec::with_capacity(64);
    frame.extend_from_slice(&[0xFF; 6]);
    frame.extend_from_slice(&st.mac);
    frame.extend_from_slice(&ETH_ARP.to_be_bytes());
    frame.extend_from_slice(&1u16.to_be_bytes()); // Ethernet
    frame.extend_from_slice(&ETH_IPV4.to_be_bytes());
    frame.push(6);
    frame.push(4);
    frame.extend_from_slice(&1u16.to_be_bytes()); // request
    frame.extend_from_slice(&st.mac);
    frame.extend_from_slice(&ip_bytes(st.ip));
    frame.extend_from_slice(&[0; 6]);
    frame.extend_from_slice(&ip_bytes(target));
    if e1000::tx(&frame) {
        state().tx_packets += 1;
    }
    state().arp_packets += 1;
}

pub fn arp_lookup(ip: u32) -> Option<[u8; 6]> {
    let st = state();
    for entry in st.arp.iter() {
        if entry.valid && entry.ip == ip {
            return Some([
                entry.mac[0],
                entry.mac[1],
                entry.mac[2],
                entry.mac[3],
                entry.mac[4],
                entry.mac[5],
            ]);
        }
    }
    None
}

fn arp_store(ip: u32, mac: [u8; 6]) {
    let st = state();
    for entry in st.arp.iter_mut() {
        if entry.valid && entry.ip == ip {
            entry.mac[..6].copy_from_slice(&mac);
            return;
        }
    }
    for entry in st.arp.iter_mut() {
        if !entry.valid {
            entry.valid = true;
            entry.ip = ip;
            entry.mac[..6].copy_from_slice(&mac);
            st.arp_count += 1;
            return;
        }
    }
    let index = st.arp_count % 8;
    st.arp[index] = ArpEntry {
        ip,
        mac: [mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 0, 0],
        valid: true,
    };
    st.arp_count += 1;
}

fn handle_arp(data: &[u8]) {
    if data.len() < 28 {
        return;
    }
    let oper = u16::from_be_bytes([data[6], data[7]]);
    let sender_mac = [data[8], data[9], data[10], data[11], data[12], data[13]];
    let sender_ip = ip_from_bytes(&data[14..18]);
    let target_ip = ip_from_bytes(&data[24..28]);
    state().arp_packets += 1;
    if sender_ip != 0 {
        arp_store(sender_ip, sender_mac);
    }
    if oper == 1 && target_ip == state().ip {
        // Reply.
        let st = state();
        let mut frame = Vec::with_capacity(64);
        frame.extend_from_slice(&sender_mac);
        frame.extend_from_slice(&st.mac);
        frame.extend_from_slice(&ETH_ARP.to_be_bytes());
        frame.extend_from_slice(&1u16.to_be_bytes());
        frame.extend_from_slice(&ETH_IPV4.to_be_bytes());
        frame.push(6);
        frame.push(4);
        frame.extend_from_slice(&2u16.to_be_bytes());
        frame.extend_from_slice(&st.mac);
        frame.extend_from_slice(&ip_bytes(st.ip));
        frame.extend_from_slice(&sender_mac);
        frame.extend_from_slice(&ip_bytes(sender_ip));
        if e1000::tx(&frame) {
            state().tx_packets += 1;
        }
    }
}

/* ── IPv4 ────────────────────────────────────────────────────────────────── */

fn ip_send(dst_ip: u32, proto: u8, payload: &[u8]) -> bool {
    if !present() {
        return false;
    }
    let dst_mac = match arp_lookup(dst_ip) {
        Some(mac) => mac,
        None => {
            arp_send_request(dst_ip);
            [0xFF; 6]
        }
    };
    let st = state();
    let total = 20 + payload.len();
    let mut frame = Vec::with_capacity(14 + total);
    frame.extend_from_slice(&dst_mac);
    frame.extend_from_slice(&st.mac);
    frame.extend_from_slice(&ETH_IPV4.to_be_bytes());

    let mut ip = Vec::with_capacity(20);
    ip.push(0x45);
    ip.push(0);
    ip.extend_from_slice(&(total as u16).to_be_bytes());
    ip.extend_from_slice(&st.ip_next_id.to_be_bytes());
    st.ip_next_id = st.ip_next_id.wrapping_add(1);
    ip.extend_from_slice(&0x4000u16.to_be_bytes()); // don't fragment
    ip.push(64);
    ip.push(proto);
    ip.extend_from_slice(&[0, 0]);
    ip.extend_from_slice(&ip_bytes(st.ip));
    ip.extend_from_slice(&ip_bytes(dst_ip));
    let sum = checksum(&ip);
    ip[10] = (sum >> 8) as u8;
    ip[11] = sum as u8;
    frame.extend_from_slice(&ip);
    frame.extend_from_slice(payload);

    let ok = e1000::tx(&frame);
    if ok {
        state().tx_packets += 1;
    }
    ok
}

fn handle_ipv4(data: &[u8]) {
    if data.len() < 20 {
        return;
    }
    let version = data[0] >> 4;
    let header_len = ((data[0] & 0x0F) as usize) * 4;
    if version != 4 || header_len < 20 || data.len() < header_len {
        return;
    }
    let total_len = u16::from_be_bytes([data[2], data[3]]) as usize;
    if total_len < header_len || total_len > data.len() {
        return;
    }
    let src = ip_from_bytes(&data[12..16]);
    let dst = ip_from_bytes(&data[16..20]);
    if dst != state().ip && dst != 0xFFFF_FFFF {
        return;
    }
    let proto = data[9];
    let payload = &data[header_len..total_len];
    match proto {
        IP_ICMP => handle_icmp(src, payload),
        IP_UDP => handle_udp(src, payload),
        IP_TCP => handle_tcp(src, payload),
        _ => {}
    }
}

/* ── ICMP ────────────────────────────────────────────────────────────────── */

fn handle_icmp(src_ip: u32, data: &[u8]) {
    if data.len() < 8 {
        return;
    }
    let kind = data[0];
    let code = data[1];
    let seq = u16::from_be_bytes([data[6], data[7]]);
    if kind == 8 && code == 0 {
        // Echo request -> reply.
        let mut reply = Vec::with_capacity(data.len());
        reply.push(0);
        reply.push(0);
        reply.extend_from_slice(&[0, 0]);
        reply.extend_from_slice(&data[4..8]);
        reply.extend_from_slice(&data[8..]);
        let sum = checksum(&reply);
        reply[2] = (sum >> 8) as u8;
        reply[3] = sum as u8;
        ip_send(src_ip, IP_ICMP, &reply);
    } else if kind == 0 && code == 0 {
        let st = state();
        st.last_echo = seq;
        st.icmp_replies = st.icmp_replies.wrapping_add(1);
    }
}

pub fn ping(target: u32) {
    let target = if target == 0 { state().gateway } else { target };
    let st = state();
    st.ping_target = target;
    st.ping_seq = st.ping_seq.wrapping_add(1);
    let seq = st.ping_seq;
    let mut icmp = Vec::with_capacity(32);
    icmp.push(8);
    icmp.push(0);
    icmp.extend_from_slice(&[0, 0]);
    icmp.extend_from_slice(&0x4154u16.to_be_bytes());
    icmp.extend_from_slice(&seq.to_be_bytes());
    icmp.extend_from_slice(b"AtrodraX ping!");
    let sum = checksum(&icmp);
    icmp[2] = (sum >> 8) as u8;
    icmp[3] = sum as u8;
    // Ensure ARP is resolved first; a failed send simply retries next call.
    if arp_lookup(target).is_none() {
        arp_send_request(target);
    }
    ip_send(target, IP_ICMP, &icmp);
}

pub fn last_echo() -> u16 {
    state().last_echo
}

pub fn icmp_replies() -> u32 {
    state().icmp_replies
}

/* ── UDP ─────────────────────────────────────────────────────────────────── */

fn udp_send(src_port: u16, dst_ip: u32, dst_port: u16, data: &[u8]) -> bool {
    let st = state();
    let mut udp = Vec::with_capacity(8 + data.len());
    udp.extend_from_slice(&src_port.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&((8 + data.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]);
    udp.extend_from_slice(data);
    // UDP checksum is optional in IPv4; zero is valid.
    let _ = st;
    ip_send(dst_ip, IP_UDP, &udp)
}

fn handle_udp(src_ip: u32, data: &[u8]) {
    if data.len() < 8 {
        return;
    }
    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let len = u16::from_be_bytes([data[4], data[5]]) as usize;
    if len < 8 || len > data.len() {
        return;
    }
    let payload = &data[8..len];
    state().udp_packets += 1;

    if dst_port == DHCP_CLIENT_PORT {
        dhcp_receive(payload);
    }
    if src_port == DNS_PORT && state().dns_pending {
        dns_receive(payload);
    }

    let list = sockets();
    for slot in list.iter_mut() {
        if let Some(sock) = slot {
            if sock.used && sock.kind == SOCK_DGRAM && sock.local_port == dst_port {
                let space = RX_CAP.saturating_sub(sock.rx.len());
                let take = payload.len().min(space);
                sock.rx.extend_from_slice(&payload[..take]);
                sock.src_ip = src_ip;
                sock.src_port = src_port;
                if sock.remote_ip == 0 {
                    sock.remote_ip = src_ip;
                    sock.remote_port = src_port;
                }
                return;
            }
        }
    }
}

/* ── DHCP ────────────────────────────────────────────────────────────────── */

fn dhcp_discover() {
    if !present() {
        return;
    }
    {
        let st = state();
        st.last_dhcp_ms = crate::sync::uptime_ms();
        st.dhcp_retries += 1;
    }
    let txid = state().dhcp_txid;
    let mut packet = Vec::with_capacity(300);
    packet.push(1); // BOOTREQUEST
    packet.push(1); // Ethernet
    packet.push(6);
    packet.push(0);
    packet.extend_from_slice(&txid.to_be_bytes());
    packet.extend_from_slice(&[0; 4]); // secs, flags
    packet.extend_from_slice(&[0; 4]); // ciaddr
    packet.extend_from_slice(&[0; 4]); // yiaddr
    packet.extend_from_slice(&[0; 4]); // siaddr
    packet.extend_from_slice(&[0; 4]); // giaddr
    packet.extend_from_slice(&state().mac);
    packet.extend_from_slice(&[0; 10]); // chaddr padding
    packet.extend_from_slice(&[0; 64]); // sname
    packet.extend_from_slice(&[0; 128]); // file
    packet.extend_from_slice(&[99, 130, 83, 99]); // magic cookie
    packet.extend_from_slice(&[53, 1, 1]); // DHCP DISCOVER
    packet.extend_from_slice(&[55, 4, 1, 3, 6, 15]); // parameter request list
    packet.extend_from_slice(&[12, 8]);
    packet.extend_from_slice(b"atrodrax");
    packet.push(255);
    udp_send(DHCP_CLIENT_PORT, 0xFFFF_FFFF, DHCP_SERVER_PORT, &packet);
    crate::kprintln!("[dhcp] DISCOVER sent (attempt {})", state().dhcp_retries);
}

fn dhcp_receive(payload: &[u8]) {
    if payload.len() < 240 {
        return;
    }
    if payload[0] != 2 {
        return;
    }
    let txid = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if txid != state().dhcp_txid {
        return;
    }
    let yiaddr = ip_from_bytes(&payload[16..20]);
    let mut message_type = 0u8;
    let mut router = 0u32;
    let mut dns = 0u32;
    let mut mask = 0u32;
    let mut server = 0u32;
    let mut index = 240;
    while index < payload.len() {
        let option = payload[index];
        if option == 255 {
            break;
        }
        if option == 0 {
            index += 1;
            continue;
        }
        if index + 1 >= payload.len() {
            break;
        }
        let len = payload[index + 1] as usize;
        if index + 2 + len > payload.len() {
            break;
        }
        let value = &payload[index + 2..index + 2 + len];
        match option {
            53 if len >= 1 => message_type = value[0],
            1 if len >= 4 => mask = ip_from_bytes(value),
            3 if len >= 4 => router = ip_from_bytes(value),
            6 if len >= 4 => dns = ip_from_bytes(value),
            54 if len >= 4 => server = ip_from_bytes(value),
            _ => {}
        }
        index += 2 + len;
    }

    match message_type {
        2 => {
            // OFFER -> REQUEST
            crate::kprintln!("[dhcp] OFFER {}", ip_string(yiaddr));
            let txid = state().dhcp_txid;
            let mac = state().mac;
            let mut packet = Vec::with_capacity(300);
            packet.push(1);
            packet.push(1);
            packet.push(6);
            packet.push(0);
            packet.extend_from_slice(&txid.to_be_bytes());
            packet.extend_from_slice(&[0; 4]);
            packet.extend_from_slice(&[0; 4]);
            packet.extend_from_slice(&[0; 4]);
            packet.extend_from_slice(&[0; 4]);
            packet.extend_from_slice(&[0; 4]);
            packet.extend_from_slice(&mac);
            packet.extend_from_slice(&[0; 10]);
            packet.extend_from_slice(&[0; 64]);
            packet.extend_from_slice(&[0; 128]);
            packet.extend_from_slice(&[99, 130, 83, 99]);
            packet.extend_from_slice(&[53, 1, 3]); // REQUEST
            packet.extend_from_slice(&[50, 4]);
            packet.extend_from_slice(&ip_bytes(yiaddr));
            packet.extend_from_slice(&[54, 4]);
            packet.extend_from_slice(&ip_bytes(server));
            packet.push(255);
            udp_send(DHCP_CLIENT_PORT, 0xFFFF_FFFF, DHCP_SERVER_PORT, &packet);
        }
        5 => {
            let st = state();
            st.ip = yiaddr;
            if router != 0 {
                st.gateway = router;
            }
            if dns != 0 {
                st.dns = dns;
            }
            if mask != 0 {
                st.netmask = mask;
            }
            st.dhcp_lease = true;
            st.arp_count = 0;
            for entry in st.arp.iter_mut() {
                entry.valid = false;
            }
            crate::kprintln!(
                "[dhcp] ACK ip={} gw={} dns={}",
                ip_string(st.ip),
                ip_string(st.gateway),
                ip_string(st.dns)
            );
        }
        _ => {}
    }
}

/* ── DNS ─────────────────────────────────────────────────────────────────── */

pub fn dns_lookup(host: &str) -> bool {
    if !present() || host.is_empty() {
        return false;
    }
    let st = state();
    st.dns_pending = true;
    st.dns_ready = false;
    st.dns_txid = st.dns_txid.wrapping_add(0x0101);
    let txid = st.dns_txid;
    let server = st.dns;
    let mut packet = Vec::with_capacity(64 + host.len());
    packet.extend_from_slice(&txid.to_be_bytes());
    packet.extend_from_slice(&0x0100u16.to_be_bytes()); // recursion desired
    packet.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    packet.extend_from_slice(&0u16.to_be_bytes());
    for label in host.split('.') {
        if label.len() > 63 {
            return false;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&1u16.to_be_bytes()); // A
    packet.extend_from_slice(&1u16.to_be_bytes()); // IN
    udp_send(0xC000 | (crate::sync::uptime_ms() as u16 & 0x3FFF), server, DNS_PORT, &packet);
    true
}

fn dns_receive(data: &[u8]) {
    let st = state();
    if data.len() < 12 {
        return;
    }
    let txid = u16::from_be_bytes([data[0], data[1]]);
    if txid != st.dns_txid {
        return;
    }
    let questions = u16::from_be_bytes([data[4], data[5]]) as usize;
    let answers = u16::from_be_bytes([data[6], data[7]]) as usize;
    let mut index = 12;
    for _ in 0..questions {
        while index < data.len() && data[index] != 0 {
            index += 1 + data[index] as usize;
        }
        index += 1 + 4;
    }
    for _ in 0..answers {
        if index >= data.len() {
            break;
        }
        if data[index] & 0xC0 == 0xC0 {
            index += 2;
        } else {
            while index < data.len() && data[index] != 0 {
                index += 1 + data[index] as usize;
            }
            index += 1;
        }
        if index + 10 > data.len() {
            break;
        }
        let kind = u16::from_be_bytes([data[index], data[index + 1]]);
        let class = u16::from_be_bytes([data[index + 2], data[index + 3]]);
        let rdlength = u16::from_be_bytes([data[index + 8], data[index + 9]]) as usize;
        index += 10;
        if kind == 1 && class == 1 && rdlength >= 4 && index + 4 <= data.len() {
            st.dns_ip = ip_from_bytes(&data[index..index + 4]);
            st.dns_ready = true;
            st.dns_pending = false;
            return;
        }
        index += rdlength;
    }
    st.dns_pending = false;
}

pub fn dns_result() -> (bool, u32) {
    let st = state();
    (st.dns_ready, st.dns_ip)
}

pub fn dns_reset() {
    let st = state();
    st.dns_ready = false;
    st.dns_pending = false;
    st.dns_ip = 0;
}

/* ── TCP (minimal active open) ───────────────────────────────────────────── */

fn tcp_send_segment(
    dst_ip: u32,
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> bool {
    let mut tcp = Vec::with_capacity(20 + payload.len());
    tcp.extend_from_slice(&src_port.to_be_bytes());
    tcp.extend_from_slice(&dst_port.to_be_bytes());
    tcp.extend_from_slice(&seq.to_be_bytes());
    tcp.extend_from_slice(&ack.to_be_bytes());
    tcp.push(5 << 4);
    tcp.push(flags);
    tcp.extend_from_slice(&8192u16.to_be_bytes()); // window
    tcp.extend_from_slice(&[0, 0]); // checksum placeholder
    tcp.extend_from_slice(&[0, 0]); // urgent
    tcp.extend_from_slice(payload);

    let src = ip_bytes(state().ip);
    let dst = ip_bytes(dst_ip);
    let mut pseudo = Vec::with_capacity(12 + tcp.len());
    pseudo.extend_from_slice(&src);
    pseudo.extend_from_slice(&dst);
    pseudo.push(0);
    pseudo.push(IP_TCP);
    pseudo.extend_from_slice(&(tcp.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(&tcp);
    let sum = checksum(&pseudo);
    tcp[16] = (sum >> 8) as u8;
    tcp[17] = sum as u8;
    ip_send(dst_ip, IP_TCP, &tcp)
}

fn handle_tcp(src_ip: u32, data: &[u8]) {
    if data.len() < 20 {
        return;
    }
    let src_port = u16::from_be_bytes([data[0], data[1]]);
    let dst_port = u16::from_be_bytes([data[2], data[3]]);
    let seq = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let ack = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let header_len = ((data[12] >> 4) as usize) * 4;
    if header_len < 20 || header_len > data.len() {
        return;
    }
    let flags = data[13];
    let payload = &data[header_len..];
    state().tcp_packets += 1;

    let list = sockets();
    for slot in list.iter_mut() {
        let sock = match slot {
            Some(s) if s.used && s.kind == SOCK_STREAM => s,
            _ => continue,
        };
        if sock.local_port != dst_port {
            continue;
        }
        let syn = flags & 0x02 != 0;
        let ackf = flags & 0x10 != 0;
        let fin = flags & 0x01 != 0;

        if sock.state == SOCK_STATE_LISTENING && syn && !ackf {
            let mut child = Socket::new(SOCK_STREAM);
            child.local_port = dst_port;
            child.remote_ip = src_ip;
            child.remote_port = src_port;
            child.seq = 0x2000_0000;
            child.ack = seq.wrapping_add(1);
            child.state = SOCK_STATE_SYN_RECV;
            tcp_send_segment(
                src_ip,
                dst_port,
                src_port,
                child.seq,
                child.ack,
                0x12,
                &[],
            );
            child.seq = child.seq.wrapping_add(1);
            sock.pending_syn = Some((src_ip, src_port, child.ack));
            return;
        }

        if sock.state == SOCK_STATE_SYN_SENT && syn && ackf {
            sock.ack = seq.wrapping_add(1);
            sock.state = SOCK_STATE_CONNECTED;
            tcp_send_segment(src_ip, dst_port, src_port, sock.seq, sock.ack, 0x10, &[]);
            return;
        }

        if sock.state == SOCK_STATE_SYN_RECV && ackf && !syn {
            sock.state = SOCK_STATE_CONNECTED;
            return;
        }

        if sock.state == SOCK_STATE_CONNECTED {
            if fin {
                sock.ack = seq.wrapping_add(1);
                tcp_send_segment(src_ip, dst_port, src_port, sock.seq, sock.ack, 0x10, &[]);
                sock.state = SOCK_STATE_CLOSED;
                return;
            }
            if !payload.is_empty() {
                if seq == sock.ack {
                    let space = RX_CAP.saturating_sub(sock.rx.len());
                    let take = payload.len().min(space);
                    sock.rx.extend_from_slice(&payload[..take]);
                    sock.ack = sock.ack.wrapping_add(take as u32);
                    sock.src_ip = src_ip;
                    sock.src_port = src_port;
                }
                tcp_send_segment(
                    src_ip,
                    dst_port,
                    src_port,
                    sock.seq,
                    sock.ack,
                    0x10,
                    &[],
                );
            }
            return;
        }
    }
}

/* ── Socket API ─────────────────────────────────────────────────────────── */

pub fn socket_create(family: i32, kind: i32, _protocol: i32) -> i32 {
    if family != AF_INET || (kind != SOCK_STREAM && kind != SOCK_DGRAM) {
        return -1;
    }
    let list = sockets();
    for (index, slot) in list.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(Socket::new(kind));
            return index as i32;
        }
    }
    -1
}

fn socket(slot: i32) -> Option<&'static mut Socket> {
    if slot < 0 || slot as usize >= MAX_SOCKETS {
        return None;
    }
    sockets()[slot as usize].as_mut()
}

fn ephemeral_port() -> u16 {
    49152 + (crate::sync::uptime_ms() as u16 % 16000)
}

pub fn socket_bind(slot: i32, _ip: u32, port: u16) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    sock.local_port = if port == 0 { ephemeral_port() } else { port };
    0
}

pub fn socket_connect(slot: i32, ip: u32, port: u16) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    if sock.local_port == 0 {
        sock.local_port = ephemeral_port();
    }
    sock.remote_ip = ip;
    sock.remote_port = port;
    match sock.kind {
        SOCK_DGRAM => {
            sock.state = SOCK_STATE_CONNECTED;
            0
        }
        _ => {
            sock.seq = 0x3141_0000u32.wrapping_add(crate::sync::uptime_ms() as u32);
            sock.ack = 0;
            sock.state = SOCK_STATE_SYN_SENT;
            if tcp_send_segment(ip, sock.local_port, port, sock.seq, 0, 0x02, &[]) {
                sock.seq = sock.seq.wrapping_add(1);
                0
            } else {
                -1
            }
        }
    }
}

pub fn socket_listen(slot: i32, _backlog: i32) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    if sock.local_port == 0 {
        sock.local_port = ephemeral_port();
    }
    sock.state = SOCK_STATE_LISTENING;
    0
}

pub fn socket_accept(slot: i32) -> (i32, u32, u16) {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return (-1, 0, 0),
    };
    if sock.state == SOCK_STATE_SYN_RECV || sock.state == SOCK_STATE_CONNECTED {
        let ip = sock.remote_ip;
        let port = sock.remote_port;
        sock.state = SOCK_STATE_CONNECTED;
        return (slot, ip, port);
    }
    (-1, 0, 0)
}

pub fn socket_send(slot: i32, data: &[u8]) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    if sock.kind == SOCK_DGRAM {
        let ip = sock.remote_ip;
        let port = sock.remote_port;
        if ip == 0 || port == 0 {
            return -1;
        }
        return if udp_send(sock.local_port, ip, port, data) {
            data.len() as i32
        } else {
            -1
        };
    }
    if sock.state != SOCK_STATE_CONNECTED {
        return -1;
    }
    if tcp_send_segment(
        sock.remote_ip,
        sock.local_port,
        sock.remote_port,
        sock.seq,
        sock.ack,
        0x18, // PSH | ACK
        data,
    ) {
        sock.seq = sock.seq.wrapping_add(data.len() as u32);
        data.len() as i32
    } else {
        -1
    }
}

pub fn socket_sendto(slot: i32, data: &[u8], ip: u32, port: u16) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    if sock.local_port == 0 {
        sock.local_port = ephemeral_port();
    }
    if sock.kind == SOCK_DGRAM {
        if udp_send(sock.local_port, ip, port, data) {
            return data.len() as i32;
        }
        return -1;
    }
    -1
}

pub fn socket_recv(slot: i32, max: usize) -> (i32, u32, u16) {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return (-1, 0, 0),
    };
    if sock.rx.is_empty() {
        return (0, sock.src_ip, sock.src_port);
    }
    let take = max.min(sock.rx.len());
    let result = (take as i32, sock.src_ip, sock.src_port);
    sock.rx.drain(..take);
    result
}

pub fn socket_peek(slot: i32) -> usize {
    socket(slot).map(|s| s.rx.len()).unwrap_or(0)
}

pub fn socket_read_into(slot: i32, buffer: &mut [u8]) -> i32 {
    let sock = match socket(slot) {
        Some(s) => s,
        None => return -1,
    };
    if sock.rx.is_empty() {
        return 0;
    }
    let take = buffer.len().min(sock.rx.len());
    buffer[..take].copy_from_slice(&sock.rx[..take]);
    sock.rx.drain(..take);
    take as i32
}

pub fn socket_close(slot: i32) -> i32 {
    if slot < 0 || slot as usize >= MAX_SOCKETS {
        return -1;
    }
    let list = sockets();
    if let Some(existing) = list[slot as usize].take() {
        if existing.state == SOCK_STATE_CONNECTED && existing.kind == SOCK_STREAM {
            let _ = tcp_send_segment(
                existing.remote_ip,
                existing.local_port,
                existing.remote_port,
                existing.seq,
                existing.ack,
                0x11, // FIN | ACK
                &[],
            );
        }
        0
    } else {
        -1
    }
}

pub fn socket_state(slot: i32) -> i32 {
    socket(slot).map(|s| s.state).unwrap_or(-1)
}

pub fn socket_src_ip(slot: i32) -> u32 {
    socket(slot).map(|s| s.src_ip).unwrap_or(0)
}

pub fn socket_src_port(slot: i32) -> u16 {
    socket(slot).map(|s| s.src_port).unwrap_or(0)
}

/* ── SYS_NET decoding ───────────────────────────────────────────────────── */

pub fn syscall(cmd: u64, arg: u64) -> u64 {
    match cmd {
        0 => (present() as u64) | ((state().icmp_replies as u64) << 32),
        1 => {
            ping(arg as u32);
            0
        }
        2 => last_echo() as u64,
        3 => (state().dhcp_lease as u64) | ((state().ip as u64) << 32),
        4 => state().gateway as u64,
        5 => state().dns as u64,
        6 => {
            // arg is a user pointer; the caller resolves and passes a copy via
            // dns_lookup_str. Implemented in syscall.rs for user memory safety.
            0
        }
        7 => {
            let (ready, ip) = dns_result();
            (ready as u64) | ((ip as u64) << 32)
        }
        8 => {
            dns_reset();
            0
        }
        9 => crate::sync::uptime_ms(),
        _ => u64::MAX,
    }
}

/* ── Ethernet entry ─────────────────────────────────────────────────────── */

pub fn receive(frame: &[u8]) {
    if frame.len() < 14 {
        return;
    }
    state().rx_packets += 1;
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let payload = &frame[14..];
    match ethertype {
        ETH_ARP => handle_arp(payload),
        ETH_IPV4 => handle_ipv4(payload),
        _ => {}
    }
}

pub fn print_info() {
    if !present() {
        crate::kprintln!("[net] no NIC");
        return;
    }
    let st = state();
    crate::kprintln!(
        "[net] ip={} gw={} dns={} dhcp={}",
        ip_string(st.ip),
        ip_string(st.gateway),
        ip_string(st.dns),
        st.dhcp_lease
    );
    crate::kprintln!(
        "[net] rx={} tx={} arp={} icmp={} udp={} tcp={}",
        st.rx_packets,
        st.tx_packets,
        st.arp_packets,
        st.icmp_replies,
        st.udp_packets,
        st.tcp_packets
    );
    for entry in st.arp.iter() {
        if entry.valid {
            crate::kprintln!(
                "[net] arp {} -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                ip_string(entry.ip),
                entry.mac[0],
                entry.mac[1],
                entry.mac[2],
                entry.mac[3],
                entry.mac[4],
                entry.mac[5]
            );
        }
    }
}

pub fn stats() -> (u64, u64, u64, u64, u64) {
    let st = state();
    (
        st.rx_packets,
        st.tx_packets,
        st.icmp_replies as u64,
        st.udp_packets,
        st.tcp_packets,
    )
}
