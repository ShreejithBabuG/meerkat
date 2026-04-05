use clap::Parser;
use std::error::Error;
use meerkat_lib::runtime::ast::Stmt;
use meerkat_lib::runtime::Manager;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(short = 'f', long = "file", default_value = "test0.meerkat")]
    input_file: String,

    #[arg(short = 'v', long = "verbose", default_value_t = false)]
    verbose: bool,
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let log_level = if args.verbose {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Warn
    };
    env_logger::Builder::from_default_env()
        .filter_level(log_level)
        .init();

    let prog = meerkat_lib::runtime::parser::parser::parse_file(&args.input_file)
        .map_err(|e| format!("Parse error: {}", e))?;

    let mut manager = Manager::new();

    for stmt in &prog {
        match stmt {
            Stmt::Service { name, decls } => {
                manager.create_service(name.clone(), decls.clone()).await
                    .map_err(|e| format!("Service error: {}", e))?;
                println!("Service '{}' loaded", name);
            }
            Stmt::Test { service, stmts } => {
                manager.run_test(service, stmts).await
                    .map_err(|e| format!("Test failed in '{}': {}", service, e))?;
                println!("@test({}) passed", service);
            }
            Stmt::Import { path, service: _ } => {
                // resolve import path relative to the input file's directory
                let base_dir = std::path::Path::new(&args.input_file)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let import_path = base_dir.join(path);
                let import_stmts = meerkat_lib::runtime::parser::parser::parse_file(
                    import_path.to_str().unwrap()
                ).map_err(|e| format!("Import parse error: {}", e))?;
                for import_stmt in &import_stmts {
                    if let Stmt::Service { name, decls } = import_stmt {
                        manager.create_service(name.clone(), decls.clone()).await
                            .map_err(|e| format!("Import service error: {}", e))?;
                        println!("Imported service '{}'", name);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}
