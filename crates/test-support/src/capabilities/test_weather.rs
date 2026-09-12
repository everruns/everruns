//! TestWeather Capability - mock weather tools for testing tool calling

use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use serde_json::Value;

pub const TEST_WEATHER_CAPABILITY_ID: &str = "test_weather";

/// TestWeather capability - mock weather tools for testing tool calling
pub struct TestWeatherCapability;

impl Capability for TestWeatherCapability {
    fn id(&self) -> &str {
        TEST_WEATHER_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Test Weather"
    }

    fn description(&self) -> &str {
        "Testing capability: adds mock weather tools (get_weather, get_forecast) for tool calling tests."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Тестова погода",
            "Тестова можливість: додає імітаційні інструменти погоди (get_weather, get_forecast) для тестів виклику інструментів.",
        )]
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

    fn display_name(&self) -> Option<&str> {
        Some("Get Weather")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
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

    fn display_name(&self) -> Option<&str> {
        Some("Get Forecast")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
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
            .clamp(1, 7) as usize;

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
            let low_c = temp_c as f64 - 8.0 - ((day_hash % 5) as f64);
            let temp_low = if units == "fahrenheit" {
                (low_c * 9.0 / 5.0) + 32.0
            } else {
                low_c
            };

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
    use serde_json::json;

    #[tokio::test]
    async fn registered_weather_tool_returns_complete_unit_aware_observations() {
        let tools = TestWeatherCapability.tools();
        assert_eq!(
            tools.iter().map(|tool| tool.name()).collect::<Vec<_>>(),
            ["get_weather", "get_forecast"]
        );
        for (arguments, location, units, temperature, conditions, humidity, wind) in [
            (
                json!({"location":"A"}),
                "A",
                "celsius",
                35.0,
                "sunny",
                45,
                10,
            ),
            (
                json!({"location":"A","units":"fahrenheit"}),
                "A",
                "fahrenheit",
                95.0,
                "sunny",
                45,
                10,
            ),
            (
                json!({"location":"B"}),
                "B",
                "celsius",
                36.0,
                "partly cloudy",
                46,
                11,
            ),
        ] {
            let before = chrono::Utc::now();
            let ToolExecutionResult::Success(mut value) = tools[0].execute(arguments).await else {
                panic!("weather must succeed")
            };
            let after = chrono::Utc::now();
            let timestamp = value.as_object_mut().unwrap().remove("timestamp").unwrap();
            let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp.as_str().unwrap())
                .unwrap()
                .with_timezone(&chrono::Utc);
            assert!(timestamp >= before && timestamp <= after);
            assert_eq!(
                value,
                json!({"location":location,"units":units,"temperature":temperature,"conditions":conditions,"humidity":humidity,"wind_speed_kmh":wind})
            );
        }
    }

    #[tokio::test]
    async fn forecasts_convert_both_temperatures_and_bound_day_counts() {
        let tools = TestWeatherCapability.tools();
        for (units, temperatures) in [
            ("celsius", [(35.0, 27.0), (7.0, -3.0), (14.0, 2.0)]),
            ("fahrenheit", [(95.0, 80.6), (44.6, 26.6), (57.2, 35.6)]),
        ] {
            let before = chrono::Utc::now().date_naive();
            let ToolExecutionResult::Success(value) = tools[1]
                .execute(json!({"location":"A","units":units}))
                .await
            else {
                panic!("forecast must succeed")
            };
            let after = chrono::Utc::now().date_naive();
            let first = chrono::NaiveDate::parse_from_str(
                value["forecast"][0]["date"].as_str().unwrap(),
                "%Y-%m-%d",
            )
            .unwrap();
            assert!(first >= before && first <= after);
            let expected = temperatures.into_iter().zip([("sunny",65),("cloudy",72),("windy",79)]).enumerate().map(|(day,((high,low),(conditions,precipitation)))| json!({
                "date":(first+chrono::Duration::days(day as i64)).to_string(),"high":high,"low":low,"conditions":conditions,"precipitation_chance":precipitation
            })).collect::<Vec<_>>();
            assert_eq!(
                value,
                json!({"location":"A","units":units,"days":3,"forecast":expected})
            );
        }
        for (requested, expected) in [(0, 1), (1, 1), (7, 7), (8, 7), (u64::MAX, 7)] {
            let ToolExecutionResult::Success(value) = tools[1]
                .execute(json!({"location":"A","days":requested}))
                .await
            else {
                panic!("forecast must succeed")
            };
            assert_eq!(value["days"], expected);
            assert_eq!(
                value["forecast"].as_array().unwrap().len(),
                expected as usize
            );
            assert_eq!(value["units"], "celsius");
        }
    }
}
