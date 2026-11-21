use std::error;
use std::fmt;
use std::io;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::string::FromUtf8Error;
use std::thread;

use prometheus::Encoder;

const LOG_TARGET: &str = "prometheus_metric_server";
// The content type of the Prometheus text exposition format.
const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

// This is a minimal, single-threaded, and incorrect HTTP server
// which answers on all requests with prometheus metrics.

pub fn start_http_server(prometheus_address: &str) -> Result<SocketAddr, io::Error> {
    let listener = TcpListener::bind(prometheus_address)?;
    let local_address = listener.local_addr()?;
    log::info!(
        target: LOG_TARGET,
        "Started Prometheus Metric Server. Listening on {}.",
        local_address
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
    Ok(local_address)
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
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
        CONTENT_TYPE,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends a request to the server and returns the raw response.
    fn request(address: &SocketAddr) -> String {
        let mut stream = TcpStream::connect(address).expect("we should be able to connect");
        stream
            .write_all(b"GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("we should be able to send a request");
        stream.flush().expect("we should be able to flush");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("we should be able to read the response");
        String::from_utf8(response).expect("the response should be valid UTF-8")
    }

    /// Splits a response into its headers and its body.
    fn split_response(response: &str) -> (&str, &str) {
        response
            .split_once("\r\n\r\n")
            .expect("the response should have a header and a body separated by an empty line")
    }

    fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
        headers
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|value| value.trim())
    }

    #[test]
    fn server_answers_with_the_metrics() {
        let gauge = prometheus::register_int_gauge!(
            "miningpool_observer_test_gauge_answers",
            "a gauge used in the metric server test"
        )
        .expect("we should be able to register a test gauge");
        gauge.set(42);

        let address = start_http_server("127.0.0.1:0").expect("the server should start");
        assert_eq!(address.ip().to_string(), "127.0.0.1");
        assert_ne!(address.port(), 0, "the bound port should be known");

        let response = request(&address);
        assert!(
            response.starts_with("HTTP/1.1 200 OK\r\n"),
            "unexpected response: {}",
            response
        );

        let (headers, body) = split_response(&response);
        assert_eq!(
            header_value(headers, "Content-Type:"),
            Some("text/plain; version=0.0.4; charset=utf-8")
        );
        let content_length: usize = header_value(headers, "Content-Length:")
            .expect("the response should have a Content-Length header")
            .parse()
            .expect("the Content-Length should be a number");
        assert_eq!(
            content_length,
            body.len(),
            "the Content-Length should be the byte length of the body"
        );

        assert!(!body.is_empty(), "the body should contain the metrics");
        assert!(
            body.contains("miningpool_observer_test_gauge_answers 42"),
            "the body should contain the registered metric: {}",
            body
        );
        // the Prometheus text exposition format has a HELP and a TYPE comment
        // for every metric
        assert!(body.contains("# HELP miningpool_observer_test_gauge_answers"));
        assert!(body.contains("# TYPE miningpool_observer_test_gauge_answers gauge"));
        for line in body.lines() {
            assert!(
                !line.starts_with(' '),
                "a metric line should not start with a space: {:?}",
                line
            );
        }
    }

    #[test]
    fn server_keeps_running_after_a_client_disconnects() {
        let gauge = prometheus::register_int_gauge!(
            "miningpool_observer_test_gauge_disconnect",
            "a gauge used in the metric server test"
        )
        .expect("we should be able to register a test gauge");
        gauge.set(7);

        let address = start_http_server("127.0.0.1:0").expect("the server should start");

        // a client that connects and immediately goes away again
        let stream = TcpStream::connect(address).expect("we should be able to connect");
        drop(stream);

        let response = request(&address);
        assert!(
            response.starts_with("HTTP/1.1 200 OK\r\n"),
            "unexpected response: {}",
            response
        );
        let (_, body) = split_response(&response);
        assert!(body.contains("miningpool_observer_test_gauge_disconnect 7"));
    }

    #[test]
    fn server_answers_more_than_one_request() {
        let address = start_http_server("127.0.0.1:0").expect("the server should start");
        for _ in 0..3 {
            let response = request(&address);
            assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        }
    }

    #[test]
    fn starting_the_server_on_an_invalid_address_fails() {
        assert!(start_http_server("999.999.999.999:1").is_err());
        assert!(start_http_server("not an address").is_err());
    }

    #[test]
    fn request_handling_errors_have_a_message_and_a_source() {
        use std::error::Error;

        let errors = vec![
            RequestHandlingError::Io(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe")),
            RequestHandlingError::Utf8(String::from_utf8(vec![0xff]).unwrap_err()),
            RequestHandlingError::Encoding(prometheus::Error::Msg("encoding failed".to_string())),
        ];
        for error in errors.iter() {
            assert!(!error.to_string().is_empty(), "{:?}", error);
            assert!(error.source().is_some(), "{:?}", error);
        }
        assert!(errors[0].to_string().contains("broken pipe"));
        assert!(errors[2].to_string().contains("encoding failed"));
    }
}
