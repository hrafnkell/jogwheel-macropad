//! USB serial link to the pad (JSON lines, see src/host.cpp).
//!
//! A background thread finds the pad, opens it (on Linux non-exclusively, so
//! firmware uploads and the headless auto-switcher still work; Windows COM
//! ports are always exclusive), reads JSON lines and
//! reconnects after unplugging. Writes go through a per-connection writer
//! thread so the UI never blocks on the port.
use serde_json::Value;
use std::io::{ErrorKind, Read, Write};
use serialport::SerialPort;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Raspberry Pi's USB vendor ID (the Pico's CDC serial port)
#[cfg(not(target_os = "linux"))]
pub const PICO_VID: u16 = 0x2E8A;

/// The pad's serial port name, if plugged in.
#[cfg(target_os = "linux")]
pub fn find_port() -> Option<String> {
    // Stable names, and no libudev needed
    let mut v: Vec<String> = std::fs::read_dir("/dev/serial/by-id")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with("usb-Raspberry_Pi_Pico_") && n.ends_with("-if00")
        })
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    v.sort();
    v.into_iter().next()
}
#[cfg(not(target_os = "linux"))]
pub fn find_port() -> Option<String> {
    let mut v: Vec<String> = serialport::available_ports()
        .ok()?
        .into_iter()
        .filter(|p| matches!(&p.port_type, serialport::SerialPortType::UsbPort(u) if u.vid == PICO_VID))
        .map(|p| p.port_name)
        .collect();
    v.sort();
    v.into_iter().next()
}

/// Short name for display ("COM5", or the by-id file name)
pub fn port_label(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

#[derive(Debug)]
pub enum SerialEvent {
    Connected(String),
    Disconnected,
    Message(Value),
    Log(String),
}

#[derive(Clone)]
pub struct SerialLink {
    out: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    enabled: Arc<AtomicBool>,
}

impl SerialLink {
    /// Start the connection manager. Events arrive on the returned channel.
    pub fn start() -> (SerialLink, futures::channel::mpsc::UnboundedReceiver<SerialEvent>) {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let link = SerialLink { out: Arc::new(Mutex::new(None)), enabled: Arc::new(AtomicBool::new(true)) };
        let l = link.clone();
        std::thread::Builder::new().name("serial".into()).spawn(move || l.run(tx)).expect("serial thread");
        (link, rx)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    pub fn connected(&self) -> bool {
        self.out.lock().unwrap().is_some()
    }

    /// Queue bytes for the pad. Returns false when not connected.
    pub fn send(&self, data: Vec<u8>) -> bool {
        match self.out.lock().unwrap().as_ref() {
            Some(tx) => tx.send(data).is_ok(),
            None => false,
        }
    }
    pub fn send_line(&self, line: &str) -> bool {
        self.send(format!("{}\n", line.replace('\n', " ")).into_bytes())
    }

    fn run(&self, tx: futures::channel::mpsc::UnboundedSender<SerialEvent>) {
        loop {
            if !self.enabled() {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }
            let Some(path) = find_port() else {
                std::thread::sleep(Duration::from_millis(700));
                continue;
            };
            match open(&path) {
                Ok(port) => {
                    self.session(port, &path, &tx);
                    *self.out.lock().unwrap() = None;
                    if tx.unbounded_send(SerialEvent::Disconnected).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.unbounded_send(SerialEvent::Log(format!("open {path}: {e}")));
                    std::thread::sleep(Duration::from_millis(1500));
                }
            }
        }
    }

    fn session(&self, mut port: Box<dyn SerialPort>, path: &str, tx: &futures::channel::mpsc::UnboundedSender<SerialEvent>) {
        let Ok(mut writer) = port.try_clone() else { return };
        let (wtx, wrx) = mpsc::channel::<Vec<u8>>();
        let failed = Arc::new(AtomicBool::new(false));
        let f2 = failed.clone();
        std::thread::spawn(move || {
            for buf in wrx {
                // Small chunks: the pad reads PUT payloads into a fixed buffer.
                for chunk in buf.chunks(512) {
                    if writer.write_all(chunk).and_then(|_| writer.flush()).is_err() {
                        f2.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            }
        });
        *self.out.lock().unwrap() = Some(wtx);
        let _ = tx.unbounded_send(SerialEvent::Connected(path.to_string()));

        let mut buf = [0u8; 4096];
        let mut line: Vec<u8> = Vec::new();
        while self.enabled() && !failed.load(Ordering::SeqCst) {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    for &b in &buf[..n] {
                        if b == b'\n' {
                            let s = String::from_utf8_lossy(&line).trim().to_string();
                            line.clear();
                            if s.starts_with('{') {
                                match serde_json::from_str::<Value>(&s) {
                                    Ok(v) => {
                                        let _ = tx.unbounded_send(SerialEvent::Message(v));
                                    }
                                    Err(_) => {
                                        let _ = tx.unbounded_send(SerialEvent::Log(s));
                                    }
                                }
                            } else if !s.is_empty() {
                                let _ = tx.unbounded_send(SerialEvent::Log(s));
                            }
                        } else if line.len() < 256 * 1024 {
                            line.push(b);
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
            #[cfg(unix)]
            if !std::path::Path::new(path).exists() {
                break; // unplugged (reads can keep timing out instead of failing)
            }
        }
        if !self.enabled() {
            // Leave the pad in its normal state: no telemetry, HID output on.
            let _ = port.write_all(b"MONITOR 0\n");
            let _ = port.flush();
        }
    }
}

fn open(path: &str) -> serialport::Result<Box<dyn SerialPort>> {
    // 115200 matters: closing the port at 1200 baud reboots the Pico into its bootloader.
    let b = serialport::new(path, 115_200).timeout(Duration::from_millis(100));
    #[cfg(unix)]
    let mut p: Box<dyn SerialPort> = {
        let mut p = b.open_native()?;
        p.set_exclusive(false)?;
        Box::new(p)
    };
    #[cfg(not(unix))]
    let mut p = b.open()?;
    // The pad only talks to a host that raises DTR (Linux does on open, Windows doesn't)
    p.write_data_terminal_ready(true)?;
    Ok(p)
}

/// Open the port, write one line and close it again - used by the headless
/// auto-switcher so it never holds the port or steals replies.
pub fn send_once(line: &str) -> bool {
    let Some(path) = find_port() else { return false };
    let Ok(mut p) = open(&path) else { return false };
    let ok = p.write_all(format!("{}\n", line.replace('\n', " ")).as_bytes()).and_then(|_| p.flush()).is_ok();
    std::thread::sleep(Duration::from_millis(30));
    ok
}
