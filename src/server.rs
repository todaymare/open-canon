use std::{collections::HashMap, fmt::Write};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sti::arena::Arena;
use tiny_http::{Method, Response, Server};
use xml::reader::XmlEvent;

use crate::json::{self, Value};


pub const PORT: u16 = 9876;
pub const URL : &str = "http://localhost:9876";
const OLLAMA_URL: &str = "http://localhost:11434";
const PLANNER_MODEL: &str = "tool-planner";
const COMPILER_MODEL: &str = "tool-compiler";

pub const SYSTEM_PROMPT : &str = r#"
The content inside <OPENCANON></OPENCANON> applies ONLY when using OpenCanon services.

<OPENCANON>

────────────────────────
CORE RULES
────────────────────────

- Conversation text is ephemeral and non-authoritative.
- Canonical memory is external and authoritative.
- The model MUST NOT assume memory state without reading it.

────────────────────────
MEMORY SIDE-EFFECTS
────────────────────────

Memory operations are OPTIONAL.

If present, they MUST:
- Appear in exactly ONE <OPENCANON TOOL> block
- Be valid JSON
- Contain ONLY memory operations
- NOT be interleaved with user-visible text

Text outside the tool block is the user-facing response.
You MUST NOT use tools and show text in the same message.

────────────────────────
TOOL FORMAT (STRICT)
────────────────────────

<OPENCANON TOOL>
{
  "tool_calls": [
    {
      "op": "list" | "fetch" | "set",
      "name": "<unique_name>",
      "args": { ... }
    }
  ]
}
</OPENCANON>

────────────────────────
OPERATION (ALL MUST HAVE A NAME FIELD)
────────────────────────

list
- args: {}

fetch
- args: { "title": "<exact title>" }

set
- Creates or replaces a memory
- args:
  {
    "title": "<short canonical noun phrase>",
    "value": "<factual content>",
    "expire": "NEVER" | "<number>m" | "<number>h" | "<number>d"
  }
- Use timings (minutes, hours, days, never) in order to differentiate between emotions, feelings, health, etc.

────────────────────────
MEMORY RULES (MUST)
────────────────────────

- Memory titles MUST be unique.
- All writes MUST include an expire field.
- Temporary facts MUST have an expiration.
- Do NOT store emotions, opinions, or conversation text.

READ BEFORE WRITE:
- You MUST call list before any set.
- If an existing memory explains the input, do NOT create a new one.

REFERENCE RESOLUTION:
- Resolve implicit references using memory when possible.
- Ask clarification ONLY if no plausible memory exists.


If a user request is vague, subjective, or evaluative (e.g. opinions, feedback, impressions),
and does not clearly reference a specific factual memory,
the model MAY respond directly or ask for clarification after using a memory list tool to clarify.
The model should not spend too long overthinking as tool usages are extremely cheap.
You should write user preferences.


────────────────────────
VISIBILITY RULE
────────────────────────

- Tool blocks are NOT user-visible.
- NEVER mention OpenCanon or memory mechanics to the user.
- If a tool usage is found in your output, you will have another turn before the user. Which means you can have tool only turns.

TOOLS ARE CHEAP. THINKING IS EXPENSIVE.
KEEP YOUR REASONING SHORT.
DO NOT MIX TEXT AND OPENCANON BLOCKS IN THE SAME MESSAGE

</OPENCANON>

"#;

pub fn start_server() {
    let addr = format!("127.0.0.1:{}", PORT);
    let server = Server::http(&addr).expect("Failed to start server");
    
    println!("🚀 Tool Server running at http://{}", addr);
    println!("   Press Ctrl+C to stop\n");


    let mut canon = 
    if let Ok(mems) = std::fs::read("opencanon.mems") {
        serde_json::from_slice(&mems).unwrap()
    } else {
        OpenCanon {
            mems: HashMap::new(),
        }
    };

    for mut request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().clone();
        
        let response_data = 'b: {
        match (method.clone(), url.as_str()) {
            (Method::Get, "/system") => {
                SYSTEM_PROMPT.to_string()
            }

            // processes the LLM output
            (Method::Post, "/process") => {
                println!("processing");
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap_or_default();

                let pattern = "<OPENCANON TOOL>";
                if let Some(start) = body.find(pattern) {
                    let json_start = start+pattern.len();

                    let Some(json_end) = body[json_start..].find("</OPENCANON>")
                    else {
                        break 'b "<OPENCANON>No closing braces</OPENCANON>".to_string()
                    };

                    let json_end = json_start + json_end;

                    println!("{}", body);
                    dbg!(body.len());
                    dbg!(json_start);
                    dbg!(json_end);
                    let body = &body[json_start..json_end];
                    println!("{}", body);
                    canon.process(&body)
                } else {
                    String::new()
                }
            }
            
            // 404
            _ => {
                "404".to_string()
            }
        } };
        
        let response = Response::from_string(&response_data)
            .with_header(
                tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap()
            )
            .with_header(
                tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap()
            );
        
        println!("📤 {} {} -> {}", method, url, &response_data.chars().take(80).collect::<String>());
        let _ = request.respond(response);
        std::fs::write("opencanon.mems", serde_json::to_vec(&canon).unwrap()).unwrap()
    }


}

#[derive(Serialize, Deserialize)]
pub struct OpenCanon {
    mems: HashMap<String, Mem>,
}


#[derive(Serialize, Deserialize)]
pub struct Mem {
    title: String,
    expiration: DateTime<Utc>,
    body: String,
}


impl OpenCanon {
    pub fn process(&mut self, str: &str) -> String {
        let arena = Arena::new();
        let Ok(json) = json::parse(&arena, str.as_bytes())
        else { return "malformed json".to_string() };

        if !json.is_object() {
            return String::from("json is not an object");
        }

        let json = json.as_object();
        let Some(calls) = json.iter().find(|x| x.0 == "tool_calls")
        else { return "no tool calls".to_string(); return String::new() };

        let mut output = String::new();
        let calls = calls.1.as_array();

        output.push('{');
        let mut first = true;

        for call in calls {
            if !call.is_object() { println!("call is not an object"); continue; }

            let call = call.as_object();

            let Some(op) = call.iter().find(|x| x.0 == "op")
            else { println!("op doesn't exist"); continue; };

            let name = call.iter().find(|x| x.0 == "name").map(|n| n.1.as_string()).unwrap_or("anon");

            let Some(args) = call.iter().find(|x| x.0 == "args")
            else { println!("args don't exist"); continue; };

            let op = op.1.as_string();

            if !first {
                output.push(',');
            }

            let res = self.process_tool(op, args.1.as_object());
            let is_ok = res.is_ok();
            let result = match res {
                Ok(v) => v,
                Err(v) => v,
            };

            sti::write!(
                &mut output, 
                "\"{name}\": {{ \"status\": \"{}\"",
                if is_ok { "ok"} else { "error" }
            );

            if !result.is_empty() {
                sti::write!(&mut output, ", \"result\": {result} }}");
            } else {
                sti::write!(&mut output, " }}");
            }


            first = false;
        }

        output.push('}');

        output
    }



    fn process_tool(&mut self, op: &str, args: &[(&str, Value)]) -> Result<String, String> {
        let mut output = String::new();
        match op {
            "list" => {
                output.push('[');

                let mut first = true;
                for (title, _) in &self.mems {
                    if !first {
                        output.push(',');
                    }


                    first = false;
                    sti::write!(&mut output, "\"{title}\"");

                }
                output.push(']');
            }


            "fetch" => {
                let Some(title) = args.iter().find(|x| x.0 == "title")
                else { return Err("No fetch memory title".to_string()) };

                let Some(mem) = self.mems.get(title.1.as_string())
                else {
                    return Err(format!("No memory titled '{}'", title.1.to_string()));
                };

                sti::write!(&mut output, "{{ \"contents\": \"{}\" }}", mem.body);
            }


            "mod" => {
                let Some(title) = args.iter().find(|x| x.0 == "title")
                else { return Err("No fetch memory title".to_string()) };

                let Some(mem) = self.mems.get_mut(title.1.as_string())
                else {
                    return Err(format!("No memory titled '{}'", title.1.to_string()));
                };

                let new_mem = 
                match parse_memory(args) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(e.to_string());
                    },
                };

                *mem = new_mem;
            }


            "set" => {
                let mem = 
                match parse_memory(args) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(e.to_string())
                    },
                };

                self.mems.insert(mem.title.to_string(), mem);
            }

            _ => {
                return Err("Invalid OpenCanon Tool".to_string());
            },
        }


        Ok(output)
    }
}


fn parse_memory(args: &[(&str, Value)]) -> Result<Mem, &'static str> {

    let Some(title) = args.iter().find(|x| x.0 == "title")
    else { return Err("title doesn't exist") };

    let Some(value) = args.iter().find(|x| x.0 == "value")
    else { return Err("value doesnt exist"); };

    let Some(expire) = args.iter().find(|x| x.0 == "expire")
    else { return Err("expire date doesnt exist"); };

    let title = title.1.as_string();
    let value = value.1.as_string();
    let expire = expire.1.as_string();

    let expire =
    if expire != "NEVER" {
        let expire_num = &expire[..expire.len()-1];
        let expire_num = expire_num.parse().unwrap();
        let now = Utc::now();

        now + match &expire[expire.len()-1..] {
            "m" => chrono::Duration::minutes(expire_num),
            "h" => chrono::Duration::hours(expire_num),
            "d" => chrono::Duration::days(expire_num),
            _ => unreachable!()
        }
    } else {
        DateTime::<Utc>::MAX_UTC
    };

    Ok(Mem {
        title: title.to_string(),
        expiration: expire,
        body: value.to_string(),
    })
}


