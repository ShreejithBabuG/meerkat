use std::io::{self, BufRead, IsTerminal, Write};

use meerkat_lib::runtime::ast::{Stmt, Value};
use meerkat_lib::runtime::interpreter::{execute, ExecuteEffect};
use meerkat_lib::runtime::parser::ReplParseResult;
use meerkat_lib::runtime::parser::parser::{parse_file, parse_repl};
use meerkat_lib::runtime::Manager;

const PROMPT: &str = "meerkat> ";
const PROMPT_CONT: &str = "       > ";

pub async fn run_repl(
    mut manager: Manager,
    remote_url_map: std::collections::HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let is_tty = stdin.is_terminal();

    if is_tty {
        println!("Meerkat REPL  (Ctrl-D to exit)");
        println!("Enter service definitions, @test blocks, statements, or expressions.");
        println!();
    }

    if !remote_url_map.is_empty() {
        let mut n = meerkat_lib::net::NetworkActor::new(meerkat_lib::net::types::NodeType::Server).await
            .map_err(|e| format!("Network error: {}", e))?;
        let listen_addr = meerkat_lib::net::Address::new("/ip4/0.0.0.0/tcp/0");
        n.handle_command(meerkat_lib::net::NetworkCommand::Listen { addr: listen_addr }).await;
        manager.network = Some(n);
    }

    // Persistent environment for let bindings across REPL inputs
    let mut repl_env: Vec<(String, Value)> = Vec::new();

    let mut buffer = String::new();
    let mut continuation = false;
    let mut lines = stdin.lock().lines();

    loop {
        if is_tty {
            if continuation {
                print!("{}", PROMPT_CONT);
            } else {
                print!("{}", PROMPT);
            }
            io::stdout().flush()?;
        }

        let line = match lines.next() {
            Some(Ok(l)) => l,
            Some(Err(e)) => return Err(e.into()),
            None => break,
        };

        buffer.push_str(&line);
        buffer.push('\n');

        if buffer.trim().is_empty() {
            buffer.clear();
            continuation = false;
            continue;
        }

        match parse_repl(&buffer) {
            ReplParseResult::Incomplete => {
                continuation = true;
            }
            ReplParseResult::Error(msg) => {
                eprintln!("Parse error: {}", msg);
                buffer.clear();
                continuation = false;
            }
            ReplParseResult::Complete(stmts) => {
                for stmt in stmts {
                    match exec_stmt(stmt, &mut manager, &mut repl_env, &remote_url_map).await {
                        Ok(Some(output)) => println!("{}", output),
                        Ok(None) => {}
                        Err(e) => eprintln!("Error: {}", e),
                    }
                }
                buffer.clear();
                continuation = false;
            }
        }
    }

    if is_tty {
        println!();
    }
    Ok(())
}

async fn exec_stmt(
    stmt: Stmt,
    manager: &mut Manager,
    repl_env: &mut Vec<(String, Value)>,
    remote_url_map: &std::collections::HashMap<String, String>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match stmt {
        Stmt::Service { name, decls } => {
            manager.create_service(name.clone(), decls).await
                .map_err(|e| format!("Service '{}': {}", name, e))?;
            Ok(Some(format!("Service '{}' loaded.", name)))
        }
        Stmt::Test { service, stmts } => {
            manager.run_test(&service, &stmts).await
                .map_err(|e| format!("@test({}): {}", service, e))?;
            Ok(Some(format!("@test({}) passed.", service)))
        }
        Stmt::Import { path, service: svc_name } => {
            if let Some(url) = remote_url_map.get(&svc_name) {
                manager.remote_services.insert(
                    svc_name.clone(),
                    meerkat_lib::net::Address::new(url.as_str()),
                );
                return Ok(Some(format!(
                    "Remote service '{}' registered at {}.", svc_name, url
                )));
            }
            let import_stmts = parse_file(&path)
                .map_err(|e| format!("Import '{}': {}", path, e))?;
            let mut loaded = Vec::new();
            for s in import_stmts {
                if let Stmt::Service { name, decls } = s {
                    manager.create_service(name.clone(), decls).await
                        .map_err(|e| format!("Imported service '{}': {}", name, e))?;
                    loaded.push(name);
                }
            }
            Ok(Some(format!("Imported service(s): {}.", loaded.join(", "))))
        }
        Stmt::ActionStmt(action_stmt) => {
            let effect = execute(&action_stmt, repl_env, manager, "")
                .await
                .map_err(|e| format!("{}", e))?;
            match effect {
                ExecuteEffect::Binding(name, val) => {
                    repl_env.push((name, val));
                    Ok(None)
                }
                ExecuteEffect::ExprValue(val) => Ok(Some(val.to_string())),
                ExecuteEffect::None => Ok(None),
            }
        }
        other => {
            Ok(Some(format!(
                "(not yet supported in REPL: {:?})",
                std::mem::discriminant(&other)
            )))
        }
    }
}
