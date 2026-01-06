pub mod server;
pub mod llm;
pub mod json;

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::net::ToSocketAddrs;
use std::{fmt::Write, io::Read};
use std::process::Command;
use tiny_http::{Method, Response, Server};

use crate::llm::start_llm;
use crate::server::start_server;



// ============ Data Structures ============

#[derive(Serialize)]
struct AppInfo {
    name: String,
    bundle_id: String,
    window_title: Option<String>,
    finder_path: Option<String>,
}

#[derive(Deserialize)]
struct ToolRequest {
    tool: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Deserialize)]
struct PromptRequest {
    prompt: String,
    #[serde(default)]
    tools: Vec<ToolDef>,
}

#[derive(Deserialize, Serialize, Clone)]
struct ToolDef {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize)]
struct ToolResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: Option<String>,
    done: Option<bool>,
}

// ============ Main Server ============

#[tokio::main]
async fn main() {
    if let Ok(file) = envfile::EnvFile::new(".env") {
        for (key, value) in &file.store {
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }

    }
    let mut args = std::env::args();

    let exec = args.next().unwrap();
    println!("{exec}");

    if Some("serve") == args.next().as_deref() {
        start_llm().await.unwrap();
        return;
    }

    start_server();
}


