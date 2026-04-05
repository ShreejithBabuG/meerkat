use std::collections::HashMap;
use super::ast::{Value, Decl, Expr, ActionStmt};
use super::interpreter::{eval, EvalContext, EvalError};
use super::semantic_analysis::var_analysis::{calc_dep_srv, DependAnalysis};
use crate::net::{Address, NetworkCommand, NetworkEvent, MeerkatMessage, NetworkActor};
use crate::net::network_layer::NetworkLayer;

pub struct Service {
    pub name: String,
    pub vars: HashMap<String, Value>,   // vars + evaluated def values
    pub defs: HashMap<String, Expr>,    // original def expressions for re-evaluation
    pub dep: DependAnalysis,            // dependency graph + topo order
}

pub struct Manager {
    pub services: HashMap<String, Service>,
    /// Maps service name to remote address (for distributed services)
    pub remote_services: HashMap<String, Address>,
    /// Network actor for distributed communication
    pub network: Option<NetworkActor>,
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            services: HashMap::new(),
            remote_services: HashMap::new(),
            network: None,
        }
    }

    pub async fn create_service(&mut self, name: String, decls: Vec<Decl>)
        -> Result<(), EvalError>
    {
        let dep = calc_dep_srv(&decls);

        let mut service = Service {
            name: name.clone(),
            vars: HashMap::new(),
            defs: HashMap::new(),
            dep,
        };

        let mut env: Vec<(String, Value)> = vec![];
        let svc_name = name.clone();

        for decl in decls {
            match decl {
                Decl::VarDecl { name, val } => {
                    let value = eval(&val, &env, &mut EvalContext { manager: self, service_name: &svc_name }).await?;
                    env.push((name.clone(), value.clone()));
                    service.vars.insert(name, value);
                }
                Decl::DefDecl { name, val, .. } => {
                    let value = eval(&val, &env, &mut EvalContext { manager: self, service_name: &svc_name }).await?;
                    env.push((name.clone(), value.clone()));
                    service.vars.insert(name.clone(), value);
                    service.defs.insert(name, val);  // store original expr
                }
                Decl::TableDecl { .. } => {
                    return Err(EvalError::NotImplemented);
                }
            }
        }

        self.services.insert(name.clone(), service);
        Ok(())
    }

    pub async fn lookup(&mut self, ident: &str, service_name: &str) -> Result<Value, EvalError> {
        // If it's a def, re-evaluate from stored expression for freshness
        let def_expr = self.services.get(service_name)
            .and_then(|s| s.defs.get(ident))
            .cloned();

        if let Some(expr) = def_expr {
            let env: Vec<(String, Value)> = self.services
                .get(service_name)
                .map(|s| s.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();
            return eval(&expr, &env, &mut EvalContext { manager: self, service_name }).await;
        }

        // Otherwise return stored var value
        if let Some(service) = self.services.get(service_name) {
            if let Some(value) = service.vars.get(ident) {
                return Ok(value.clone());
            }
        }
        Err(EvalError::LookupError(format!("Variable '{}' not found in service '{}'", ident, service_name)))
    }

    pub async fn assign(&mut self, service_name: &str, var: &str, value: Value) -> Result<(), EvalError> {
        // update the var
        if let Some(service) = self.services.get_mut(service_name) {
            if service.vars.contains_key(var) {
                service.vars.insert(var.to_string(), value);
            } else {
                return Err(EvalError::LookupError(format!("Variable '{}' not found in service '{}'", var, service_name)));
            }
        } else {
            return Err(EvalError::LookupError(format!("Service '{}' not found", service_name)));
        }

        // propagate: re-evaluate defs that depend on this var in topo order
        self.propagate(service_name, var).await
    }

    async fn propagate(&mut self, service_name: &str, changed_var: &str) -> Result<(), EvalError> {
        // collect defs that need re-evaluation in topo order
        let topo_order: Vec<String> = self.services
            .get(service_name)
            .map(|s| s.dep.topo_order.clone())
            .unwrap_or_default();

        for def_name in topo_order {
            let needs_update = self.services
                .get(service_name)
                .and_then(|s| s.dep.dep_vars.get(&def_name))
                .map(|dep_vars| dep_vars.contains(changed_var))
                .unwrap_or(false);

            let is_def = self.services
                .get(service_name)
                .map(|s| s.defs.contains_key(&def_name))
                .unwrap_or(false);

            if needs_update && is_def {
                // build env from current var values
                let expr = self.services
                    .get(service_name)
                    .and_then(|s| s.defs.get(&def_name))
                    .cloned();

                if let Some(expr) = expr {
                    let env: Vec<(String, Value)> = self.services
                        .get(service_name)
                        .map(|s| s.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default();

                    let value = eval(&expr, &env, &mut EvalContext { manager: self, service_name }).await?;

                    if let Some(service) = self.services.get_mut(service_name) {
                        service.vars.insert(def_name, value);
                    }
                }
            }
        }
        Ok(())
    }

    #[async_recursion::async_recursion]
    pub async fn execute_action_stmt(&mut self, stmt: &ActionStmt, env: &[(String, Value)], service_name: &str) -> Result<(), EvalError> {
        match stmt {
            ActionStmt::Assign { var, expr } => {
                let value = eval(expr, env, &mut EvalContext { manager: self, service_name }).await?;
                self.assign(service_name, var, value).await
            }
            ActionStmt::Do(expr) => {
                let val = eval(expr, env, &mut EvalContext { manager: self, service_name }).await?;
                match val {
                    Value::ActionClosure { stmts, env: closure_env, service_name: action_svc } => {
                        // Use the action's own service context, not the caller's
                        // closure_env only contains free vars (function args etc.), not service vars
                        for s in &stmts {
                            self.execute_action_stmt(s, &closure_env, &action_svc).await?;
                        }
                        Ok(())
                    }
                    _ => Err(EvalError::TypeError("do expects an action".to_string())),
                }
            }
            ActionStmt::Assert(expr) => {
                let val = eval(expr, env, &mut EvalContext { manager: self, service_name }).await?;
                match val {
                    Value::Bool { val: true } => Ok(()),
                    Value::Bool { val: false } => Err(EvalError::TypeError("Assertion failed".to_string())),
                    _ => Err(EvalError::TypeError("assert expects a boolean".to_string())),
                }
            }
            ActionStmt::Let { name: _, expr } => {
                let _val = eval(expr, env, &mut EvalContext { manager: self, service_name }).await?;
                Ok(())
            }
            ActionStmt::Insert { .. } => Err(EvalError::NotImplemented),
        }
    }

    pub async fn remote_lookup(&mut self, service: &str, member: &str) -> Result<Value, EvalError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let full_url = self.remote_services.get(service)
            .ok_or_else(|| EvalError::LookupError(format!("Remote service '{}' not found", service)))?
            .clone();

        // Strip the service slug from the end of the address
        // e.g. /ip4/.../p2p/12D3.../s1 -> /ip4/.../p2p/12D3...
        let addr_str = full_url.0.trim_end_matches(&format!("/{}", service));
        let addr = Address::new(addr_str);

        let request_id = NEXT_ID.fetch_add(1, Ordering::SeqCst);

        // Get our local address + peer ID to include as reply_to
        let reply_to = {
            let net = self.network.as_mut().unwrap();
            let peer_id = net.local_peer_id();
            let reply = net.handle_command(NetworkCommand::GetLocalAddresses).await;
            match reply {
                crate::net::NetworkReply::LocalAddresses { addrs } => {
                    if let Some(addr) = addrs.first() {
                        format!("{}/p2p/{}", addr.0, peer_id)
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        };

        let msg = MeerkatMessage::LookupRequest {
            request_id,
            service: service.to_string(),
            member: member.to_string(),
            reply_to,
        };

        let net = self.network.as_mut()
            .ok_or_else(|| EvalError::NetworkError("No network layer available".to_string()))?;

        net.handle_command(NetworkCommand::SendMessage { addr, msg }).await;

        // Poll for response with timeout
        let start = std::time::Instant::now();
        loop {
            if start.elapsed().as_secs() > 15 {
                return Err(EvalError::NetworkError(format!(
                    "Timeout waiting for remote lookup of {}.{}", service, member
                )));
            }
            let net = self.network.as_mut().unwrap();
            if let Some(event) = net.try_recv_event() {
                match event {
                    NetworkEvent::MessageReceived {
                        msg: MeerkatMessage::LookupResponse { request_id: rid, value }, ..
                    } if rid == request_id => {
                        let val: Value = serde_json::from_str(&value)
                            .map_err(|e| EvalError::NetworkError(e.to_string()))?;
                        return Ok(val);
                    }
                    NetworkEvent::MessageReceived {
                        msg: MeerkatMessage::LookupError { request_id: rid, error }, ..
                    } if rid == request_id => {
                        return Err(EvalError::LookupError(error));
                    }
                    _ => {}
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    pub async fn run_test(&mut self, service_name: &str, stmts: &[ActionStmt]) -> Result<(), EvalError> {
        for stmt in stmts {
            self.execute_action_stmt(stmt, &[], service_name).await?;
        }
        Ok(())
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Decl, Expr, Value};

    #[tokio::test]
    async fn test_create_service_with_var() {
        let mut manager = Manager::new();
        let decls = vec![
            Decl::VarDecl {
                name: "x".to_string(),
                val: Expr::Literal { val: Value::Number { val: 1 } },
            },
        ];
        manager.create_service("foo".to_string(), decls).await.unwrap();
        let result = manager.lookup("x", "foo").await.unwrap();
        assert_eq!(result, Value::Number { val: 1 });
    }

    #[tokio::test]
    async fn test_create_service_with_def() {
        let mut manager = Manager::new();
        let decls = vec![
            Decl::VarDecl {
                name: "x".to_string(),
                val: Expr::Literal { val: Value::Number { val: 2 } },
            },
            Decl::DefDecl {
                name: "f".to_string(),
                val: Expr::Binop {
                    op: crate::ast::BinOp::Add,
                    expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                    expr2: Box::new(Expr::Literal { val: Value::Number { val: 3 } }),
                },
                is_pub: true,
            },
        ];
        manager.create_service("foo".to_string(), decls).await.unwrap();
        let result = manager.lookup("f", "foo").await.unwrap();
        assert_eq!(result, Value::Number { val: 5 });
    }

    #[tokio::test]
    async fn test_lookup_missing_var_returns_error() {
        let mut manager = Manager::new();
        manager.create_service("foo".to_string(), vec![]).await.unwrap();
        let result = manager.lookup("nonexistent", "foo").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_def_updates_after_var_change() {
        let mut manager = Manager::new();
        // service foo { var x = 1; def f = x + 10; }
        let decls = vec![
            Decl::VarDecl {
                name: "x".to_string(),
                val: Expr::Literal { val: Value::Number { val: 1 } },
            },
            Decl::DefDecl {
                name: "f".to_string(),
                val: Expr::Binop {
                    op: crate::ast::BinOp::Add,
                    expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                    expr2: Box::new(Expr::Literal { val: Value::Number { val: 10 } }),
                },
                is_pub: true,
            },
        ];
        manager.create_service("foo".to_string(), decls).await.unwrap();

        // f should be 11 initially
        let result = manager.lookup("f", "foo").await.unwrap();
        assert_eq!(result, Value::Number { val: 11 });

        // update x to 5, f should become 15
        manager.assign("foo", "x", Value::Number { val: 5 }).await.unwrap();
        let result = manager.lookup("f", "foo").await.unwrap();
        assert_eq!(result, Value::Number { val: 15 });
    }
}
