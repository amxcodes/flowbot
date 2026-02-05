import asyncio
import websockets
import json
import sys

async def test_gateway():
    uri = "ws://localhost:3000/ws"
    print(f"Connecting to {uri}...")
    try:
        async with websockets.connect(uri) as websocket:
            print("Connected!")
            
            # Send message
            msg = {"message": "Hello, this is a test from the verification script!"}
            print(f"Sending: {msg}")
            await websocket.send(json.dumps(msg))
            
            # Receive loop
            while True:
                try:
                    response_text = await asyncio.wait_for(websocket.recv(), timeout=60.0) # Wait up to a minute
                    response = json.loads(response_text)
                    
                    if response.get("type") == "done":
                        print("\n[Stream finished]")
                        # Don't break, keep listening for cron events
                        continue
                    
                    if response.get("type") == "text_delta":
                        sys.stdout.write(response.get("delta", ""))
                        sys.stdout.flush()
                        
                except asyncio.TimeoutError:
                    print("\nTimeout waiting for events")
                    break
                except websockets.exceptions.ConnectionClosed:
                    print("\nConnection closed")
                    break
    except Exception as e:
        print(f"Error: {e}")

if __name__ == "__main__":
    asyncio.run(test_gateway())
