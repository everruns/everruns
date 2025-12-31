//! TestWeather Capability - mock weather tools for testing tool calling

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;

/// TestWeather capability - mock weather tools for testing tool calling
pub struct TestWeatherCapability;

impl Capability for TestWeatherCapability {
    fn id(&self) -> &str {
        CapabilityId::TEST_WEATHER
    }

    fn name(&self) -> &str {
        "Test Weather"
    }

    fn description(&self) -> &str {
        "Testing capability: adds mock weather tools (get_weather, get_forecast) for tool calling tests."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud-sun")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to weather tools. Use get_weather for current conditions and get_forecast for multi-day forecasts.")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(GetWeatherTool), Box::new(GetForecastTool)]
    }
}

// ============================================================================
// Tool: get_weather
// ============================================================================

/// Tool that returns mock weather data for a location
pub struct GetWeatherTool;

#[async_trait]
impl Tool for GetWeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }

    fn description(&self) -> &str {
        "Get the current weather for a location. Returns temperature, conditions, humidity, and wind speed."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city or location name (e.g., 'New York', 'London', 'Tokyo')"
                },
                "units": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"],
                    "description": "Temperature units. Defaults to 'celsius'."
                }
            },
            "required": ["location"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let location = arguments
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let units = arguments
            .get("units")
            .and_then(|v| v.as_str())
            .unwrap_or("celsius");

        // Generate deterministic mock weather based on location hash
        let hash = location
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));
        let temp_c = ((hash % 35) as i32) + 5; // 5-40°C range
        let temp = if units == "fahrenheit" {
            (temp_c as f64 * 9.0 / 5.0) + 32.0
        } else {
            temp_c as f64
        };

        let conditions = match hash % 5 {
            0 => "sunny",
            1 => "partly cloudy",
            2 => "cloudy",
            3 => "rainy",
            _ => "windy",
        };

        let humidity = (hash % 50) + 30; // 30-80%
        let wind_speed = (hash % 30) + 5; // 5-35 km/h

        ToolExecutionResult::success(serde_json::json!({
            "location": location,
            "temperature": temp,
            "units": units,
            "conditions": conditions,
            "humidity": humidity,
            "wind_speed_kmh": wind_speed,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }
}

// ============================================================================
// Tool: get_forecast
// ============================================================================

/// Tool that returns mock weather forecast for a location
pub struct GetForecastTool;

#[async_trait]
impl Tool for GetForecastTool {
    fn name(&self) -> &str {
        "get_forecast"
    }

    fn description(&self) -> &str {
        "Get the weather forecast for a location for the next several days."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "The city or location name (e.g., 'New York', 'London', 'Tokyo')"
                },
                "days": {
                    "type": "integer",
                    "description": "Number of days to forecast (1-7). Defaults to 3."
                },
                "units": {
                    "type": "string",
                    "enum": ["celsius", "fahrenheit"],
                    "description": "Temperature units. Defaults to 'celsius'."
                }
            },
            "required": ["location"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let location = arguments
            .get("location")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let days = arguments
            .get("days")
            .and_then(|v| v.as_u64())
            .unwrap_or(3)
            .min(7) as usize;

        let units = arguments
            .get("units")
            .and_then(|v| v.as_str())
            .unwrap_or("celsius");

        // Generate deterministic mock forecast based on location hash
        let hash = location
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_add(b as u32));

        let today = chrono::Utc::now().date_naive();
        let mut forecast_days = Vec::new();

        for day_offset in 0..days {
            let day_hash = hash.wrapping_add(day_offset as u32 * 7);
            let temp_c = ((day_hash % 35) as i32) + 5;
            let temp_high = if units == "fahrenheit" {
                (temp_c as f64 * 9.0 / 5.0) + 32.0
            } else {
                temp_c as f64
            };
            let temp_low = temp_high - 8.0 - ((day_hash % 5) as f64);

            let conditions = match day_hash % 5 {
                0 => "sunny",
                1 => "partly cloudy",
                2 => "cloudy",
                3 => "rainy",
                _ => "windy",
            };

            let date = today + chrono::Duration::days(day_offset as i64);

            forecast_days.push(serde_json::json!({
                "date": date.to_string(),
                "high": temp_high,
                "low": temp_low,
                "conditions": conditions,
                "precipitation_chance": (day_hash % 100) as i32
            }));
        }

        ToolExecutionResult::success(serde_json::json!({
            "location": location,
            "units": units,
            "days": days,
            "forecast": forecast_days
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn test_capability_metadata() {
        let cap = TestWeatherCapability;

        assert_eq!(cap.id(), "test_weather");
        assert_eq!(cap.name(), "Test Weather");
        assert_eq!(cap.icon(), Some("cloud-sun"));
        assert_eq!(cap.category(), Some("Testing"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = TestWeatherCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 2);
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"get_weather"));
        assert!(tool_names.contains(&"get_forecast"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = TestWeatherCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("weather tools"));
    }

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get("test_weather").unwrap();

        assert_eq!(cap.id(), "test_weather");
        assert_eq!(cap.tools().len(), 2);
    }

    #[tokio::test]
    async fn test_get_weather_tool() {
        let tool = GetWeatherTool;
        let result = tool
            .execute(serde_json::json!({"location": "New York"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("location").unwrap().as_str().unwrap(), "New York");
            assert!(value.get("temperature").is_some());
            assert!(value.get("conditions").is_some());
            assert!(value.get("humidity").is_some());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_weather_fahrenheit() {
        let tool = GetWeatherTool;
        let result = tool
            .execute(serde_json::json!({"location": "London", "units": "fahrenheit"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("units").unwrap().as_str().unwrap(), "fahrenheit");
            // Fahrenheit temps should be higher than Celsius
            let temp = value.get("temperature").unwrap().as_f64().unwrap();
            assert!(temp > 30.0); // At least 30°F
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_forecast_tool() {
        let tool = GetForecastTool;
        let result = tool
            .execute(serde_json::json!({"location": "Tokyo", "days": 5}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("location").unwrap().as_str().unwrap(), "Tokyo");
            assert_eq!(value.get("days").unwrap().as_u64().unwrap(), 5);
            let forecast = value.get("forecast").unwrap().as_array().unwrap();
            assert_eq!(forecast.len(), 5);
            // Check first day has expected fields
            let first_day = &forecast[0];
            assert!(first_day.get("date").is_some());
            assert!(first_day.get("high").is_some());
            assert!(first_day.get("low").is_some());
            assert!(first_day.get("conditions").is_some());
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_forecast_default_days() {
        let tool = GetForecastTool;
        let result = tool
            .execute(serde_json::json!({"location": "Paris"}))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value.get("days").unwrap().as_u64().unwrap(), 3); // Default is 3
            let forecast = value.get("forecast").unwrap().as_array().unwrap();
            assert_eq!(forecast.len(), 3);
        } else {
            panic!("Expected success");
        }
    }
}
