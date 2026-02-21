use anyhow::{Result, anyhow};
use rhai::{AST, Dynamic, Engine, Scope};

/// Rhai script engine for glue logic in agents
pub struct ScriptEngine {
    engine: Engine,
    scope: Scope<'static>,
    ast: AST,
}

impl ScriptEngine {
    /// Create a new script engine and compile the source code
    pub fn new(source: &str) -> Result<Self> {
        let mut engine = Engine::new();

        // Set safety limits
        engine.set_max_expr_depths(50, 10); // Prevent deep recursion
        engine.set_max_string_size(1024 * 100); // 100KB max string
        engine.set_max_array_size(1000); // Max 1000 array elements

        // Disable dangerous features
        engine.set_allow_looping(false); // No while/for loops

        // Compile the script into an AST
        let ast = match engine.compile(source) {
            Ok(ast) => ast,
            Err(e) => return Err(anyhow!("Script compilation failed: {}", e)),
        };

        let mut scope = Scope::new();

        // Execute the script to populate the scope with variables/functions
        if !source.trim().is_empty()
            && let Err(e) = engine.run_ast_with_scope(&mut scope, &ast)
        {
            return Err(anyhow!("Script execution failed: {}", e));
        }

        Ok(Self { engine, scope, ast })
    }

    /// Check if a function exists in the script
    pub fn has_function(&self, fn_name: &str) -> bool {
        self.ast.iter_functions().any(|f| f.name == fn_name)
    }

    /// Call a function with string argument, return string result
    pub fn call_str(&mut self, fn_name: &str, arg: &str) -> Result<String> {
        // call_fn signature: call_fn(&mut scope, &AST, fn_name, args_tuple)
        match self.engine.call_fn::<Dynamic>(
            &mut self.scope,
            &self.ast,
            fn_name,
            (arg.to_string(),),
        ) {
            Ok(result) => Ok(result.to_string()),
            Err(e) => Err(anyhow!("Function '{}' call failed: {}", fn_name, e)),
        }
    }

    /// Call a function with multiple dynamic arguments
    pub fn call_function(&mut self, fn_name: &str, args: Vec<Dynamic>) -> Result<String> {
        let result = match args.len() {
            0 => {
                match self
                    .engine
                    .call_fn::<Dynamic>(&mut self.scope, &self.ast, fn_name, ())
                {
                    Ok(r) => r,
                    Err(e) => return Err(anyhow!("Call failed: {}", e)),
                }
            }
            1 => {
                match self.engine.call_fn::<Dynamic>(
                    &mut self.scope,
                    &self.ast,
                    fn_name,
                    (args[0].clone(),),
                ) {
                    Ok(r) => r,
                    Err(e) => return Err(anyhow!("Call failed: {}", e)),
                }
            }
            2 => {
                match self.engine.call_fn::<Dynamic>(
                    &mut self.scope,
                    &self.ast,
                    fn_name,
                    (args[0].clone(), args[1].clone()),
                ) {
                    Ok(r) => r,
                    Err(e) => return Err(anyhow!("Call failed: {}", e)),
                }
            }
            _ => return Err(anyhow!("Too many arguments (max 2 supported)")),
        };

        Ok(result.to_string())
    }

    /// Evaluate an expression and return the result
    pub fn eval(&mut self, expr: &str) -> Result<String> {
        match self
            .engine
            .eval_with_scope::<Dynamic>(&mut self.scope, expr)
        {
            Ok(result) => Ok(result.to_string()),
            Err(e) => Err(anyhow!("Evaluation failed: {}", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function() {
        let script = r#"
            fn greet(name) {
                "Hello, " + name + "!"
            }
        "#;

        let mut engine = ScriptEngine::new(script).unwrap();
        let result = engine.call_str("greet", "World").unwrap();
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_expression_eval() {
        let script = r#"
            let x = 10;
            let y = 20;
        "#;

        let mut engine = ScriptEngine::new(script).unwrap();
        let result = engine.eval("x + y").unwrap();
        assert_eq!(result, "30");
    }

    #[test]
    fn test_safety_limits() {
        // This should fail due to loop restriction
        let script = "for i in 0..100 { print(i); }";
        let result = ScriptEngine::new(script);
        assert!(result.is_err());
    }
}
