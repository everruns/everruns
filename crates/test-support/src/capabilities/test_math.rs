//! TestMath Capability - calculator tools for testing tool calling

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use serde_json::Value;

pub const TEST_MATH_CAPABILITY_ID: &str = "test_math";

/// TestMath capability - calculator tools for testing tool calling
pub struct TestMathCapability;

impl Capability for TestMathCapability {
    fn id(&self) -> &str {
        TEST_MATH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Test Math"
    }

    fn description(&self) -> &str {
        "Testing capability: adds calculator tools (add, subtract, multiply, divide) for tool calling tests."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Тестова математика",
            "Тестова можливість: додає інструменти калькулятора (add, subtract, multiply, divide) для тестів виклику інструментів.",
        )]
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

    fn display_name(&self) -> Option<&str> {
        Some("Add")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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

    fn display_name(&self) -> Option<&str> {
        Some("Subtract")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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

    fn display_name(&self) -> Option<&str> {
        Some("Multiply")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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

    fn display_name(&self) -> Option<&str> {
        Some("Divide")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
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
    use serde_json::json;

    #[tokio::test]
    async fn registered_math_tools_preserve_operands_and_operation_results() {
        let tools = TestMathCapability.tools();
        assert_eq!(
            tools.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
            ["add", "subtract", "multiply", "divide"]
        );
        for (tool, expected) in tools.iter().zip([-4.5, -10.5, -22.5, -2.5]) {
            let ToolExecutionResult::Success(value) = tool.execute(json!({"a":-7.5,"b":3})).await
            else {
                panic!("{} should return a result", tool.name());
            };
            assert_eq!(
                value,
                json!({"a":-7.5,"b":3.0,"operation":tool.name(),"result":expected})
            );
        }
    }

    #[tokio::test]
    async fn division_rejects_both_signed_zero_divisors() {
        for zero in [0.0, -0.0] {
            let ToolExecutionResult::ToolError(error) =
                DivideTool.execute(json!({"a":10,"b":zero})).await
            else {
                panic!("zero divisor must fail");
            };
            assert_eq!(error, "Cannot divide by zero");
        }
    }
}
