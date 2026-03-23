use std::collections::HashMap;
use super::ast::{Value, Decl};
use super::interpreter::{eval, EvalError};

pub struct Service {
    pub name: String,
    pub vars: HashMap<String, Value>,
}

pub struct Manager {
    pub services: HashMap<String, Service>,
    pub current_service: Option<String>,
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            services: HashMap::new(),
            current_service: None,
        }
    }

    pub async fn create_service(&mut self, name: String, decls: Vec<Decl>)
    -> Result<(), EvalError>
    {
        let mut service = Service {
            name: name.clone(),
            vars: HashMap::new(),
        };
        self.current_service = Some(name.clone());

        let mut env: Vec<(String, Value)> = vec![];  // accumulate evaluated vars here

        for decl in decls {
            match decl {
                Decl::VarDecl { name, val } |
                Decl::DefDecl { name, val, .. } => {
                    let value = eval(&val, &env, self).await?;  // pass env, not empty vec
                    env.push((name.clone(), value.clone()));    // make it visible to later decls
                    service.vars.insert(name, value);
                }
                Decl::TableDecl { .. } => {
                    return Err(EvalError::NotImplemented);
                }
            }
        }

        self.services.insert(name.clone(), service);
        Ok(())
    }

    pub async fn lookup(&mut self, ident: &str) -> Result<Value, EvalError> {
        if let Some(service_name) = self.current_service.clone() {
            if let Some(service) = self.services.get(&service_name) {
                if let Some(value) = service.vars.get(ident) {
                    return Ok(value.clone() as Value);
                }
            }
        }
        Err(EvalError::LookupError(format!("Variable '{}' not found", ident)))
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

        // service foo { var x = 1; }
        let decls = vec![
            Decl::VarDecl {
                name: "x".to_string(),
                val: Expr::Literal { val: Value::Number { val: 1 } },
            },
        ];

        manager.create_service("foo".to_string(), decls).await.unwrap();

        // lookup should find x in the foo service
        let result = manager.lookup("x").await.unwrap();
        assert_eq!(result, Value::Number { val: 1 });
    }

    #[tokio::test]
    async fn test_create_service_with_def() {
        let mut manager = Manager::new();

        // service foo { var x = 2; def f = x + 3; }
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

        let result = manager.lookup("f").await.unwrap();
        assert_eq!(result, Value::Number { val: 5 });
    }

    #[tokio::test]
    async fn test_lookup_missing_var_returns_error() {
        let mut manager = Manager::new();
        manager.create_service("foo".to_string(), vec![]).await.unwrap();

        let result = manager.lookup("nonexistent").await;
        assert!(result.is_err());
    }
}