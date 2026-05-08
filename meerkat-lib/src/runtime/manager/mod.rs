use std::collections::HashMap;
use tokio::sync::oneshot;
use tokio::time::Duration;
use super::ast::{Value, Decl, Expr, ActionStmt};
use super::interpreter::{eval, EvalContext, EvalError, execute, ExecuteEffect};
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
    /// Pending reply channels keyed by request_id
    pub pending_replies: HashMap<u64, oneshot::Sender<MeerkatMessage>>,
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            services: HashMap::new(),
            remote_services: HashMap::new(),
            network: None,
            pending_replies: HashMap::new(),
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
        // Check if service is remote
        if self.remote_services.contains_key(service_name) {
            return self.remote_lookup(service_name, ident).await;
        }

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


    /// Drain all pending network events and dispatch each to the matching
    /// oneshot channel in pending_replies. Non-matching events are dropped.
    pub fn dispatch_network_events(&mut self) {
        loop {
            let event = match self.network.as_mut() {
                Some(n) => n.try_recv_event(),
                None => break,
            };
            match event {
                Some(NetworkEvent::MessageReceived { msg, .. }) => {
                    let rid = match &msg {
                        MeerkatMessage::LookupResponse { request_id, .. } => Some(*request_id),
                        MeerkatMessage::LookupError { request_id, .. } => Some(*request_id),
                        MeerkatMessage::ActionResponse { request_id, .. } => Some(*request_id),
                        _ => None,
                    };
                    if let Some(id) = rid {
                        if let Some(tx) = self.pending_replies.remove(&id) {
                            let _ = tx.send(msg);
                        }
                    }
                }
                Some(_) => {}
                None => break,
            }
        }
    }

    /// Send a message and await a reply using tokio::select! for timeout.
    /// Encapsulates the duplicated send + register channel + await pattern
    /// shared by remote_lookup and remote_action.
    async fn send_and_await_reply(
        &mut self,
        addr: Address,
        msg: MeerkatMessage,
        request_id: u64,
        timeout_msg: String,
    ) -> Result<MeerkatMessage, EvalError> {
        // Send the message
        let net = self.network.as_mut()
            .ok_or_else(|| EvalError::NetworkError("No network layer available".to_string()))?;
        net.handle_command(NetworkCommand::SendMessage { addr, msg }).await;

        // Register oneshot channel for this request
        let (tx, mut rx) = oneshot::channel::<MeerkatMessage>();
        self.pending_replies.insert(request_id, tx);

        // Loop with pinned timeout + tokio::select!. Each iteration dispatches
        // pending network events then checks for reply, timeout, or yields 10ms.
        // The loop is required until the tokio::join! background message loop
        // architecture is implemented as a follow-up.
        let timeout = tokio::time::sleep(Duration::from_secs(15));
        tokio::pin!(timeout);

        loop {
            self.dispatch_network_events();
            tokio::select! {
                biased;
                result = &mut rx => {
                    return result.map_err(|_| {
                        EvalError::NetworkError("Reply channel closed".to_string())
                    });
                }
                _ = &mut timeout => {
                    self.pending_replies.remove(&request_id);
                    return Err(EvalError::NetworkError(timeout_msg));
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    }

    /// Get the network address for a remote service (strips the slug)
    fn remote_addr(&self, service: &str) -> Result<Address, EvalError> {
        let full_url = self.remote_services.get(service)
            .ok_or_else(|| EvalError::LookupError(format!("Remote service '{}' not found", service)))?;
        let addr_str = full_url.0.trim_end_matches(&format!("/{}", service));
        Ok(Address::new(addr_str))
    }

    /// Get our local address with peer ID for use as reply_to
    /// Replaces loopback/unspecified with the actual outbound IP
    async fn local_reply_addr(&mut self) -> String {
        let net = match self.network.as_mut() {
            Some(n) => n,
            None => return String::new(),
        };
        let peer_id = net.local_peer_id();
        let reply = net.handle_command(NetworkCommand::GetLocalAddresses).await;
        let public_ip = Self::get_public_ip();
        match reply {
            crate::net::NetworkReply::LocalAddresses { addrs } => {
                if let Some(addr) = addrs.first() {
                    let addr_str = addr.0
                        .replace("0.0.0.0", &public_ip)
                        .replace("127.0.0.1", &public_ip);
                    format!("{}/p2p/{}", addr_str, peer_id)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    /// Get the local machine's outbound IP address (non-loopback)
    pub fn get_public_ip() -> String {
        use std::net::UdpSocket;
        UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| { s.connect("8.8.8.8:80")?; s.local_addr() })
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|_| "127.0.0.1".to_string())
    }

    pub async fn remote_lookup(&mut self, service: &str, member: &str) -> Result<Value, EvalError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let addr = self.remote_addr(service)?;
        let request_id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let reply_to = self.local_reply_addr().await;

        let msg = MeerkatMessage::LookupRequest {
            request_id,
            service: service.to_string(),
            member: member.to_string(),
            reply_to,
        };

        let reply = self.send_and_await_reply(
            addr, msg, request_id,
            format!("Timeout waiting for remote lookup of {}.{}", service, member),
        ).await?;

        match reply {
            MeerkatMessage::LookupResponse { value, .. } => {
                let val: Value = serde_json::from_str(&value)
                    .map_err(|e| EvalError::NetworkError(e.to_string()))?;
                Ok(val)
            }
            MeerkatMessage::LookupError { error, .. } => {
                Err(EvalError::LookupError(error))
            }
            _ => Err(EvalError::NetworkError("Unexpected reply to lookup request".to_string())),
        }
    }

    pub async fn remote_action(&mut self, service: &str, stmts: Vec<ActionStmt>, env: Vec<(String, Value)>) -> Result<(), EvalError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ACTION_ID: AtomicU64 = AtomicU64::new(1);

        let addr = self.remote_addr(service)?;
        let request_id = NEXT_ACTION_ID.fetch_add(1, Ordering::SeqCst);
        let reply_to = self.local_reply_addr().await;

        let msg = MeerkatMessage::ActionRequest {
            request_id,
            service: service.to_string(),
            stmts,
            env,
            reply_to,
        };

        let reply = self.send_and_await_reply(
            addr, msg, request_id,
            format!("Timeout waiting for remote action on service '{}'", service),
        ).await?;

        match reply {
            MeerkatMessage::ActionResponse { success, error, .. } => {
                if success {
                    Ok(())
                } else {
                    Err(EvalError::NetworkError(
                        error.unwrap_or_else(|| "Remote action failed".to_string())
                    ))
                }
            }
            _ => Err(EvalError::NetworkError("Unexpected reply to action request".to_string())),
        }
    }

    pub async fn run_test(&mut self, service_name: &str, stmts: &[ActionStmt]) -> Result<(), EvalError> {
        self.run_test_with_env(service_name, stmts, &[]).await
    }

    pub async fn run_test_with_env(&mut self, service_name: &str, stmts: &[ActionStmt], initial_env: &[(String, Value)]) -> Result<(), EvalError> {
        let mut env: Vec<(String, Value)> = initial_env.to_vec();
        for stmt in stmts {
            match execute(stmt, &env, self, service_name).await? {
                ExecuteEffect::Binding(name, val) => env.push((name, val)),
                ExecuteEffect::None | ExecuteEffect::ExprValue(_) => {}
            }
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
