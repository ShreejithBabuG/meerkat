use std::collections::HashSet;
use crate::ast::{Expr, ActionStmt};

impl Expr {
    /// return free variables in expr wrt var_binded, used for
    /// 1. for extracting dependency of each def declaration
    /// 2. for evaluation a expression (substitution based evaluation)
    pub fn free_var(
        &self,
        reactive_names: &HashSet<String>,
        var_binded: &HashSet<String>,
    ) -> HashSet<String> {
        match self {
            Expr::Literal { .. } | Expr::Table { .. } => HashSet::new(),
            Expr::Variable { ident } => {
                if var_binded.contains(ident) {
                    HashSet::new()
                } else {
                    HashSet::from([ident.clone()])
                }
            }
            Expr::KeyVal { value, .. } => {
                value.free_var(reactive_names, var_binded)
            }
            Expr::Tuple { val } => {
                let mut free_vars = HashSet::new();
                for item in val {
                    free_vars.extend(item.free_var(reactive_names, var_binded));
                }
                free_vars
            }
            Expr::Unop { op: _, expr } => expr.free_var(reactive_names, var_binded),
            Expr::Binop { op: _, expr1, expr2 } => {
                let mut free_vars = expr1.free_var(reactive_names, var_binded);
                free_vars.extend(expr2.free_var(reactive_names, var_binded));
                free_vars
            }
            Expr::If { cond, expr1, expr2 } => {
                let mut free_vars = cond.free_var(reactive_names, var_binded);
                free_vars.extend(expr1.free_var(reactive_names, var_binded));
                free_vars.extend(expr2.free_var(reactive_names, var_binded));
                free_vars
            }
            Expr::Func { params, body } => {
                let mut new_binds = var_binded.clone();
                new_binds.extend(params.iter().cloned());
                body.free_var(reactive_names, &new_binds)
            }
            Expr::Call { func, args } => {
                let mut free_vars = func.free_var(reactive_names, var_binded);
                for arg in args {
                    free_vars.extend(arg.free_var(reactive_names, var_binded));
                }
                free_vars
            }
            Expr::Action(stmts) => {
                let mut free_vars = HashSet::new();
                for stmt in stmts {
                    match stmt {
                        ActionStmt::Assign { var: _, expr } => {
                            free_vars.extend(expr.free_var(reactive_names, var_binded));
                        }
                        ActionStmt::Do(expr) => {
                            free_vars.extend(expr.free_var(reactive_names, var_binded));
                        }
                        ActionStmt::Assert(expr) => {
                            free_vars.extend(expr.free_var(reactive_names, var_binded));
                        }
                        ActionStmt::Let { name: _, expr } => {
                            free_vars.extend(expr.free_var(reactive_names, var_binded));
                        }
                        ActionStmt::Expr(expr) => {
                            free_vars.extend(expr.free_var(reactive_names, var_binded));
                        }
                        ActionStmt::Insert { row, .. } => {
                            free_vars.extend(row.free_var(reactive_names, var_binded));
                        }
                    }
                }
                free_vars.difference(reactive_names).cloned().collect()
            }
            Expr::MemberAccess { .. } => {
                // member access on another service - no local free vars
                HashSet::new()
            }
            Expr::Select { table_name, where_clause, .. } => {
                let mut free_vars = where_clause.free_var(reactive_names, var_binded);
                free_vars.insert(table_name.clone());
                free_vars
            }
            Expr::Fold { operation, identity, .. } => {
                let mut free_vars = HashSet::new();
                free_vars.extend(operation.free_var(reactive_names, var_binded));
                free_vars.extend(identity.free_var(reactive_names, var_binded));
                free_vars
            }
        }
    }
}
