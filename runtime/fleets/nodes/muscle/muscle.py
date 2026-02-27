# fleets/nodes/muscle/muscle.py
import asyncio
import os
import json

from libs.heiwa_sdk.nervous_system import HeiwaNervousSystem

async def handle_task(msg):
    """
    Executes the directive.
    """
    subject = msg.subject
    data = msg.data.decode()
    print(f"\n[MUSCLE] >>> DIRECTIVE RECEIVED: {subject}")
    print(f"[MUSCLE] Payload: {data}")
    
    # Simulate Atomic Work
    print("[MUSCLE] Executing sovereign logic...")
    await asyncio.sleep(2) 
    
    # Acknowledge
    await msg.ack()
    print("[MUSCLE] TASK COMPLETE. SYNCING STATE...")

async def main():
    print("[MUSCLE] Waking up...")
    
    # USER CONFIG: Set this to your NATS URL.
    # If using Tailscale, use the Tailscale IP of the Railway node + 4222
    # Or if your Machine is on the same Mesh DNS, using the internal URL might work if routed.
    # Defaulting to the user-provided internal URL for 'Sovereign' context (assuming VPN routing)
    nats_url = os.getenv("NATS_URL", "nats://devon:noved@nats.railway.internal:4222")
    
    nerve = HeiwaNervousSystem(nats_url=nats_url)
    
    try:
        await nerve.connect()
        
        # Subscribe to all tasks
        print("[MUSCLE] Connected. Waiting for 'heiwa.tasks.*'...")
        await nerve.subscribe_worker("heiwa.tasks.>", handle_task)
        
        # Keep alive
        while True:
            await asyncio.sleep(1)

    except Exception as e:
        print(f"[MUSCLE] Error: {e}")
        await nerve.disconnect()

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("[MUSCLE] Resting.")
