use crate::ast::{
    Expr, Value, BinOp, UnOp, ActionStmt
};
use crate::runtime::Manager;
use std::collections::HashSet;

#[derive(Debug)]
pub enum EvalError {
    TypeError(String),
    NetworkError(String),
    LookupError(String),
    NotImplemented,
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            EvalError::TypeError(s) => write!(f, "Type error: {}", s),
            EvalError::NetworkError(s) => write!(f, "Network error: {}", s),
            EvalError::LookupError(s) => write!(f, "Lookup error: {}", s),
            EvalError::NotImplemented => write!(f, "Not yet implemented"),
        }
    }
}

impl std::error::Error for EvalError {}

#[async_recursion::async_recursion]
pub async fn eval(expr: &Expr, env: &Vec<(String, Value)>, manager: &mut Manager) -> Result<Value, EvalError> {
    match expr {
        Expr::Literal { val } => Ok(val.clone()),
        Expr::Call { func, args } => {
            let func_val = eval(func, env, manager).await?;
            let mut arg_vals = Vec::new();
            for arg in args {
                arg_vals.push(eval(arg, env, manager).await?);
            }
            match func_val {
                Value::Closure { params, body, env } => {
                    // create a new environment for the function call
                    let mut new_env = env.clone();
                    for (param, arg_val) in params.iter().zip(arg_vals) {
                        new_env.push((param.clone(), arg_val ));
                    }
                    eval(&body, &new_env, manager).await
                }
                _ => Err(EvalError::TypeError("Attempting to call a non-function value".to_string())),
            }
        }
        Expr::Variable { ident } => {
            for (var_name, var_val) in env.iter().rev() {
                if var_name == ident {
                    return Ok(var_val.clone());
                }
            }
            // variable not found in env, so ask the Manager to look up its value
            manager.lookup(ident).await     // may result in a network call to the service that owns this variable
        }
        Expr::Binop { op, expr1, expr2 } => {
            let val1 = eval(expr1, env, manager).await?;
            let val2 = eval(expr2, env, manager).await?;
            match (op, val1, val2) {
                (BinOp::Add, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 + v2 }),
                (BinOp::Sub, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 - v2 }),
                (BinOp::Mul, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 * v2 }),
                (BinOp::Div, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 / v2 }),
                (BinOp::Eq, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 == v2 }),
                (BinOp::Lt, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 < v2 }),
                (BinOp::Gt, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 > v2 }),
                (BinOp::And, Value::Bool { val: v1 }, Value::Bool { val: v2 }) => Ok(Value::Bool { val: v1 && v2 }),
                (BinOp::Or, Value::Bool { val: v1 }, Value::Bool { val: v2 }) => Ok(Value::Bool { val: v1 || v2 }),
                _ => Err(EvalError::TypeError("Type error in binary operation".to_string())),
            }
        }
        Expr::Unop { op, expr } => {
            let val = eval(expr, env, manager).await?;
            match (op, val) {
                (UnOp::Neg, Value::Number { val: v }) => Ok(Value::Number { val: -v }),
                (UnOp::Not, Value::Bool { val: v }) => Ok(Value::Bool { val: !v }),
                _ => Err(EvalError::TypeError("Type error in unary operation".to_string())),
            }
        }
        Expr::If { cond, expr1, expr2 } => {
            let cond_val = eval(cond, env, manager).await?;
            match cond_val {
                Value::Bool { val: true } => eval(expr1, env, manager).await,
                Value::Bool { val: false } => eval(expr2, env, manager).await,
                _ => Err(EvalError::TypeError("Condition must be boolean".to_string())),
            }
        }
        Expr::Func { params, body } => {
            // Build var_binded with ONLY the function parameters
            // (not the environment variables - those are potentially free)
            let mut var_binded: HashSet<String> = params.iter().cloned().collect();
            
            // Compute free variables (we don't have reactive_names in this context, use empty set)
            let reactive_names = HashSet::new();
            let free_vars = body.free_var(&reactive_names, &var_binded);
            
            // Filter environment to only include free variables
            let captured_env: Vec<(String, Value)> = env.iter()
                .filter(|(name, _)| free_vars.contains(name))
                .cloned()
                .collect();
            
            // Create a closure with minimal captured environment
            Ok(Value::Closure {
                params: params.clone(),
                body: body.clone(),
                env: captured_env,
            })
        }
        Expr::Action(stmts) => {
            // For actions, we also want to minimize the captured environment
            // Compute free variables in all statements
            // For now, capture the full environment as ActionStmt free_var is not implemented
            // TODO: implement free_var for ActionStmt and optimize this
            Ok(Value::ActionClosure {
                stmts: stmts.clone(),
                env: env.clone(),
            })
        }
        _ => Err(EvalError::NotImplemented),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Value, BinOp};
    use crate::runtime::Manager;

    #[tokio::test]
    async fn test_literal() {
        let mut manager = Manager::default();
        let env = vec![];
        let expr = Expr::Literal { val: Value::Number { val: 42 } };
        let result = eval(&expr, &env, &mut manager).await.unwrap();
        assert_eq!(result, Value::Number { val: 42 });
    }

    #[tokio::test]
    async fn test_binop_add() {
        let mut manager = Manager::default();
        let env = vec![];
        let expr = Expr::Binop {
            op: BinOp::Add,
            expr1: Box::new(Expr::Literal { val: Value::Number { val: 2 } }),
            expr2: Box::new(Expr::Literal { val: Value::Number { val: 3 } }),
        };
        let result = eval(&expr, &env, &mut manager).await.unwrap();
        assert_eq!(result, Value::Number { val: 5 });
    }

    #[tokio::test]
    async fn test_func_and_call() {
        let mut manager = Manager::default();
        let env = vec![];
        
        // Create a function: func(x) { x + 10 }
        let func_expr = Expr::Func {
            params: vec!["x".to_string()],
            body: Box::new(Expr::Binop {
                op: BinOp::Add,
                expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                expr2: Box::new(Expr::Literal { val: Value::Number { val: 10 } }),
            }),
        };
        
        // Call the function with argument 5
        let call_expr = Expr::Call {
            func: Box::new(func_expr),
            args: vec![Expr::Literal { val: Value::Number { val: 5 } }],
        };
        
        let result = eval(&call_expr, &env, &mut manager).await.unwrap();
        assert_eq!(result, Value::Number { val: 15 });
    }

    #[tokio::test]
    async fn test_action_creation() {
        let mut manager = Manager::default();
        let env = vec![];
        
        // Create an action: action { x = 5; }
        let action_expr = Expr::Action(vec![
            ActionStmt::Assign {
                var: "x".to_string(),
                expr: Expr::Literal { val: Value::Number { val: 5 } },
            },
        ]);
        
        let result = eval(&action_expr, &env, &mut manager).await.unwrap();
        
        // Should create an ActionClosure
        match result {
            Value::ActionClosure { stmts, env: _ } => {
                assert_eq!(stmts.len(), 1);
            }
            _ => panic!("Expected ActionClosure"),
        }
    }

    #[tokio::test]
    async fn test_closure_captures_only_free_vars() {
        let mut manager = Manager::default();
        
        // Environment with multiple variables
        let env = vec![
            ("a".to_string(), Value::Number { val: 1 }),
            ("b".to_string(), Value::Number { val: 2 }),
            ("c".to_string(), Value::Number { val: 3 }),
        ];
        
        // Create a function that only uses 'a': func(x) { x + a }
        let func_expr = Expr::Func {
            params: vec!["x".to_string()],
            body: Box::new(Expr::Binop {
                op: BinOp::Add,
                expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                expr2: Box::new(Expr::Variable { ident: "a".to_string() }),
            }),
        };
        
        let result = eval(&func_expr, &env, &mut manager).await.unwrap();
        
        // Check that closure only captures 'a', not 'b' or 'c'
        match result {
            Value::Closure { params, body: _, env: captured_env } => {
                assert_eq!(params.len(), 1);
                assert_eq!(captured_env.len(), 1); // Should only capture 'a'
                assert_eq!(captured_env[0].0, "a");
                assert_eq!(captured_env[0].1, Value::Number { val: 1 });
            }
            _ => panic!("Expected Closure"),
        }
    }
}