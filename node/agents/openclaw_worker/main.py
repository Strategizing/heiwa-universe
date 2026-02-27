import asyncio
import os
import json
from nats.aio.client import Client as NATS
from playwright.async_api import async_playwright

async def main():
    nc = NATS()
    nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
    await nc.connect(nats_url)
    
    # Agent capability broadcast
    async def announce():
        while True:
            await nc.publish("heiwa.mesh.capability.broadcast", json.dumps({
                "agent_id": "openclaw-worker-1",
                "capabilities": ["mcp.browser.navigate", "mcp.browser.read", "mcp.research"],
                "status": "ready"
            }).encode())
            await asyncio.sleep(30)

    # Listen for MCP tasks directed to this agent or capability
    async def handle_task(msg):
        data = json.loads(msg.data.decode())
        url = data.get("url", "https://example.com")
        
        print(f"OpenClaw received research task for: {url}")
        
        # Simple Playwright interaction
        async with async_playwright() as p:
            browser = await p.chromium.launch()
            page = await browser.new_page()
            await page.goto(url)
            content = await page.content()
            title = await page.title()
            await browser.close()
        
        # Broadcast the results back to the mesh
        await nc.publish("heiwa.tasks.exec.result", json.dumps({
            "task_id": data.get("task_id", "unknown"),
            "result": {
                "title": title,
                "length": len(content),
                "summary": "Scraped successfully."
            },
            "status": "completed"
        }).encode())

    # We listen to general new tasks. A real bidding engine would bid first, but we accept it here for MVP
    await nc.subscribe("heiwa.tasks.new", cb=handle_task)
    
    print("OpenClaw Worker initialized and listening on the Blackboard...")
    
    asyncio.create_task(announce())
    
    while True:
        await asyncio.sleep(1)

if __name__ == '__main__':
    asyncio.run(main())
