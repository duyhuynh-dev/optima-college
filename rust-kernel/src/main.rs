use std::collections::HashMap;
use std::path::PathBuf;

use rust_kernel::conflicts::{detect_conflicts, load_meetings_from_csv};
use rust_kernel::grpc::optima::v1::kernel_server::KernelServer;
use rust_kernel::grpc::KernelService;
use rust_kernel::telemetry;
use serde::Serialize;
use tiny_http::{Header, Response, Server};
use tonic::transport::Server as GrpcServer;

#[derive(Serialize)]
struct KernelHealth {
    status: &'static str,
    service: &'static str,
}

#[derive(Serialize)]
struct ConflictResponse {
    status: &'static str,
    csv_path: String,
    selected_sections: Vec<String>,
    has_conflict: bool,
    conflict_count: usize,
    conflicts: Vec<rust_kernel::conflicts::ConflictPair>,
}

fn parse_query(url: &str) -> HashMap<String, String> {
    let mut query_map = HashMap::new();
    let query = match url.split_once('?') {
        Some((_, query)) => query,
        None => return query_map,
    };

    for entry in query.split('&') {
        if entry.is_empty() {
            continue;
        }
        let (key, value) = match entry.split_once('=') {
            Some((k, v)) => (k, v),
            None => (entry, ""),
        };
        query_map.insert(key.to_string(), value.to_string());
    }

    query_map
}

fn json_response<T: Serialize>(payload: &T, status_code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{\"status\":\"error\"}".to_vec());
    let content_type = Header::from_bytes("Content-Type", "application/json")
        .expect("failed to create content type header");
    Response::from_data(body)
        .with_status_code(status_code)
        .with_header(content_type)
}

fn run_http_server() {
    let server = Server::http("0.0.0.0:8090").expect("failed to bind kernel http server");
    println!("rust-kernel http listening on :8090");

    for request in server.incoming_requests() {
        let url = request.url().to_string();

        if url == "/health" {
            let body = serde_json::to_string(&KernelHealth {
                status: "ok",
                service: "rust-kernel",
            })
            .expect("json serialization failed");

            let content_type = Header::from_bytes("Content-Type", "application/json")
                .expect("failed to create content type header");
            let response = Response::from_string(body).with_header(content_type);
            let _ = request.respond(response);
            continue;
        }

        if url.starts_with("/v1/conflicts") {
            let query = parse_query(&url);
            let csv_path = query
                .get("csv_path")
                .cloned()
                .unwrap_or_else(|| "../python-ml/output/meetings_1269.csv".to_string());
            let selected_sections = query
                .get("sections")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(ToString::to_string)
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();

            if selected_sections.is_empty() {
                let response = json_response(
                    &serde_json::json!({
                        "status": "error",
                        "message": "missing required query param: sections=COMP112-01,COMP211-01"
                    }),
                    400,
                );
                let _ = request.respond(response);
                continue;
            }

            let path = PathBuf::from(csv_path.clone());
            let meetings = match load_meetings_from_csv(&path) {
                Ok(rows) => rows,
                Err(err) => {
                    let response = json_response(
                        &serde_json::json!({
                            "status": "error",
                            "message": err
                        }),
                        500,
                    );
                    let _ = request.respond(response);
                    continue;
                }
            };

            let conflicts = detect_conflicts(&meetings, &selected_sections);
            let payload = ConflictResponse {
                status: "ok",
                csv_path,
                selected_sections,
                has_conflict: !conflicts.is_empty(),
                conflict_count: conflicts.len(),
                conflicts,
            };
            let response = json_response(&payload, 200);
            let _ = request.respond(response);
            continue;
        }

        let _ = request.respond(Response::from_string("not found").with_status_code(404));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Err(e) = telemetry::init() {
        eprintln!("telemetry init: {e} (continuing with default logging)");
    }

    let default_csv = PathBuf::from("../python-ml/output/meetings_1269.csv");
    let default_sections_csv = PathBuf::from("../python-ml/output/sections_1269.csv");
    let svc = KernelService {
        default_csv: default_csv.clone(),
        default_sections_csv,
    };

    std::thread::spawn(run_http_server);

    let addr = "0.0.0.0:50051".parse()?;
    println!("rust-kernel gRPC listening on :50051 (Kernel service)");
    GrpcServer::builder()
        .add_service(KernelServer::new(svc))
        .serve(addr)
        .await?;
    Ok(())
}
