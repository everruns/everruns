//! TestMath Capability - calculator tools for testing tool calling

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;

/// TestMath capability - calculator tools for testing tool calling
pub struct TestMathCapability;

impl Capability for TestMathCapability {
    fn id(&self) -> &str {
        CapabilityId::TEST_MATH
    }

    fn name(&self) -> &str {
        "Test Math"
    }

    fn description(&self) -> &str {
        "Testing capability: adds calculator tools (add, subtract, multiply, divide) for tool calling tests."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("calculator")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to math tools. Use them for calculations: add, subtract, multiply, divide.")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(AddTool),
            Box::new(SubtractTool),
            Box::new(MultiplyTool),
            Box::new(DivideTool),
        ]
    }
}

// ============================================================================
// Tool: add
// ============================================================================

/// Tool that adds two numbers
pub struct AddTool;

#[async_trait]
impl Tool for AddTool {
    fn name(&self) -> &str {
        "add"
    }

    fn description(&self) -> &str {
        "Add two numbers together and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The first number"
                },
                "b": {
                    "type": "number",
                    "description": "The second number"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a + b;

        ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "add",
            "a": a,
            "b": b
        }))
    }
}

// ============================================================================
// Tool: subtract
// ============================================================================

/// Tool that subtracts two numbers
pub struct SubtractTool;

#[async_trait]
impl Tool for SubtractTool {
    fn name(&self) -> &str {
        "subtract"
    }

    fn description(&self) -> &str {
        "Subtract the second number from the first and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The number to subtract from"
                },
                "b": {
                    "type": "number",
                    "description": "The number to subtract"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a - b;

        ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "subtract",
            "a": a,
            "b": b
        }))
    }
}

// ============================================================================
// Tool: multiply
// ============================================================================

/// Tool that multiplies two numbers
pub struct MultiplyTool;

#[async_trait]
impl Tool for MultiplyTool {
    fn name(&self) -> &str {
        "multiply"
    }

    fn description(&self) -> &str {
        "Multiply two numbers together and return the result."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The first number"
                },
                "b": {
                    "type": "number",
                    "description": "The second number"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let result = a * b;

        ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "multiply",
            "a": a,
            "b": b
        }))
    }
}

// ============================================================================
// Tool: divide
// ============================================================================

/// Tool that divides two numbers
pub struct DivideTool;

#[async_trait]
impl Tool for DivideTool {
    fn name(&self) -> &str {
        "divide"
    }

    fn description(&self) -> &str {
        "Divide the first number by the second and return the result. Returns an error if dividing by zero."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "number",
                    "description": "The dividend (number to be divided)"
                },
                "b": {
                    "type": "number",
                    "description": "The divisor (number to divide by)"
                }
            },
            "required": ["a", "b"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let a = arguments.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let b = arguments.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);

        if b == 0.0 {
            return ToolExecutionResult::tool_error("Cannot divide by zero");
        }

        let result = a / b;

        ToolExecutionResult::success(serde_json::json!({
            "result": result,
            "operation": "divide",
            "a": a,
            "b": b
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn test_capability_metadata() {
        let cap = TestMathCapability;

        assert_eq!(cap.id(), "test_math");
        assert_eq!(cap.name(), "Test Math");
        assert_eq!(cap.icon(), Some("calculator"));
        assert_eq!(cap.category(), Some("Testing"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = TestMathCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 4);
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"add"));
        assert!(tool_names.contains(&"subtract"));
        assert!(tool_names.contains(&"multiply"));
        assert!(tool_names.contains(&"divide"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = TestMathCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("math tools"));
    }

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get("test_math").unwrap();

        assert_eq!(cap.id(), "test_math");
        assert_eq!(cap.tools().len(), 4);
    }

    #[tokio::test]
    async fn test_add_tool() {
        let tool = AddTool;
        let result = tool.execute(serde_json::json!({"a": 5, "b": 3})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 8.0);
            assert_eq!(value.get("operation").unwrap().as_str().unwrap(), "add");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_subtract_tool() {
        let tool = SubtractTool;
        let result = tool.execute(serde_json::json!({"a": 10, "b": 4})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 6.0);
            assert_eq!(
                value.get("operation").unwrap().as_str().unwrap(),
                "subtract"
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_multiply_tool() {
        let tool = MultiplyTool;
        let result = tool.execute(serde_json::json!({"a": 6, "b": 7})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 42.0);
            assert_eq!(
                value.get("operation").unwrap().as_str().unwrap(),
                "multiply"
            );
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_divide_tool() {
        let tool = DivideTool;
        let result = tool.execute(serde_json::json!({"a": 20, "b": 4})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("result").unwrap().as_f64().unwrap(), 5.0);
            assert_eq!(value.get("operation").unwrap().as_str().unwrap(), "divide");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_divide_by_zero() {
        let tool = DivideTool;
        let result = tool.execute(serde_json::json!({"a": 10, "b": 0})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("divide by zero"));
        } else {
            panic!("Expected tool error for division by zero");
        }
    }
}
