use crate::ast::{
    Expr, Value
};
use crate::runtime::Manager;

#[async_recursion::async_recursion]
pub async fn eval(expr: &Expr, env: &Vec<(String, Value)>, manager: &mut Manager) -> Value {
    match expr {
        Expr::Literal { val } => val.clone(),
        Expr::Call { func, args } => {
            let func_val = eval(func, env, manager).await;
            let mut arg_vals = Vec::new();
            for arg in args {
                arg_vals.push(eval(arg, env, manager).await);
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
                _ => panic!("Attempting to call a non-function value"),
            }
        }
        Expr::Variable { ident } => {
            for (var_name, var_val) in env.iter().rev() {
                if var_name == ident {
                    return var_val.clone();
                }
            }
            // variable not found in env, so ask the Manager to look up its value
            manager.lookup(ident).await     // may result in a network call to the service that owns this variable
        }
        _ => unimplemented!(),

    }
}

