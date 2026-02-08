---
name: weather
description: "Get current weather and forecasts using OpenWeather API"
category: productivity
status: active
---

# Weather Skill

Provides weather information using the OpenWeather API (cloud-based, no local data).

## Prerequisites

1. Get a free API key from [OpenWeatherMap](https://openweathermap.org/api)
2. Add to environment or config:
   ```bash
   export OPENWEATHER_API_KEY="your_key_here"
   ```

## Tools Provided

### `weather_current`
Get current weather for a location.
- **Args**: `location` (city name or "lat,lon")
- **Example**: `{"tool": "weather_current", "location": "London"}`
- **Returns**: Temperature, conditions, humidity, wind speed

### `weather_forecast`
Get 5-day weather forecast.
- **Args**: `location`, `days` (1-5, default: 3)
- **Example**: `{"tool": "weather_forecast", "location": "New York", "days": 3}`

## Configuration

```toml
[skills.weather]
enabled = true
api_key = "${OPENWEATHER_API_KEY}"  # Or hardcode (not recommended)
units = "metric"  # or "imperial" for Fahrenheit
```

## Usage Examples

**Current weather:**
```
> What's the weather in Tokyo?
✓ Tokyo, Japan
  Temperature: 18°C (64°F)
  Conditions: Partly cloudy
  Humidity: 65%
  Wind: 12 km/h SW
```

**Forecast:**
```
> Show me the 3-day weather forecast for San Francisco
✓ San Francisco 3-Day Forecast:
  Today: 22°C, Sunny
  Tomorrow: 20°C, Cloudy  
  Day 3: 19°C, Light rain
```

## Implementation Notes

- Uses OpenWeather API (free tier: 60 calls/min)
- Cloud-based, no local data storage
- Supports city names, coordinates, zip codes
- Returns data in metric or imperial units
