mod request;
mod response;

use clap::Parser;
use rand::{Rng, SeedableRng};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio::time::sleep;
use std::sync::Arc;
use std::time::Duration;

/// Contains information parsed from the command-line invocation of balancebeam. The Clap macros
/// provide a fancy way to automatically construct a command-line argument parser.
#[derive(Parser, Debug)]
#[command(about = "Fun with load balancing")]
struct CmdOptions {
    /// "IP/port to bind to"
    #[arg(short, long, default_value = "0.0.0.0:1100")]
    bind: String,
    /// "Upstream host to forward requests to"
    #[arg(short, long)]
    upstream: Vec<String>,
    /// "Perform active health checks on this interval (in seconds)"
    #[arg(long, default_value = "10")]
    active_health_check_interval: usize,
    /// "Path to send request to for active health checks"
    #[arg(long, default_value = "/")]
    active_health_check_path: String,
    /// "Maximum number of requests to accept per IP per minute (0 = unlimited)"
    #[arg(long, default_value = "0")]
    max_requests_per_minute: usize,
}

/// Contains information about the state of balancebeam (e.g. what servers we are currently proxying
/// to, what servers have failed, rate limiting counts, etc.)
///
/// You should add fields to this struct in later milestones.
struct ProxyState {
    /// How frequently we check whether upstream servers are alive (Milestone 4)
    active_health_check_interval: usize,
    /// Where we should send requests when doing active health checks (Milestone 4)
    active_health_check_path: String,
    /// Maximum number of requests an individual IP can make in a minute (Milestone 5)
    max_requests_per_minute: usize,
    /// Addresses of servers that we are proxying to
    upstream_addresses: Vec<String>,
    live_upstream_addresses: RwLock<Vec<Option<String>>>,
}

#[tokio::main]
async fn main() {
    // Initialize the logging library. You can print log messages using the `log` macros:
    // https://docs.rs/log/0.4.8/log/ You are welcome to continue using print! statements; this
    // just looks a little prettier.
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "debug");
    }
    pretty_env_logger::init();

    // Parse the command line arguments passed to this program
    let options = CmdOptions::parse();
    if options.upstream.len() < 1 {
        log::error!("At least one upstream server must be specified using the --upstream option.");
        std::process::exit(1);
    }

    // Start listening for connections
    let listener = match TcpListener::bind(&options.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            log::error!("Could not bind to {}: {}", options.bind, err);
            std::process::exit(1);
        }
    };
    log::info!("Listening for requests on {}", options.bind);

    // Handle incoming connections
    let state = ProxyState {
        upstream_addresses: options.upstream.clone(),
        live_upstream_addresses: RwLock::new(options.upstream.into_iter().map(|x| Some(x)).collect()),
        active_health_check_interval: options.active_health_check_interval,
        active_health_check_path: options.active_health_check_path,
        max_requests_per_minute: options.max_requests_per_minute,
    };
    let common_state = Arc::new(state);

    // Handle the connection!
    loop {
        let stream = listener.accept().await;

        if let Ok((stream, _addr)) = stream {
            let state = common_state.clone();
            tokio::spawn(async move {
                active_health_checks(&state).await;
            });

            let state2 = common_state.clone();
            tokio::spawn(async move {
                handle_connection(stream, &state2).await;
            });
        }
    }
}

async fn rate_limiting(conn: &mut TcpStream, state: &ProxyState) {
    let response = response::make_http_error(http::StatusCode::TOO_MANY_REQUESTS);
    response::write_to_stream(&response, conn).await.unwrap();
    todo!()
}

async fn connect_to_upstream(state: &ProxyState) -> Result<TcpStream, std::io::Error> {
    let upstream_addresses_lock = state.live_upstream_addresses.read().await;

    let mut rng = rand::rngs::StdRng::from_entropy();
    let mut upstream_idx = rng.gen_range(0..upstream_addresses_lock.len());
    while upstream_addresses_lock[upstream_idx].clone().is_none() {
        upstream_idx = rng.gen_range(0..upstream_addresses_lock.len());
    }
    let upstream_ip = upstream_addresses_lock[upstream_idx].clone().unwrap();

    match TcpStream::connect(upstream_ip.clone()).await {
        Ok(t) => Ok(t),
        Err(err) => {
            drop(upstream_addresses_lock);
            log::error!("Failed to connect to upstream {}: {} and trying again", upstream_ip, err);
            filter_upstream(err, upstream_idx, state).await
        }
    }
}

async fn filter_upstream(err: std::io::Error, upstream_idx: usize, state: &ProxyState) -> Result<TcpStream, std::io::Error> {
    let mut upstream_addresses_lock = state.live_upstream_addresses.write().await;
    upstream_addresses_lock[upstream_idx] = None;
    let mut err = err;

    for i in 0..upstream_addresses_lock.len() {
        if i == upstream_idx {
            continue;
        }

        match upstream_addresses_lock[i].clone() {
            Some(upstream_ip) => {
                match TcpStream::connect(upstream_ip).await {
                    Ok(stream) => {
                        return Ok(stream)
                    },
                    Err(e) => {
                        upstream_addresses_lock[i] = None;
                        err = e;
                    }
                }
            },
            None => continue
        }
    }

    log::error!("Failed to connect to all upstream {}", err);
    Err(err)
}

async fn active_health_checks(state: &ProxyState) {
    // active_health_check_interval: usize,
    for i in 0..state.upstream_addresses.len() {
        sleep(Duration::from_secs(
            state.active_health_check_interval.try_into().unwrap(),
        )).await;

        let mut live_upstream_addresses = state.live_upstream_addresses.write().await;

        let upstream_ip = state.upstream_addresses[i].clone();

        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri(&state.active_health_check_path)
            .header("Host", upstream_ip.clone())
            .body(Vec::new())
            .unwrap();

        match TcpStream::connect(upstream_ip.clone()).await {
            Ok(mut conn) => {
                if let Err(_e) = send_request(&mut conn, &request).await {
                    return;
                }

                let response = match response::read_from_stream(&mut conn, &request.method()).await {
                    Ok(response) => response,
                    Err(error) => {
                        log::error!("Error reading response from server: {:?}", error);
                        return;
                    }
                };
                match response.status().as_u16() {
                    200 => {
                        live_upstream_addresses[i] = Some(upstream_ip.clone());
                    }
                    _ => {
                        live_upstream_addresses[i] = None;
                        log::error!(
                            "upstream server {} is not working",
                            upstream_ip,
                        );
                        return;
                    }
                }
            },
            Err(e) => {
                log::error!("Failed to connect to upstream {}: {}", upstream_ip, e);
                return;
            },
        }
    }
}

async fn send_request(conn: &mut TcpStream, request: &http::Request<Vec<u8>>) -> Result<(), std::io::Error> {
    let upstream_ip = conn.peer_addr().unwrap().ip().to_string();
    match request::write_to_stream(&request, conn).await {
        Ok(_x) => Ok(()),
        Err(error) => {
            log::error!(
                "Failed to send request to upstream {}: {}",
                upstream_ip,
                error
            );
            Err(error)
        }
    }
}

async fn send_response(client_conn: &mut TcpStream, response: &http::Response<Vec<u8>>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!(
        "{} <- {}",
        client_ip,
        response::format_response_line(&response)
    );
    if let Err(error) = response::write_to_stream(&response, client_conn).await {
        log::warn!("Failed to send response to client: {}", error);
        return;
    }
}

async fn handle_connection(mut client_conn: TcpStream, state: &ProxyState) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("Connection received from {}", client_ip);

    // Open a connection to a random destination server
    let mut upstream_conn = match connect_to_upstream(state).await {
        Ok(stream) => stream,
        Err(_error) => {
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
    };
    let upstream_ip = upstream_conn.peer_addr().unwrap().ip().to_string();

    // The client may now send us one or more requests. Keep trying to read requests until the
    // client hangs up or we get an error.
    loop {
        // Read a request from the client
        let mut request = match request::read_from_stream(&mut client_conn).await {
            Ok(request) => request,
            // Handle case where client closed connection and is no longer sending requests
            Err(request::Error::IncompleteRequest(0)) => {
                log::debug!("Client finished sending requests. Shutting down connection");
                return;
            }
            // Handle I/O error in reading from the client
            Err(request::Error::ConnectionError(io_err)) => {
                log::info!("Error reading request from client stream: {}", io_err);
                return;
            }
            Err(error) => {
                log::debug!("Error parsing request: {:?}", error);
                let response = response::make_http_error(match error {
                    request::Error::IncompleteRequest(_)
                    | request::Error::MalformedRequest(_)
                    | request::Error::InvalidContentLength
                    | request::Error::ContentLengthMismatch => http::StatusCode::BAD_REQUEST,
                    request::Error::RequestBodyTooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
                    request::Error::ConnectionError(_) => http::StatusCode::SERVICE_UNAVAILABLE,
                });
                send_response(&mut client_conn, &response).await;
                continue;
            }
        };
        log::info!(
            "{} -> {}: {}",
            client_ip,
            upstream_ip,
            request::format_request_line(&request)
        );

        // Add X-Forwarded-For header so that the upstream server knows the client's IP address.
        // (We're the ones connecting directly to the upstream server, so without this header, the
        // upstream server will only know our IP, not the client's.)
        request::extend_header_value(&mut request, "x-forwarded-for", &client_ip);

        // Forward the request to the server
        if let Err(error) = request::write_to_stream(&request, &mut upstream_conn).await {
            log::error!(
                "Failed to send request to upstream {}: {}",
                upstream_ip,
                error
            );
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
        log::debug!("Forwarded request to server");

        // Read the server's response
        let response = match response::read_from_stream(&mut upstream_conn, request.method()).await {
            Ok(response) => response,
            Err(error) => {
                log::error!("Error reading response from server: {:?}", error);
                let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                send_response(&mut client_conn, &response).await;
                return;
            }
        };
        // Forward the response to the client
        send_response(&mut client_conn, &response).await;
        log::debug!("Forwarded response to client");
    }
}
