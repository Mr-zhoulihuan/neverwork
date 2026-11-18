//! 最小 WebSocket 客户端：只服务本机（127.0.0.1）的 Chrome DevTools Protocol 连接。
//!
//! 为什么手写而不是引依赖：整个协议只用到「客户端发掩码文本帧、服务端回未掩码
//! 文本帧」这一小片，外加 ping/pong 与分片；引一个完整的 ws 库要连带异步运行时。
//! 这里刻意不做：
//! - 不校验 `Sec-WebSocket-Accept`（仅回环地址，握手对端必然是本机浏览器）；
//! - 不支持二进制消息（CDP 全走 JSON 文本）；
//! - 不做 TLS（`ws://` 回环）。
//!
//! 超长消息（>16MB）直接报错：CDP 回复再大也不该到这个量级，
//! 真到了说明对端异常，继续读只会把内存吃光。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;

/// 单条消息上限。
const MAX_MESSAGE: usize = 16 * 1024 * 1024;

const OP_CONT: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// 握手后读超时：CDP 命令本身有自己的超时，这里只防「连上了但永远不说话」。
pub struct WsReader {
    stream: TcpStream,
    /// 握手响应多读出来的字节（服务器可能把第一帧和 101 响应一起发出）。
    pending: Vec<u8>,
    /// 与命令共用同一把写锁：回 pong 不能和业务帧交错。
    writer: Arc<Mutex<WsWriter>>,
}

/// 只负责写的一侧：与 `WsReader` 共享同一 TCP 连接。
pub struct WsWriter {
    stream: TcpStream,
}

/// 连接 `ws://host:port/path` 并完成握手，返回读写两半。
/// `timeout` 同时用作 TCP 连接与单次读写超时。
pub fn connect(
    host: &str,
    port: u16,
    path: &str,
    timeout: Duration,
) -> Result<(WsReader, Arc<Mutex<WsWriter>>), String> {
    let addr = format!("{host}:{port}");
    let stream = TcpStream::connect(&addr).map_err(|e| format!("连接 {addr} 失败：{e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;

    let key = base64::engine::general_purpose::STANDARD.encode(random_key());
    let req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    let mut s = stream
        .try_clone()
        .map_err(|e| format!("复制连接句柄失败：{e}"))?;
    s.write_all(req.as_bytes())
        .map_err(|e| format!("发送握手请求失败：{e}"))?;

    // 逐字节读到 \r\n\r\n，剩余的留给帧解析。
    let mut head = Vec::with_capacity(1024);
    let mut one = [0u8; 1];
    loop {
        match s.read(&mut one) {
            Ok(0) => return Err("握手时连接被关闭".into()),
            Ok(_) => {
                head.push(one[0]);
                if head.len() >= 4 && &head[head.len() - 4..] == b"\r\n\r\n" {
                    break;
                }
                if head.len() > 16 * 1024 {
                    return Err("握手响应头异常过大".into());
                }
            }
            Err(e) => return Err(format!("读握手响应失败：{e}")),
        }
    }
    let head_text = String::from_utf8_lossy(&head);
    let status_ok = head_text
        .lines()
        .next()
        .map(|l| l.contains(" 101 "))
        .unwrap_or(false);
    if !status_ok {
        return Err(format!("WebSocket 握手被拒绝：{}", head_text.trim()));
    }
    let reader_stream = stream
        .try_clone()
        .map_err(|e| format!("复制连接句柄失败：{e}"))?;
    // 握手完成后取消读超时：CDP 空闲几十秒是常态，读超时会把连接误判成断开。
    // 调用超时由 `Cdp::call` 自己控制，浏览器进程退出时 socket 会 EOF。
    let _ = reader_stream.set_read_timeout(None);
    let writer = Arc::new(Mutex::new(WsWriter { stream }));
    Ok((
        WsReader {
            stream: reader_stream,
            // 逐字节读握手响应，`head` 只含头部，不含任何帧数据。
            // 曾经把 `head` 当残留帧字节，结果读线程把 `HT` 当 close 帧退出。
            pending: Vec::new(),
            writer: writer.clone(),
        },
        writer,
    ))
}

impl WsReader {
    /// 读一条完整文本消息：内部跳过 pong、回应 ping、按续帧拼接分片。
    /// 连接关闭（收到 close 或 EOF）返回错误。
    pub fn read_message(&mut self) -> Result<String, String> {
        // ping 先攒着，读完这条消息后统一回 pong：读取期间不能再借 stream。
        let mut pongs: Vec<Vec<u8>> = Vec::new();
        let out = {
            let mut src = PendingReader {
                stream: &mut self.stream,
                pending: &mut self.pending,
            };
            read_message_from(&mut src, |payload| pongs.push(payload.to_vec()))
        };
        if !pongs.is_empty() {
            // 走同一把写锁：pong 与命令帧不能交错写入。
            if let Ok(mut w) = self.writer.lock() {
                for p in pongs {
                    let _ = w.send_pong(&p);
                }
            }
        }
        out
    }
}

impl WsWriter {
    fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let frame = encode_frame(opcode, payload, Some(random_key()));
        self.stream
            .write_all(&frame)
            .and_then(|_| self.stream.flush())
            .map_err(|e| format!("发送 WebSocket 帧失败：{e}"))
    }

    /// 发送一条文本消息（客户端帧必须掩码）。
    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.write_frame(OP_TEXT, text.as_bytes())
    }

    fn send_pong(&mut self, payload: &[u8]) -> Result<(), String> {
        self.write_frame(OP_PONG, payload)
    }

    /// 尽力发 close 帧；失败无所谓，调用方随后会丢弃连接。
    pub fn close(&mut self) {
        let frame = encode_frame(OP_CLOSE, &[], Some(random_key()));
        let _ = self.stream.write_all(&frame);
        let _ = self.stream.flush();
    }
}

/// 把「握手后残留字节 + 底层流」拼成一个连续读取器。
struct PendingReader<'a> {
    stream: &'a mut TcpStream,
    pending: &'a mut Vec<u8>,
}

impl Read for PendingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if !self.pending.is_empty() {
            let n = buf.len().min(self.pending.len());
            buf[..n].copy_from_slice(&self.pending[..n]);
            self.pending.drain(..n);
            return Ok(n);
        }
        self.stream.read(buf)
    }
}

/// 帧编码。`mask=None` 只用于测试里构造服务端帧；客户端消息一律带掩码。
fn encode_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    let len = payload.len();
    if len < 126 {
        out.push(mask_bit | len as u8);
    } else if len <= u16::MAX as usize {
        out.push(mask_bit | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(mask_bit | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    if let Some(key) = mask {
        out.extend_from_slice(&key);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// 解析单帧。`mask` 位存在时（服务端不该有，但宽容处理）按掩码还原。
fn parse_frame<R: Read>(r: &mut R) -> Result<(bool, u8, Vec<u8>), String> {
    let mut head = [0u8; 2];
    read_exact(r, &mut head).map_err(|e| format!("读帧头失败：{e}"))?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len7 = head[1] & 0x7F;

    let len = match len7 {
        126 => {
            let mut b = [0u8; 2];
            read_exact(r, &mut b).map_err(|e| format!("读 16 位长度失败：{e}"))?;
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            read_exact(r, &mut b).map_err(|e| format!("读 64 位长度失败：{e}"))?;
            let v = u64::from_be_bytes(b);
            if v > usize::MAX as u64 {
                return Err("帧长度溢出".into());
            }
            v as usize
        }
        n => n as usize,
    };
    if len > MAX_MESSAGE {
        return Err(format!("WebSocket 消息过大（{len} 字节）"));
    }
    let mut key = [0u8; 4];
    if masked {
        read_exact(r, &mut key).map_err(|e| format!("读掩码失败：{e}"))?;
    }
    let mut payload = vec![0u8; len];
    read_exact(r, &mut payload).map_err(|e| format!("读帧负载失败：{e}"))?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= key[i % 4];
        }
    }
    Ok((fin, opcode, payload))
}

fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> std::io::Result<()> {
    let mut off = 0;
    while off < buf.len() {
        match r.read(&mut buf[off..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "连接已关闭",
                ))
            }
            Ok(n) => off += n,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// 从任意读取器读一条完整消息（文本 + 续帧拼接），ping 交给回调回应。
fn read_message_from<R: Read>(
    r: &mut R,
    mut on_ping: impl FnMut(&[u8]),
) -> Result<String, String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut in_fragment = false;
    loop {
        let (fin, opcode, payload) = parse_frame(r)?;
        match opcode {
            OP_PING => on_ping(&payload),
            OP_PONG => {}
            OP_TEXT => {
                if in_fragment {
                    return Err("协议错误：分片未结束又收到新文本帧".into());
                }
                if fin {
                    return String::from_utf8(payload)
                        .map_err(|_| "WebSocket 文本不是合法 UTF-8".to_string());
                }
                in_fragment = true;
                buf = payload;
            }
            OP_CONT => {
                if !in_fragment {
                    return Err("协议错误：收到孤立的续帧".into());
                }
                if buf.len() + payload.len() > MAX_MESSAGE {
                    return Err("WebSocket 分片消息过大".into());
                }
                buf.extend_from_slice(&payload);
                if fin {
                    return String::from_utf8(buf)
                        .map_err(|_| "WebSocket 文本不是合法 UTF-8".to_string());
                }
            }
            OP_CLOSE => return Err("WebSocket 连接被对端关闭".into()),
            other => return Err(format!("不支持的 WebSocket 操作码：0x{other:X}")),
        }
    }
}

/// 掩码 / Sec-WebSocket-Key 用。不追求密码学随机，只要每次不同即可。
fn random_key() -> [u8; 4] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = t ^ CTR.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x as u32).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造任意 FIN/掩码的帧，供测试使用。
    fn frame(fin: bool, op: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
        let mut f = encode_frame(op, payload, mask);
        if !fin {
            f[0] &= 0x7F;
        }
        f
    }

    #[test]
    fn client_frame_is_masked_and_roundtrips() {
        let f = frame(true, OP_TEXT, b"hello", Some([1, 2, 3, 4]));
        assert_eq!(f[0], 0x81, "FIN + text");
        assert_eq!(f[1], 0x80 | 5, "掩码位 + 长度");
        let mut cur = std::io::Cursor::new(f);
        let (fin, op, payload) = parse_frame(&mut cur).unwrap();
        assert!(fin);
        assert_eq!(op, OP_TEXT);
        assert_eq!(payload, b"hello");
    }

    #[test]
    fn decodes_16bit_length() {
        let body = vec![b'a'; 300];
        let f = frame(true, OP_TEXT, &body, None);
        let mut cur = std::io::Cursor::new(f);
        let (_, op, payload) = parse_frame(&mut cur).unwrap();
        assert_eq!(op, OP_TEXT);
        assert_eq!(payload.len(), 300);
    }

    #[test]
    fn decodes_64bit_length() {
        let body = vec![b'b'; 70_000];
        let f = frame(true, OP_TEXT, &body, None);
        let mut cur = std::io::Cursor::new(f);
        let (_, _, payload) = parse_frame(&mut cur).unwrap();
        assert_eq!(payload.len(), 70_000);
    }

    #[test]
    fn rejects_unknown_opcode() {
        let f = frame(true, 0x3, b"x", None);
        let mut cur = std::io::Cursor::new(f);
        let err = read_message_from(&mut cur, |_| {}).unwrap_err();
        assert!(err.contains("操作码"), "{err}");
    }

    #[test]
    fn rejects_oversize() {
        // 只发头部：127 + 8 字节长度（17MB），解析应在分配负载前就报错。
        let mut f = vec![0x81, 0x80 | 127];
        f.extend_from_slice(&((MAX_MESSAGE as u64) + 1).to_be_bytes());
        let mut cur = std::io::Cursor::new(f);
        let err = parse_frame(&mut cur).unwrap_err();
        assert!(err.contains("过大"), "{err}");
    }

    #[test]
    fn reassembles_fragments() {
        let mut data = frame(false, OP_TEXT, b"hel", None);
        data.extend(frame(true, OP_CONT, b"lo", None));
        let mut cur = std::io::Cursor::new(data);
        assert_eq!(read_message_from(&mut cur, |_| {}).unwrap(), "hello");
    }

    #[test]
    fn answers_ping_between_fragments() {
        let mut data = frame(false, OP_TEXT, b"he", None);
        data.extend(frame(true, OP_PING, b"p", None));
        data.extend(frame(true, OP_CONT, b"llo", None));
        let mut cur = std::io::Cursor::new(data);
        let mut pings = Vec::new();
        let msg = read_message_from(&mut cur, |p| pings.push(p.to_vec())).unwrap();
        assert_eq!(msg, "hello");
        assert_eq!(pings, vec![b"p".to_vec()]);
    }

    #[test]
    fn close_frame_is_error() {
        let f = frame(true, OP_CLOSE, &[], None);
        let mut cur = std::io::Cursor::new(f);
        let err = read_message_from(&mut cur, |_| {}).unwrap_err();
        assert!(err.contains("关闭"), "{err}");
    }

    #[test]
    fn handshake_headers_are_not_treated_as_frames() {
        // 回归：`pending` 曾把整个 101 响应头当成帧数据，
        // 首帧解析读到 `HT` 就当成 close 帧，读线程直接退出。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let resp = "HTTP/1.1 101 Switching Protocols\r\n\
                        Upgrade: websocket\r\nConnection: Upgrade\r\n\
                        Sec-WebSocket-Accept: x\r\n\r\n";
            s.write_all(resp.as_bytes()).unwrap();
            s.write_all(&encode_frame(OP_TEXT, br#"{"id":1,"result":{}}"#, None))
                .unwrap();
            s.flush().unwrap();
            // 保持连接，等客户端读完再关闭
            std::thread::sleep(Duration::from_millis(300));
        });
        let (mut reader, _writer) =
            connect("127.0.0.1", port, "/devtools/page/x", Duration::from_secs(5)).unwrap();
        assert_eq!(reader.read_message().unwrap(), r#"{"id":1,"result":{}}"#);
    }
}
