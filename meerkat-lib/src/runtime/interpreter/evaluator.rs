use crate::ast::{
    Expr, Value, BinOp, UnOp
};
use crate::runtime::Manager;

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
}