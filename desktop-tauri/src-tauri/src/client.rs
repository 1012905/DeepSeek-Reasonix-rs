// The shell's own route to the kernel, from the process that holds the
// credential. Routing these through the page would put the token within its
// reach for nothing beyond convenience.
use std::time::Duration;

pub const TOKEN_COOKIE: &str = "reasonix_token";
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct KernelClient {
    origin: String,
    token: String,
    agent: ureq::Agent,
}

pub struct Answer {
    pub status: u16,
    pub body: String,
}

impl KernelClient {
    pub fn new(origin: String, token: String) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(TIMEOUT)
            .redirects(0)
            .build();
        Self {
            origin,
            token,
            agent,
        }
    }

    /// A path and query under the kernel's origin. A refusal is an answer like
    /// any other; only a kernel that could not be reached is an error.
    pub fn get(&self, path_and_query: &str) -> Result<Answer, String> {
        if !path_and_query.starts_with('/') {
            return Err("the kernel path must be absolute".into());
        }
        let url = format!("{}{}", self.origin, path_and_query);
        let response = self
            .agent
            .get(&url)
            .set("Cookie", &format!("{TOKEN_COOKIE}={}", self.token))
            .call();
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(err) => return Err(format!("the kernel did not answer: {err}")),
        };
        let status = response.status();
        let body = response
            .into_string()
            .map_err(|e| format!("reading the kernel's answer: {e}"))?;
        Ok(Answer { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve_once(reply: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let mut request = Vec::new();
            loop {
                let n = stream.read(&mut buf).expect("read");
                request.extend_from_slice(&buf[..n]);
                if n == 0 || request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            stream.write_all(reply.as_bytes()).expect("write");
            String::from_utf8_lossy(&request).into_owned()
        });
        (origin, handle)
    }

    #[test]
    fn get_carries_the_credential_cookie() {
        let (origin, handle) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into(),
        );
        let client = KernelClient::new(origin, "t3".into());
        let answer = client
            .get("/host/workspaces/locate?root=x")
            .expect("answers");
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, "ok");
        let request = handle.join().expect("server");
        assert!(request.starts_with("GET /host/workspaces/locate?root=x HTTP/1.1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: reasonix_token=t3\r\n")
        );
    }

    #[test]
    fn a_refusal_is_an_answer() {
        let body = "{\"code\":\"workspace.unknown\",\"error\":\"no\"}";
        let reply = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (origin, handle) = serve_once(reply);
        let answer = KernelClient::new(origin, "t".into())
            .get("/x")
            .expect("answers");
        assert_eq!(answer.status, 404);
        assert_eq!(answer.body, body);
        handle.join().expect("server");
    }

    #[test]
    fn get_refuses_a_relative_path() {
        let client = KernelClient::new("http://127.0.0.1:1".into(), "t".into());
        assert!(client.get("host/x").is_err());
    }
}
