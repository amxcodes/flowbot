import asyncio
import websockets
import json

async def test_tool_call():
    async with websockets.connect("ws://localhost:3000/ws") as ws:
        # Ask FlowBot to read a file (should trigger read tool)
        await ws.send(json.dumps({"message": "Read the contents of Cargo.toml"}))
        
        async for msg in ws:
            data = json.loads(msg)
            if data.get("type") == "text_delta":
                print(data["delta"], end="", flush=True)
            elif data.get("type") == "done":
                print("\n✅ Stream finished")
                break

asyncio.run(test_tool_call())
