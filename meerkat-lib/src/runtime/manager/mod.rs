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

        for decl in decls {
            match decl {
                Decl::VarDecl { name, val } |
                Decl::DefDecl { name, val, .. } => {
                    let value = eval(&val, &vec![], self).await?;
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
