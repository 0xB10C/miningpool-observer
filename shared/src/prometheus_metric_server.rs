use std::error;
use std::fmt;
use std::io;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::string::FromUtf8Error;
use std::thread;

use prometheus::Encoder;

const LOG_TARGET: &str = "prometheus_metric_server";

// This is a minimal, single-threaded, and incorrect HTTP server
// which answers on all requests with prometheus metrics.

pub fn start_http_server(prometheus_address: &str) -> Result<(), io::Error> {
    let listener = TcpListener::bind(prometheus_address)?;
    log::info!(
        target: LOG_TARGET,
        "Started Prometheus Metric Server. Listening on {}.",
        prometheus_address
    );
    thread::spawn(move || {
        for incoming_request in listener.incoming() {
            let stream = match incoming_request {
                Ok(s) => s,
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET,
                        "Could not process incomming request {}.",
                        e
                    );
                    continue;
                }
            };
            if let Err(e) = handle_request(stream) {
                log::error!(target: LOG_TARGET, "Could not handle request {}.", e);
                continue;
            };
        }
    });
    Ok(())
}

fn handle_request(mut stream: TcpStream) -> Result<(), RequestHandlingError> {
    let mut buffer = [0; 1024];
    let _ = stream.read(&mut buffer)?;

    let mut output_buffer = vec![];
    let encoder = prometheus::TextEncoder::new();
    let metric_families = prometheus::gather();
    if let Err(e) = encoder.encode(&metric_families, &mut output_buffer) {
        return Err(RequestHandlingError::Encoding(e));
    };
    let contents = String::from_utf8(output_buffer.clone())?;
    output_buffer.clear();

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
        contents.len(),
        contents
    );

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[derive(Debug)]
enum RequestHandlingError {
    Io(io::Error),
    Utf8(FromUtf8Error),
    Encoding(prometheus::Error),
}

impl fmt::Display for RequestHandlingError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RequestHandlingError::Utf8(e) => write!(f, "UTF-8 error: {}", e),
            RequestHandlingError::Io(e) => write!(f, "IO error: {}", e),
            RequestHandlingError::Encoding(e) => write!(f, "encoding error: {}", e),
        }
    }
}

impl error::Error for RequestHandlingError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            RequestHandlingError::Utf8(ref e) => Some(e),
            RequestHandlingError::Io(ref e) => Some(e),
            RequestHandlingError::Encoding(ref e) => Some(e),
        }
    }
}

impl From<io::Error> for RequestHandlingError {
    fn from(err: io::Error) -> RequestHandlingError {
        RequestHandlingError::Io(err)
    }
}

impl From<FromUtf8Error> for RequestHandlingError {
    fn from(err: FromUtf8Error) -> RequestHandlingError {
        RequestHandlingError::Utf8(err)
    }
}

impl From<prometheus::Error> for RequestHandlingError {
    fn from(err: prometheus::Error) -> RequestHandlingError {
        RequestHandlingError::Encoding(err)
    }
}
