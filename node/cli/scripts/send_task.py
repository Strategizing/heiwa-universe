import asyncio
import json
import sys
import os

# Ensure project root is on sys.path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../")))

import nats
from fleets.hub.protocol import Subject, Payload

async def send_command():
    # Connect to NATS
    nc = await nats.connect("nats://localhost:4222")
    
    # The Command
    command = {
        "type": "research_task",
        "instruction": "Ping the network and verify latency.",
        "params": {"target": "google.com", "packets": 3}
    }
    
    # Wrap in Protocol
    payload = {
        Payload.SENDER_ID: "devon-cli",
        Payload.TYPE: Subject.CORE_REQUEST.value,
        Payload.DATA: command
    }
    
    # Send
    await nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
    print(f"🚀 Sent Command: {command['instruction']}")
    
    await nc.drain()

if __name__ == "__main__":
    asyncio.run(send_command())
