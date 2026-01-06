use std::io::{self, BufRead, Write};

use futures::StreamExt;
use openrouter_rs::{
    api::chat::ChatCompletionRequest,
    types::Role,
    Message, OpenRouterClient,
};

use crate::server::URL;

/// A simple chat message for history
#[derive(Clone)]
struct ChatMessage {
    role: Role,
    content: String,
}

/// Start an interactive chat session with the LLM
pub async fn start_llm() -> Result<(), Box<dyn std::error::Error>> {
    let client = OpenRouterClient::builder()
        .api_key(std::env::var("API_KEY")?)
        .build()?;

    let model = std::env::var("AI_MODEL").unwrap_or_else(|_| "openai/gpt-4o".to_string());

    // Fetch system prompt from opencanon server
    let system_prompt = fetch_system_prompt()?;
    println!("📝 System prompt loaded ({} chars)", system_prompt.len());

    // Conversation history
    let mut history: Vec<ChatMessage> = Vec::new();

    println!("\n🤖 OpenCanon Chat (model: {})", model);
    println!("   Type your message and press Enter. Type 'quit' to exit.\n");

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        // Prompt
        print!("You: ");
        stdout.flush()?;

        // Read user input
        let mut input = String::new();
        stdin.lock().read_line(&mut input)?;
        let input = input.trim();

        if input.is_empty() {
            continue;
        }

        if input == "quit" || input == "exit" {
            println!("👋 Goodbye!");
            break;
        }

        if input == "/clear" {
            history.clear();
            println!("🗑️  History cleared.\n");
            continue;
        }

        if input == "/history" {
            println!("📜 History ({} messages):", history.len());
            for (i, msg) in history.iter().enumerate() {
                let role = match msg.role {
                    Role::User => "You",
                    Role::Assistant => "AI",
                    _ => "System",
                };
                println!("  {}: [{}] {}...", i + 1, role, &msg.content.chars().take(50).collect::<String>());
            }
            println!();
            continue;
        }

        // Add user message to history
        history.push(ChatMessage {
            role: Role::User,
            content: input.to_string(),
        });

        // Agentic loop - keep going until no tool results
        loop {
            // Build messages for the API
            let mut messages: Vec<Message> = vec![Message::new(Role::System, &system_prompt)];
            for msg in &history {
                messages.push(Message::new(msg.role.clone(), &msg.content));
            }

            // Make the request
            let request = ChatCompletionRequest::builder()
                .model(&model)
                .messages(messages)
                .reasoning_effort(openrouter_rs::types::Effort::Medium)
                .reasoning_max_tokens(256)
                .build()?;

            print!("AI: ");
            stdout.flush()?;

            let mut full_response = String::new();
            let mut reasoning_response = String::new();
            let mut stream = client.stream_chat_completion(&request).await?;

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => {
                        for c in event.choices {
                            if let Some(reasoning) = c.reasoning() {
                                print!("\x1b[90m{}\x1b[0m", reasoning); // dim gray for reasoning
                                stdout.flush()?;
                                reasoning_response.push_str(reasoning);
                            }
                            if let Some(content) = c.content() {
                                print!("{}", content);
                                stdout.flush()?;
                                full_response.push_str(content);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("\n❌ Error: {:?}", e);
                    }
                }
            }
            println!("\n");

            // Use content if available, otherwise fall back to reasoning
            let response_to_process = if full_response.is_empty() {
                reasoning_response.clone()
            } else {
                full_response.clone()
            };

            if response_to_process.is_empty() {
                break;
            }

            // Add assistant response to history
            history.push(ChatMessage {
                role: Role::Assistant,
                content: response_to_process.clone(),
            });

            // Post to opencanon /process for memory operations  
            match process_response(&response_to_process) {
                Ok(result) if !result.trim().is_empty() => {
                    println!("\x1b[33m📝 Tool Result:\x1b[0m {}", result.trim());
                    
                    // Feed the tool result back to the LLM
                    history.push(ChatMessage {
                        role: Role::User,
                        content: result,
                    });
                    
                    // Continue the loop to let LLM respond to tool results
                    println!();
                    continue;
                }
                Err(e) => {
                    eprintln!("⚠️  Process error: {}", e);
                }
                _ => {}
            }

            // No tool results, break out of agentic loop
            break;
        }
    }

    Ok(())
}

/// Fetch the system prompt from the opencanon server
fn fetch_system_prompt() -> Result<String, String> {
    ureq::get(&format!("{URL}/system"))
        .call()
        .map_err(|e| format!("Failed to fetch system prompt: {}", e))?
        .into_string()
        .map_err(|e| format!("Failed to read system prompt: {}", e))
}

/// Post the LLM response to opencanon for processing memory ops
fn process_response(content: &str) -> Result<String, String> {
    ureq::post(&format!("{URL}/process"))
        .send_string(content)
        .map_err(|e| format!("Failed to process: {}", e))?
        .into_string()
        .map_err(|e| format!("Failed to read result: {}", e))
}
