use crate::ast::{Value, ActionStmt};
use crate::runtime::Manager;
use super::evaluator::{eval, EvalContext, EvalError};

#[async_recursion::async_recursion]
pub async fn execute(
    stmt: &ActionStmt,
    env: &[(String, Value)],
    manager: &mut Manager,
    service_name: &str,
) -> Result<(), EvalError> {
    match stmt {
        ActionStmt::Assign { var, expr } => {
            let value = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            manager.assign(service_name, var, value).await
        }
        ActionStmt::Do(expr) => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            match val {
                Value::ActionClosure { stmts, env: closure_env, service_name: action_svc } => {
                    if manager.remote_services.contains_key(&action_svc) {
                        // Execute action remotely
                        manager.remote_action(&action_svc, stmts, closure_env).await?;
                        // Heuristic delay to allow remote propagation to complete
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    } else {
                        // Execute locally
                        for s in &stmts {
                            execute(s, &closure_env, manager, &action_svc).await?;
                        }
                    }
                    Ok(())
                }
                _ => Err(EvalError::TypeError("do expects an action".to_string())),
            }
        }
        ActionStmt::Assert(expr) => {
            let val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            match val {
                Value::Bool { val: true } => Ok(()),
                Value::Bool { val: false } => Err(EvalError::TypeError("Assertion failed".to_string())),
                _ => Err(EvalError::TypeError("assert expects a boolean".to_string())),
            }
        }
        ActionStmt::Let { name: _, expr } => {
            let _val = eval(expr, env, &mut EvalContext { manager, service_name }).await?;
            Ok(())
        }
        ActionStmt::Insert { .. } => Err(EvalError::NotImplemented),
    }
}
