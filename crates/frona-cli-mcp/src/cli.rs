use std::process::ExitCode;

use clap::{Arg, Command};
use frona_api_types::mcp::BridgeToolInfo;

use crate::client::BridgeClient;

pub async fn run(client: BridgeClient, args: Vec<String>) -> ExitCode {
    match run_inner(client, args).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run_inner(client: BridgeClient, args: Vec<String>) -> Result<ExitCode, crate::client::Error> {
    let positionals: Vec<&str> = args
        .iter()
        .skip(1)
        .take_while(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let has_help = args.iter().any(|a| a == "--help" || a == "-h" || a == "help");

    match (positionals.first().copied(), positionals.get(1).copied()) {
        (None, _) | (Some("help"), None) => {
            print_top_level_help();
            Ok(ExitCode::SUCCESS)
        }
        (Some("list"), _) => {
            let servers = client.list_servers().await?;
            if servers.is_empty() {
                println!("No MCP servers available.");
            } else {
                for s in &servers {
                    let desc = s.description.as_deref().unwrap_or("");
                    println!("{:<20} {:<30} ({} tools)  {}", s.handle, s.display_name, s.tool_count, desc);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        (Some("help"), Some(server)) => {
            if let Some(tool) = positionals.get(2) {
                print_tool_help_by_name(&client, server, tool).await
            } else {
                print_server_help(&client, server).await?;
                Ok(ExitCode::SUCCESS)
            }
        }
        (Some(server), Some("help")) => {
            print_server_help(&client, server).await?;
            Ok(ExitCode::SUCCESS)
        }
        (Some(server), None) if has_help => {
            print_server_help(&client, server).await?;
            Ok(ExitCode::SUCCESS)
        }
        (Some(server), Some(tool)) if has_help => {
            print_tool_help_by_name(&client, server, tool).await
        }
        (Some(server), Some(tool)) => {
            call_tool(&client, server, tool, &args).await
        }
        (Some(server), None) => {
            eprintln!("Missing tool name. Run: mcpctl {server} --help");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn print_top_level_help() {
    println!(
        "\
mcpctl — CLI bridge for MCP server tools

USAGE:
    mcpctl <SERVER> <TOOL> [OPTIONS]
    mcpctl list

DISCOVERY:
    mcpctl list                          List available servers
    mcpctl <server> --help               List tools on a server
    mcpctl <server> <tool> --help        Show tool parameters

EXAMPLES:
    mcpctl github list_repos --owner octocat
    mcpctl slack send_message --channel \"#general\" --text \"Hello\"

ENVIRONMENT:
    FRONA_TOKEN_FILE    Path to JWT auth token file (required)
    FRONA_API_URL       Base URL of the frona server (required)"
    );
}

async fn print_server_help(
    client: &BridgeClient,
    server: &str,
) -> Result<(), crate::client::Error> {
    let detail = client.server_tools(server).await?;
    let desc = detail.description.as_deref().unwrap_or("");

    println!("{} — {}", detail.handle, desc);
    println!();
    println!("TOOLS:");

    let max_name = detail.tools.iter().map(|t| t.name.len()).max().unwrap_or(10);
    for t in &detail.tools {
        println!("    {:<width$}  {}", t.name, t.description, width = max_name);
    }

    println!();
    println!("USAGE:");
    println!("    mcpctl {} <TOOL> [OPTIONS]", detail.handle);
    println!("    mcpctl {} <TOOL> --help", detail.handle);
    Ok(())
}

async fn print_tool_help_by_name(
    client: &BridgeClient,
    server: &str,
    tool: &str,
) -> Result<ExitCode, crate::client::Error> {
    let detail = client.server_tools(server).await?;
    if let Some(t) = detail.tools.iter().find(|t| t.name == tool) {
        print_tool_help(server, t);
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("Unknown tool '{tool}' on server '{server}'.");
        Ok(ExitCode::FAILURE)
    }
}

fn print_tool_help(server: &str, tool: &BridgeToolInfo) {
    println!("{server} {} — {}", tool.name, tool.description);
    println!();

    let props = tool
        .input_schema
        .get("properties")
        .and_then(|v| v.as_object());
    let required: Vec<&str> = tool
        .input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    if let Some(props) = props {
        println!("OPTIONS:");
        let max_name = props.keys().map(|k| k.len()).max().unwrap_or(10);
        for (name, schema) in props {
            let type_str = schema_type_label(schema);
            let desc = schema
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let req_marker = if required.contains(&name.as_str()) {
                " [required]"
            } else {
                ""
            };
            println!(
                "    --{:<width$}  <{}>  {}{}",
                name,
                type_str,
                desc,
                req_marker,
                width = max_name,
            );
        }
        println!();
    }

    println!("USAGE:");
    let example_args: String = required
        .iter()
        .map(|r| format!(" --{r} <value>"))
        .collect();
    println!("    mcpctl {server} {}{}", tool.name, example_args);
}

fn schema_type_label(schema: &serde_json::Value) -> &str {
    match schema.get("type").and_then(|v| v.as_str()) {
        Some("string") => "STRING",
        Some("integer") => "INT",
        Some("number") => "NUMBER",
        Some("boolean") => "BOOL",
        Some("array") => "JSON",
        Some("object") => "JSON",
        _ => "VALUE",
    }
}

async fn call_tool(
    client: &BridgeClient,
    server: &str,
    tool: &str,
    args: &[String],
) -> Result<ExitCode, crate::client::Error> {
    let detail = client.server_tools(server).await?;
    let tool_info = detail.tools.iter().find(|t| t.name == tool);

    let Some(tool_info) = tool_info else {
        eprintln!("Unknown tool '{tool}' on server '{server}'.");
        eprintln!("Run: mcpctl {server} --help");
        return Ok(ExitCode::FAILURE);
    };

    let arguments = parse_tool_args(tool_info, args)?;

    let result = client.call_tool(server, tool, arguments).await?;
    if result.is_error {
        eprintln!("{}", result.content);
        Ok(ExitCode::FAILURE)
    } else {
        println!("{}", result.content);
        Ok(ExitCode::SUCCESS)
    }
}

fn parse_tool_args(
    tool_info: &BridgeToolInfo,
    args: &[String],
) -> Result<serde_json::Value, crate::client::Error> {
    let props = tool_info
        .input_schema
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let required_fields: Vec<String> = tool_info
        .input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let mut cmd = Command::new("mcpctl")
        .disable_help_flag(true)
        .no_binary_name(true);

    let param_names: Vec<String> = props.keys().cloned().collect();

    for (name, schema) in &props {
        let type_str = schema
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("string");
        let desc: &'static str = Box::leak(
            schema
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
                .into_boxed_str(),
        );
        let is_required = required_fields.iter().any(|r| r == name);
        let name_static: &'static str = Box::leak(name.clone().into_boxed_str());

        let arg = match type_str {
            "boolean" => Arg::new(name_static)
                .long(name_static)
                .help(desc)
                .num_args(0..=1)
                .default_missing_value("true")
                .required(is_required),
            _ => Arg::new(name_static)
                .long(name_static)
                .help(desc)
                .required(is_required),
        };
        cmd = cmd.arg(arg);
    }

    let flag_args: Vec<&str> = args
        .iter()
        .skip(1)
        .skip_while(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    let matches = cmd.try_get_matches_from(&flag_args).map_err(|e| {
        crate::client::Error::Api {
            status: 0,
            body: e.to_string(),
        }
    })?;

    let mut arguments = serde_json::Map::new();
    for name in &param_names {
        let schema = &props[name];
        let type_str = schema
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("string");

        match type_str {
            "boolean" => {
                if let Some(val) = matches.get_one::<String>(name.as_str()) {
                    let b = val == "true" || val == "1";
                    arguments.insert(name.clone(), serde_json::Value::Bool(b));
                }
            }
            "integer" => {
                if let Some(val) = matches.get_one::<String>(name.as_str()) {
                    if let Ok(n) = val.parse::<i64>() {
                        arguments.insert(name.clone(), serde_json::json!(n));
                    } else {
                        arguments.insert(name.clone(), serde_json::Value::String(val.clone()));
                    }
                }
            }
            "number" => {
                if let Some(val) = matches.get_one::<String>(name.as_str()) {
                    if let Ok(n) = val.parse::<f64>() {
                        arguments.insert(
                            name.clone(),
                            serde_json::Number::from_f64(n)
                                .map(serde_json::Value::Number)
                                .unwrap_or(serde_json::Value::String(val.clone())),
                        );
                    } else {
                        arguments.insert(name.clone(), serde_json::Value::String(val.clone()));
                    }
                }
            }
            "object" | "array" => {
                if let Some(val) = matches.get_one::<String>(name.as_str()) {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(val) {
                        arguments.insert(name.clone(), parsed);
                    } else {
                        arguments.insert(name.clone(), serde_json::Value::String(val.clone()));
                    }
                }
            }
            _ => {
                if let Some(val) = matches.get_one::<String>(name.as_str()) {
                    arguments.insert(name.clone(), serde_json::Value::String(val.clone()));
                }
            }
        }
    }

    Ok(serde_json::Value::Object(arguments))
}
