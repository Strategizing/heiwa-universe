import asyncio
import os
import json
from nats.aio.client import Client as NATS
import google.generativeai as genai

async def main():
    nc = NATS()
    nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
    await nc.connect(nats_url)
    
    genai.configure(api_key=os.getenv("GEMINI_API_KEY"))
    model = genai.GenerativeModel('gemini-1.5-pro-latest')

    # Agent capability broadcast
    async def announce():
        while True:
            await nc.publish("heiwa.mesh.capability.broadcast", json.dumps({
                "agent_id": "gemini-strategist-1",
                "capabilities": ["mcp.reasoning", "mcp.planning", "mcp.synthesis"],
                "status": "ready"
            }).encode())
            await asyncio.sleep(30)

    # Listen for raw directives
    async def handle_directive(msg):
        data = json.loads(msg.data.decode())
        prompt = data.get("prompt", "")
        
        print(f"Strategist received directive: {prompt}")
        
        # Simulate planning via Gemini
        prompt_instruction = (
            f"You are the Heiwa Strategist. Convert this directive into a structured JSON plan for the V2 Blackboard.\n"
            f"You must break the directive into actionable steps.\n"
            f"For each step, specify the 'instruction', the 'target_tool' (e.g., 'codex', 'openclaw', 'picoclaw', 'ollama'), "
            f"and crucially, the 'target_tier' based on the 7-Tier AI Ecosystem (e.g., 'tier1_local', 'tier5_heavy_code', 'tier2_fast_context').\n\n"
            f"Directive: {prompt}\n\n"
            f"Return ONLY valid JSON in this format:\n"
            f"{{\"steps\": [{{\"instruction\": \"...\", \"target_tool\": \"...\", \"target_tier\": \"...\"}}]}}"
        )
        
        response = model.generate_content(prompt_instruction)
        
        try:
            # Strip potential markdown blocks
            json_str = response.text.replace("```json", "").replace("```", "").strip()
            plan_data = json.loads(json_str)
            steps = plan_data.get("steps", [])
        except json.JSONDecodeError:
            print("Failed to parse Gemini output as JSON. Falling back to single generic step.")
            steps = [{"instruction": response.text, "target_tool": "ollama", "target_tier": "tier1_local"}]
            
        # Broadcast the planned task to the mesh
        await nc.publish("heiwa.tasks.new", json.dumps({
            "task_id": f"task-{os.urandom(4).hex()}",
            "type": "mcp_task",
            "steps": steps
        }).encode())

    await nc.subscribe("heiwa.core.request", cb=handle_directive)
    
    print("Gemini Strategist initialized and listening on the Blackboard...")
    
    asyncio.create_task(announce())
    
    # Keep running
    while True:
        await asyncio.sleep(1)

if __name__ == '__main__':
    asyncio.run(main())
