use std::{io::Read, net::TcpStream};
pub fn response(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(response[..split].to_vec()).unwrap();
    let mut body = &response[split + 4..];
    let mut decoded = Vec::new();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        loop {
            let end = body.windows(2).position(|w| w == b"\r\n").unwrap();
            let size = usize::from_str_radix(
                std::str::from_utf8(&body[..end])
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap(),
                16,
            )
            .unwrap();
            body = &body[end + 2..];
            if size == 0 {
                break;
            }
            decoded.extend_from_slice(&body[..size]);
            body = &body[size + 2..];
        }
    } else {
        decoded.extend_from_slice(body);
    }
    (head, decoded)
}
