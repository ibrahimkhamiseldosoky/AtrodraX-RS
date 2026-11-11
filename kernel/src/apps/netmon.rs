//! Network monitor app.

use crate::gfx::window;
use crate::net;
use alloc::format;
use alloc::string::String;

const BG: u32 = 0x0B101A;
const TEXT: u32 = 0xE2E8F0;
const ACCENT: u32 = 0x8BE9FD;
const MUTED: u32 = 0x64748B;
const OK: u32 = 0x00FF41;
const WARN: u32 = 0xFFD166;

pub struct Netmon {
    pub win: Option<u32>,
    last_refresh: u64,
    last_ping: u64,
    message: String,
}

static mut NETMON: Option<Netmon> = None;

pub fn init() {
    unsafe {
        NETMON = Some(Netmon {
            win: None,
            last_refresh: 0,
            last_ping: 0,
            message: String::new(),
        });
    }
}

fn netmon() -> &'static mut Netmon {
    unsafe { NETMON.as_mut().expect("netmon not initialized") }
}

fn line(id: u32, y: &mut i32, text: alloc::string::String, color: u32) {
    window::draw_text(id, 12, *y, &text, color, BG);
    *y += 16;
}

impl Netmon {
    pub fn render(&mut self) {
        let Some(id) = self.win else { return };
        if !window::exists(id) {
            self.win = None;
            return;
        }
        let width = window::get(id).map(|w| w.w).unwrap_or(520);
        window::clear(id, BG);
        window::draw_text(id, 12, 8, "Network Monitor", ACCENT, BG);
        window::fill_rect(id, 12, 30, width - 24, 1, 0x1E2D3D);

        let mac = net::mac();
        let mut y = 40;
        line(
            id,
            &mut y,
            format!(
                "MAC  {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            ),
            TEXT,
        );
        line(id, &mut y, format!("IP   {}", net::ip_string(net::local_ip())), if net::present() { TEXT } else { WARN });
        line(id, &mut y, format!("GW   {}", net::ip_string(net::gateway())), TEXT);
        line(id, &mut y, format!("DNS  {}", net::ip_string(net::dns_server())), TEXT);
        line(id, &mut y, format!("Link {}", if net::present() { "up (e1000)" } else { "down" }), if net::present() { OK } else { WARN });
        y += 4;
        window::draw_text(id, 12, y, "TRAFFIC", ACCENT, BG);
        y += 16;
        let (rx, tx, icmp, udp, tcp) = net::stats();
        line(id, &mut y, format!("RX packets      {}", rx), TEXT);
        line(id, &mut y, format!("TX packets      {}", tx), TEXT);
        line(id, &mut y, format!("ICMP replies    {}", icmp), TEXT);
        line(id, &mut y, format!("UDP packets     {}", udp), TEXT);
        line(id, &mut y, format!("TCP segments    {}", tcp), TEXT);
        line(id, &mut y, format!("Last ICMP echo  {}", net::last_echo()), TEXT);
        y += 4;

        window::fill_rect(id, 12, y, 130, 22, 0x123041);
        window::rect_outline(id, 12, y, 130, 22, 0x2A6A80);
        window::draw_text(id, 20, y + 7, "P  ping gateway", ACCENT, 0x123041);
        y += 28;

        if !self.message.is_empty() {
            window::draw_text(id, 12, y, &self.message, OK, BG);
        }
        window::present(id);
        self.last_refresh = crate::sync::uptime_ms();
    }

    fn ping_gateway(&mut self) {
        let gateway = net::gateway();
        if gateway != 0 {
            net::ping(gateway);
            self.message = format!("ping sent to {}", net::ip_string(gateway));
        } else {
            self.message = String::from("no gateway (DHCP pending)");
        }
        self.last_ping = crate::sync::uptime_ms();
        self.render();
    }
}

pub fn open() -> u32 {
    let n = netmon();
    if let Some(id) = n.win {
        if window::exists(id) {
            window::focus(id);
            n.render();
            return id;
        }
        n.win = None;
    }
    let id = window::create(260, 90, 520, 420, "Network", window::KIND_NETMON, BG);
    n.win = Some(id);
    n.render();
    id
}

pub fn window_id() -> Option<u32> {
    let n = netmon();
    match n.win {
        Some(id) if window::exists(id) => Some(id),
        _ => None,
    }
}

pub fn close_window(id: u32) {
    let n = netmon();
    if n.win == Some(id) {
        n.win = None;
    }
}

pub fn handle_key_pub(key: u16) {
    let n = netmon();
    match key {
        0x70 => n.ping_gateway(),
        0x64 => {
            n.message = String::from("resolving example.com...");
            net::dns_lookup("example.com");
            n.render();
        }
        _ => {}
    }
}

pub fn handle_click_pub(local_x: i32, local_y: i32) {
    if local_y >= 204 && local_y <= 226 && local_x >= 12 && local_x <= 142 {
        netmon().ping_gateway();
    }
}

pub fn tick() {
    let n = netmon();
    if n.win.is_none() {
        return;
    }
    let now = crate::sync::uptime_ms();
    if now.saturating_sub(n.last_refresh) >= 500 {
        n.render();
    }
    if n.last_ping > 0 && now.saturating_sub(n.last_ping) > 2000 && !n.message.is_empty() {
        let mut text = String::from("ping sent, replies: ");
        text.push_str(&format!("{}", net::icmp_replies()));
        n.message = text;
        n.last_ping = 0;
    }
}
