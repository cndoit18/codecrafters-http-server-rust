use flate2::write::GzEncoder;
use flate2::Compression;
use matchit::{Params, Router};
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::io::Write;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:4221").await.unwrap();
    let mut engine = Engine::new();

    engine.register(
        HTTPMethod::Get,
        "/",
        |_: &HTTPRequest| -> Result<HTTPResponse, String> {
            Ok(HTTPResponse {
                version: HTTPVersion::HTTP1_1,
                state: "200 OK".to_string(),
                headers: HashMap::new(),
                body: None,
            })
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/echo/{echo}",
        |req: &HTTPRequest| -> Result<HTTPResponse, String> {
            let mut resp = HTTPResponse {
                version: HTTPVersion::HTTP1_1,
                state: "200 OK".to_string(),
                headers: HashMap::new(),
                body: None,
            };
            let content = req.params.get("echo").unwrap_or("");
            resp.headers
                .insert("Content-Type".to_string(), "text/plain".to_string());
            resp.headers
                .insert("Content-Length".to_string(), content.len().to_string());
            resp.body = Some(content.to_string().into_bytes());
            if req
                .headers
                .get("Accept-Encoding")
                .is_some_and(|v| v.split(',').any(|v| v.trim() == "gzip"))
            {
                let mut e = GzEncoder::new(Vec::new(), Compression::default());
                e.write_all(content.as_bytes()).unwrap();
                let c = e.finish().unwrap();
                resp.headers
                    .insert("Content-Encoding".to_string(), "gzip".to_string());
                resp.headers
                    .insert("Content-Type".to_string(), "text/plain".to_string());
                resp.headers
                    .insert("Content-Length".to_string(), c.len().to_string());
                resp.body = Some(c);
            }
            Ok(resp)
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/user-agent",
        |req: &HTTPRequest| -> Result<HTTPResponse, String> {
            let agent = req
                .headers
                .get("User-Agent")
                .cloned()
                .unwrap_or("".to_string());
            let mut resp = HTTPResponse {
                version: HTTPVersion::HTTP1_1,
                state: "200 OK".to_string(),
                headers: HashMap::new(),
                body: None,
            };

            resp.headers
                .insert("Content-Type".to_string(), "text/plain".to_string());
            resp.headers
                .insert("Content-Length".to_string(), agent.len().to_string());
            resp.body = Some(agent.into_bytes());
            Ok(resp)
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/files/{file}",
        move |req: &HTTPRequest| -> Result<HTTPResponse, String> {
            let name = req.params.get("file").unwrap_or("");
            let mut dir = env::args().collect::<Vec<String>>()[2].clone();
            dir.push_str(name);
            match File::open(&dir) {
                Ok(mut f) => {
                    let mut contents = String::new();
                    f.read_to_string(&mut contents).unwrap();
                    let mut resp = HTTPResponse {
                        version: HTTPVersion::HTTP1_1,
                        state: "200 OK".to_string(),
                        headers: HashMap::new(),
                        body: None,
                    };
                    resp.headers.insert(
                        "Content-Type".to_string(),
                        "application/octet-stream".to_string(),
                    );
                    resp.headers
                        .insert("Content-Length".to_string(), contents.len().to_string());
                    resp.body = Some(contents.clone().into_bytes());
                    if req
                        .headers
                        .get("Accept-Encoding")
                        .is_some_and(|v| v.split(',').any(|v| v.trim() == "gzip"))
                    {
                        let mut e = GzEncoder::new(Vec::new(), Compression::default());
                        e.write_all(contents.as_bytes()).unwrap();
                        let c = e.finish().unwrap();
                        resp.headers
                            .insert("Content-Encoding".to_string(), "gzip".to_string());
                        resp.headers
                            .insert("Content-Length".to_string(), c.len().to_string());
                        resp.body = Some(c);
                    }
                    Ok(resp)
                }
                Err(_) => Ok(HTTPResponse {
                    version: HTTPVersion::HTTP1_1,
                    state: "404 Not Found".to_string(),
                    headers: HashMap::new(),
                    body: None,
                }),
            }
        },
    );

    engine.register(
        HTTPMethod::Post,
        "/files/{file}",
        move |req: &HTTPRequest| -> Result<HTTPResponse, String> {
            let name = req.params.get("file").unwrap_or("");
            let mut dir = env::args().collect::<Vec<String>>()[2].clone();
            dir.push_str(name);
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&dir)
            {
                Ok(mut f) => {
                    req.body.clone().map(|s| f.write_all(s.as_bytes()));
                    Ok(HTTPResponse {
                        version: HTTPVersion::HTTP1_1,
                        state: "201 Created".to_string(),
                        headers: HashMap::new(),
                        body: None,
                    })
                }
                Err(_) => Ok(HTTPResponse {
                    version: HTTPVersion::HTTP1_1,
                    state: "404 Not Found".to_string(),
                    headers: HashMap::new(),
                    body: None,
                }),
            }
        },
    );
    engine.serve(listener).await.unwrap();
}

trait Handler {
    fn handle(&self, req: &HTTPRequest) -> Result<HTTPResponse, String>;
}

impl<F> Handler for F
where
    F: Fn(&HTTPRequest) -> Result<HTTPResponse, String>,
{
    fn handle(&self, req: &HTTPRequest) -> Result<HTTPResponse, String> {
        self(req)
    }
}
struct Engine {
    handlers: HashMap<HTTPMethod, Router<Box<dyn Handler + Sync + Send>>>,
}

unsafe impl Sync for Engine {}
unsafe impl Send for Engine {}

impl Engine {
    fn register(
        &mut self,
        method: HTTPMethod,
        path: &str,
        handler: impl Handler + 'static + Sync + Send,
    ) {
        let _ = self
            .handlers
            .entry(method)
            .or_default()
            .insert(path.to_string(), Box::new(handler));
    }

    fn new() -> Engine {
        Engine {
            handlers: HashMap::new(),
        }
    }

    async fn serve(self, listener: TcpListener) -> Result<(), String> {
        let original = Arc::new(self);
        loop {
            let (stream, _) = listener.accept().await.map_err(|err| err.to_string())?;
            println!("accepted new connection");
            let handle = original.clone();
            tokio::spawn(async move {
                handle.handle_request(stream).await.unwrap();
            });
        }
    }

    async fn handle_request(&self, mut stream: TcpStream) -> Result<(), String> {
        loop {
            let mut req = HTTPRequest::parse(&mut stream).await?;
            match self
                .handlers
                .get(&req.method)
                .and_then(|x| x.at(&req.path).ok())
            {
                Some(s) => {
                    let close = req.headers.get("Connection").is_some_and(|x| x == "close");
                    req.params = s.params;
                    let mut resp = s.value.handle(&req)?;
                    if close {
                        resp.headers
                            .insert("Connection".to_string(), "close".to_string());
                    }
                    stream
                        .write_all(resp.to_vec().as_slice())
                        .await
                        .map_err(|err| format!("invalid wriate response {}", err))?;
                    if close {
                        return Ok(());
                    }
                }
                None => {
                    stream
                        .write_all("HTTP/1.1 404 Not Found\r\n\r\n".to_string().as_bytes())
                        .await
                        .map_err(|err| format!("invalid wriate response {}", err))?;
                }
            }
        }
    }
}

#[derive(Debug)]
struct HTTPResponse {
    version: HTTPVersion,
    state: String,
    headers: HashMap<String, String>,
    body: Option<Vec<u8>>,
}

impl HTTPResponse {
    fn to_vec(&self) -> Vec<u8> {
        let mut buf = vec![format!("{} {}", self.version.to_string(), self.state)];
        buf.extend(
            self.headers
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>(),
        );
        buf.push("\r\n".to_string());

        let mut buf = buf.join("\r\n").into_bytes();
        if let Some(body) = &self.body {
            buf.extend(body);
        }
        buf
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum HTTPMethod {
    Get,
    Post,
}

#[derive(Debug)]
enum HTTPVersion {
    HTTP1_0,
    HTTP1_1,
    HTTP2,
    HTTP3,
}

impl ToString for HTTPVersion {
    fn to_string(&self) -> String {
        match self {
            HTTPVersion::HTTP1_0 => "HTTP/1.0".to_string(),
            HTTPVersion::HTTP1_1 => "HTTP/1.1".to_string(),
            HTTPVersion::HTTP2 => "HTTP/2".to_string(),
            HTTPVersion::HTTP3 => "HTTP/3".to_string(),
        }
    }
}

#[derive(Debug)]
struct HTTPRequest<'k, 'v> {
    params: Params<'k, 'v>,
    version: HTTPVersion,
    method: HTTPMethod,
    path: String,
    headers: HashMap<String, String>,
    body: Option<String>,
}

impl HTTPRequest<'_, '_> {
    pub async fn parse(stream: &mut TcpStream) -> Result<HTTPRequest, String> {
        let mut req = HTTPRequest::default();
        let mut buf = [0; 2048];
        let mut reader = Vec::new();
        while reader.is_empty() {
            stream
                .read(&mut buf)
                .await
                .map(|n| reader.extend_from_slice(&buf[..n]))
                .map_err(|e| e.to_string())?;
        }
        let content = String::from_utf8(reader).map_err(|e| e.to_string())?;
        dbg!(&content);
        let mut content = content.split("\r\n");
        content
            .next()
            .map(|n| {
                if let [method, path, version] = n
                    .split(" ")
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    match method.as_str() {
                        "GET" => req.method = HTTPMethod::Get,
                        "POST" => req.method = HTTPMethod::Post,
                        _ => unimplemented!(),
                    }
                    match version.as_str() {
                        "HTTP/1.0" => req.version = HTTPVersion::HTTP1_0,
                        "HTTP/1.1" => req.version = HTTPVersion::HTTP1_1,
                        "HTTP/2" => req.version = HTTPVersion::HTTP2,
                        "HTTP/3" => req.version = HTTPVersion::HTTP3,
                        _ => unimplemented!(),
                    }
                    req.path = path.to_string();
                };
            })
            .ok_or("invalid parse start line")?;

        while let Some(line) = content
            .next()
            .map(|n| n.split(": ").map(|s| s.to_string()).collect::<Vec<_>>())
            .filter(|v| v.len() == 2)
        {
            if let [k, v] = line.as_slice() {
                req.headers.insert(k.clone(), v.clone());
            }
        }

        let _ = content.next().map(|n| {
            if !n.is_empty() {
                req.body = Some(n.to_string());
            }
        });
        dbg!(&req);
        Ok(req)
    }

    fn default() -> Self {
        Self {
            params: Params::new(),
            version: HTTPVersion::HTTP1_1,
            method: HTTPMethod::Get,
            path: "/".to_string(),
            headers: HashMap::new(),
            body: None,
        }
    }
}
