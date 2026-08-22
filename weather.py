import requests

def get_weather(city):
    # Using a public API for weather. 
    # Note: In a real scenario, an API key would be needed for services like OpenWeatherMap.
    # For this demo, I will use a mock response or a free open API if available.
    url = f"https://wttr.in/{city}?format=j1"
    response = requests.get(url)
    if response.status_code == 200:
        data = response.json()
        current = data['current_condition'][0]
        temp = current['temp_C']
        desc = current['weatherDesc'][0]['value']
        return f"The current weather in {city} is {temp}°C with {desc}."
    else:
        return "Could not retrieve weather data."

print(get_weather("London"))
