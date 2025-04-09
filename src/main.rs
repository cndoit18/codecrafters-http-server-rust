use flate2::write::GzEncoder;
use flate2::Compression;
use matchit::{Params, Router};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::prelude::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::{env, thread};

fn main() {
    let listener = TcpListener::bind("127.0.0.1:4221").unwrap();
    let mut engine = Engine::new();

    engine.register(
        HTTPMethod::Get,
        "/",
        |_: &HTTPRequest| -> Result<Vec<u8>, String> {
            Ok("HTTP/1.1 200 OK\r\n\r\n".to_string().into_bytes())
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/echo/{echo}",
        |req: &HTTPRequest| -> Result<Vec<u8>, String> {
            let content = req.params.get("echo").unwrap_or("");
            if req
                .headers
                .get("Accept-Encoding")
                .is_some_and(|v| v.split(',').any(|v| v.trim() == "gzip"))
            {
                let mut e = GzEncoder::new(Vec::new(), Compression::default());
                e.write_all(content.as_bytes()).unwrap();
                let c  = e.finish().unwrap();
                let mut resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",c.len()
                        ).into_bytes();
                resp.extend(c);
                return Ok(resp);
            }
            Ok(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                content.len(),
                content
            )
            .into_bytes())
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/user-agent",
        |req: &HTTPRequest| -> Result<Vec<u8>, String> {
            let agent = req
                .headers
                .get("User-Agent")
                .cloned()
                .unwrap_or("".to_string());
            Ok(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                agent.len(),
                agent
            )
            .into_bytes())
        },
    );

    engine.register(
        HTTPMethod::Get,
        "/files/{file}",
        move |req: &HTTPRequest| -> Result<Vec<u8>, String> {
            let name = req.params.get("file").unwrap_or("");
            let mut dir = env::args().collect::<Vec<String>>()[2].clone();
            dir.push_str(name);
            match File::open(&dir) {
                Ok(mut f) => {
                    let mut contents = String::new();
                    f.read_to_string(&mut contents).unwrap();
                        if req
                            .headers
                            .get("Accept-Encoding")
                            .is_some_and(|v| v.split(',').any(|v| v.trim() == "gzip"))
                        {
                            let mut e = GzEncoder::new(Vec::new(), Compression::default());
                            e.write_all(contents.as_bytes()).unwrap();
                            let c  = e.finish().unwrap();
                            let mut resp = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",c.len()
                                    ).into_bytes();
                            resp.extend(c);
                            return Ok(resp);
                        }
                    Ok(format!( "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n{}",contents.len(), contents).into_bytes())
                }
                Err(_) => Ok("HTTP/1.1 404 Not Found\r\n\r\n".to_string().into_bytes()),
            }
        },
    );

    engine.register(
        HTTPMethod::Post,
        "/files/{file}",
        move |req: &HTTPRequest| -> Result<Vec<u8>, String> {
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
                    f.write_all(req.body.clone().unwrap().as_bytes()).unwrap();
                    Ok("HTTP/1.1 201 Created\r\n\r\n".to_string().into_bytes())
                }
                Err(_) => Ok("HTTP/1.1 404 Not Found\r\n\r\n".to_string().into_bytes()),
            }
        },
    );
    engine.serve(listener).unwrap();
}

trait Handler {
    fn handle(&self, req: &HTTPRequest) -> Result<Vec<u8>, String>;
}

impl<F> Handler for F
where
    F: Fn(&HTTPRequest) -> Result<Vec<u8>, String>,
{
    fn handle(&self, req: &HTTPRequest) -> Result<Vec<u8>, String> {
        self(req)
    }
}
struct Engine {
    handlers: HashMap<HTTPMethod, Router<Box<dyn Handler>>>,
}

unsafe impl Sync for Engine {}
unsafe impl Send for Engine {}

impl Engine {
    fn register(&mut self, method: HTTPMethod, path: &str, handler: impl Handler + 'static) {
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

    fn serve(self, listener: TcpListener) -> Result<(), String> {
        let original = Arc::new(self);
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler = original.clone();
                    println!("accepted new connection");
                    thread::spawn(move || {
                        handler.handle_request(stream).unwrap();
                    });
                }
                Err(e) => return Err(format!("failed to accept connection: {}", e)),
            }
        }
        Ok(())
    }

    fn handle_request(&self, mut stream: TcpStream) -> Result<(), String> {
        println!("accepted new connection");
        let mut req = HTTPRequest::parse(&stream).unwrap();
        if let Some(s) = self
            .handlers
            .get(&req.method)
            .and_then(|x| x.at(&req.path).ok())
        {
            req.params = s.params;
            let resp = s.value.handle(&req)?;
            stream
                .write_all(resp.as_slice())
                .map_err(|err| format!("invalid wriate response {}", err))?;
            return Ok(());
        }
        stream
            .write_all("HTTP/1.1 404 Not Found\r\n\r\n".to_string().as_bytes())
            .map_err(|err| format!("invalid wriate response {}", err))?;
        Ok(())
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
    pub fn parse(mut stream: &TcpStream) -> Result<HTTPRequest, String> {
        let mut req = HTTPRequest::default();
        let mut buf = [0; 2048];
        let mut reader = Vec::new();
        stream
            .read(&mut buf)
            .map(|n| reader.extend_from_slice(&buf[..n]))
            .map_err(|e| e.to_string())?;
        let content = String::from_utf8(reader).map_err(|e| e.to_string())?;
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

        while let [key, value] = content
            .next()
            .ok_or("invalid parse headers")
            .map(|n| n.split(": ").map(|s| s.to_string()).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?
            .as_slice()
        {
            req.headers.insert(key.to_owned(), value.to_owned());
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
