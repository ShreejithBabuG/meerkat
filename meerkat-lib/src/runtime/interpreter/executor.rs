use crate::ast::{Value, ActionStmt};
use crate::runtime::Manager;
use super::evaluator::{eval, EvalContext, EvalError};

/// The effect produced by executing a single statement.
pub enum ExecuteEffect {
    /// Statement completed with no binding or value.
    None,
    /// A `let` binding: the name and value to add to env.
    Binding(String, Value),
    /// An expression statement was evaluated: the result value.
    ExprValue(Value),
}

#[async_recursion::async_recursion]
pub async fn execute(
    stmt: &ActionStmt,
    env: &[(String, Value)],
    manager: &mut Manager,
    service_name: &str,
) -> Result<ExecuteEffect, EvalError> {
    match stmt {
        ActionStmt::Assign { var, expr } => {
            let value = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            manager.assign(service_name, var, value).await?;
            Ok(ExecuteEffect::None)
        }
        ActionStmt::Do(expr) => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            match val {
                Value::ActionClosure { stmts, env: closure_env, service_name: action_svc } => {
                    if manager.remote_services.contains_key(&action_svc) {
                        manager.remote_action(&action_svc, stmts, closure_env).await?;
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    } else {
                        let mut exec_env = closure_env.clone();
                        for s in &stmts {
                            if let ExecuteEffect::Binding(name, val) =
                                execute(s, &exec_env, manager, &action_svc).await?
                            {
                                exec_env.push((name, val));
                            }
                        }
                    }
                    Ok(ExecuteEffect::None)
                }
                _ => Err(EvalError::TypeError("do expects an action".to_string())),
            }
        }
        ActionStmt::Assert(expr) => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            match val {
                Value::Bool { val: true } => Ok(ExecuteEffect::None),
                Value::Bool { val: false } => Err(EvalError::TypeError("Assertion failed".to_string())),
                _ => Err(EvalError::TypeError("assert expects a boolean".to_string())),
            }
        }
        ActionStmt::Let { name, expr } => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            Ok(ExecuteEffect::Binding(name.clone(), val))
        }
        ActionStmt::Expr(expr) => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            Ok(ExecuteEffect::ExprValue(val))
        }
        ActionStmt::Insert { .. } => Err(EvalError::NotImplemented),
    }
}
