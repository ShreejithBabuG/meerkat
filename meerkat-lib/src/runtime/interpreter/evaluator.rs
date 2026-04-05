use crate::ast::{
    Expr, Value, BinOp, UnOp, ActionStmt
};
use std::collections::HashSet;
use crate::runtime::manager::Manager;

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

/// Evaluation context: holds the stable execution state that doesn't
/// change per call frame. Passed as &mut so manager can be updated.
/// env is kept separate since it changes at each function call boundary.
pub struct EvalContext<'a> {
    pub manager: &'a mut Manager,
    pub service_name: &'a str,
}

#[async_recursion::async_recursion]
pub async fn eval(
    expr: &Expr,
    env: &[(String, Value)],
    ctx: &mut EvalContext<'_>,
) -> Result<Value, EvalError> {
    match expr {
        Expr::Literal { val } => Ok(val.clone()),

        Expr::Call { func, args } => {
            let func_val = eval(func, env, ctx).await?;
            let mut arg_vals = Vec::new();
            for arg in args {
                arg_vals.push(eval(arg, env, ctx).await?);
            }
            match func_val {
                Value::Closure { params, body, env: closure_env } => {
                    let mut new_env = closure_env.clone();
                    for (param, arg_val) in params.iter().zip(arg_vals) {
                        new_env.push((param.clone(), arg_val));
                    }
                    eval(&body, &new_env, ctx).await
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
            ctx.manager.lookup(ident, ctx.service_name).await
        }

        Expr::Binop { op, expr1, expr2 } => {
            let val1 = eval(expr1, env, ctx).await?;
            let val2 = eval(expr2, env, ctx).await?;
            match (op, val1, val2) {
                (BinOp::Add, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 + v2 }),
                (BinOp::Sub, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 - v2 }),
                (BinOp::Mul, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 * v2 }),
                (BinOp::Div, Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Number { val: v1 / v2 }),
                (BinOp::Eq,  Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 == v2 }),
                (BinOp::Lt,  Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 < v2 }),
                (BinOp::Gt,  Value::Number { val: v1 }, Value::Number { val: v2 }) => Ok(Value::Bool { val: v1 > v2 }),
                (BinOp::And, Value::Bool { val: v1 },   Value::Bool { val: v2 })   => Ok(Value::Bool { val: v1 && v2 }),
                (BinOp::Or,  Value::Bool { val: v1 },   Value::Bool { val: v2 })   => Ok(Value::Bool { val: v1 || v2 }),
                _ => Err(EvalError::TypeError("Type error in binary operation".to_string())),
            }
        }

        Expr::Unop { op, expr } => {
            let val = eval(expr, env, ctx).await?;
            match (op, val) {
                (UnOp::Neg, Value::Number { val: v }) => Ok(Value::Number { val: -v }),
                (UnOp::Not, Value::Bool { val: v })   => Ok(Value::Bool { val: !v }),
                _ => Err(EvalError::TypeError("Type error in unary operation".to_string())),
            }
        }

        Expr::If { cond, expr1, expr2 } => {
            let cond_val = eval(cond, env, ctx).await?;
            match cond_val {
                Value::Bool { val: true }  => eval(expr1, env, ctx).await,
                Value::Bool { val: false } => eval(expr2, env, ctx).await,
                _ => Err(EvalError::TypeError("Condition must be boolean".to_string())),
            }
        }

        Expr::Func { params, body } => {
            let var_binded: HashSet<String> = params.iter().cloned().collect();
            let free_vars = body.free_var(&HashSet::new(), &var_binded);
            let captured_env: Vec<(String, Value)> = env.iter()
                .filter(|(name, _)| free_vars.contains(name))
                .cloned()
                .collect();
            Ok(Value::Closure {
                params: params.clone(),
                body: body.clone(),
                env: captured_env,
            })
        }

        Expr::Action(stmts) => {
            // Capture only free variables from the local env (function args etc.)
            // Service vars/defs are looked up fresh via the manager at execution time
            use crate::ast::ActionStmt;
            let mut free_in_action: std::collections::HashSet<String> = std::collections::HashSet::new();
            for stmt in stmts {
                match stmt {
                    ActionStmt::Assign { expr, .. } |
                    ActionStmt::Do(expr) |
                    ActionStmt::Assert(expr) => {
                        free_in_action.extend(expr.free_var(&std::collections::HashSet::new(), &std::collections::HashSet::new()));
                    }
                    ActionStmt::Let { expr, .. } => {
                        free_in_action.extend(expr.free_var(&std::collections::HashSet::new(), &std::collections::HashSet::new()));
                    }
                    ActionStmt::Insert { row, .. } => {
                        free_in_action.extend(row.free_var(&std::collections::HashSet::new(), &std::collections::HashSet::new()));
                    }
                }
            }
            // Only capture vars from local env (not from service — those are looked up via manager)
            let captured_env: Vec<(String, Value)> = env.iter()
                .filter(|(name, _)| free_in_action.contains(name))
                .cloned()
                .collect();
            Ok(Value::ActionClosure {
                stmts: stmts.clone(),
                env: captured_env,
                service_name: ctx.service_name.to_string(),
            })
        }

        Expr::MemberAccess { service, member } => {
            // Check if service is remote
            if ctx.manager.remote_services.contains_key(service) {
                ctx.manager.remote_lookup(service, member).await
            } else {
                ctx.manager.lookup(member, service).await
            }
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
        let mut ctx = EvalContext { manager: &mut manager, service_name: "" };
        let expr = Expr::Literal { val: Value::Number { val: 42 } };
        let result = eval(&expr, &[], &mut ctx).await.unwrap();
        assert_eq!(result, Value::Number { val: 42 });
    }

    #[tokio::test]
    async fn test_binop_add() {
        let mut manager = Manager::default();
        let mut ctx = EvalContext { manager: &mut manager, service_name: "" };
        let expr = Expr::Binop {
            op: BinOp::Add,
            expr1: Box::new(Expr::Literal { val: Value::Number { val: 2 } }),
            expr2: Box::new(Expr::Literal { val: Value::Number { val: 3 } }),
        };
        let result = eval(&expr, &[], &mut ctx).await.unwrap();
        assert_eq!(result, Value::Number { val: 5 });
    }

    #[tokio::test]
    async fn test_func_and_call() {
        let mut manager = Manager::default();
        let mut ctx = EvalContext { manager: &mut manager, service_name: "" };
        let func_expr = Expr::Func {
            params: vec!["x".to_string()],
            body: Box::new(Expr::Binop {
                op: BinOp::Add,
                expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                expr2: Box::new(Expr::Literal { val: Value::Number { val: 10 } }),
            }),
        };
        let call_expr = Expr::Call {
            func: Box::new(func_expr),
            args: vec![Expr::Literal { val: Value::Number { val: 5 } }],
        };
        let result = eval(&call_expr, &[], &mut ctx).await.unwrap();
        assert_eq!(result, Value::Number { val: 15 });
    }

    #[tokio::test]
    async fn test_action_creation() {
        let mut manager = Manager::default();
        let mut ctx = EvalContext { manager: &mut manager, service_name: "" };
        let action_expr = Expr::Action(vec![
            ActionStmt::Assign {
                var: "x".to_string(),
                expr: Expr::Literal { val: Value::Number { val: 5 } },
            },
        ]);
        let result = eval(&action_expr, &[], &mut ctx).await.unwrap();
        match result {
            Value::ActionClosure { stmts, .. } => assert_eq!(stmts.len(), 1),
            _ => panic!("Expected ActionClosure"),
        }
    }

    #[tokio::test]
    async fn test_closure_captures_only_free_vars() {
        let mut manager = Manager::default();
        let mut ctx = EvalContext { manager: &mut manager, service_name: "" };
        let env = vec![
            ("a".to_string(), Value::Number { val: 1 }),
            ("b".to_string(), Value::Number { val: 2 }),
            ("c".to_string(), Value::Number { val: 3 }),
        ];
        let func_expr = Expr::Func {
            params: vec!["x".to_string()],
            body: Box::new(Expr::Binop {
                op: BinOp::Add,
                expr1: Box::new(Expr::Variable { ident: "x".to_string() }),
                expr2: Box::new(Expr::Variable { ident: "a".to_string() }),
            }),
        };
        let result = eval(&func_expr, &env, &mut ctx).await.unwrap();
        match result {
            Value::Closure { params, body: _, env: captured_env } => {
                assert_eq!(params.len(), 1);
                assert_eq!(captured_env.len(), 1);
                assert_eq!(captured_env[0].0, "a");
                assert_eq!(captured_env[0].1, Value::Number { val: 1 });
            }
            _ => panic!("Expected Closure"),
        }
    }
}
