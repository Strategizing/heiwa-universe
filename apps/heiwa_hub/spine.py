# fleets/hub/spine.py
import asyncio
import os
import uvicorn
from fastapi import FastAPI, Request

from heiwa_sdk.nervous_system import HeiwaNervousSystem
from heiwa_sdk.translator import HeiwaTranslator

app = FastAPI(title="Heiwa Cloud HQ (The Spine)")

# --- State ---
nerve = HeiwaNervousSystem()
translator = HeiwaTranslator()

@app.on_event("startup")
async def startup_event():
    print("[SPINE] Waking up...")
    try:
        await nerve.connect()
        # Verify Stream
        try:
            await nerve.js.add_stream(name="HEIWA", subjects=["heiwa.*"])
            print("[SPINE] 'HEIWA' stream ready.")
        except Exception as e:
            print(f"[SPINE] Stream note: {e}")
    except Exception as e:
        print(f"[SPINE] Failed to connect to Nervous System: {e}")
        # We don't exit, we might recover or it might be a config issue

@app.on_event("shutdown")
async def shutdown_event():
    print("[SPINE] Going to sleep...")
    await nerve.disconnect()

@app.get("/")
async def health_check():
    return {"status": "online", "system": "heiwa-limited"}

@app.post("/webhook")
async def receive_directive(request: Request):
    """
    Receives Natural Language from Discord (or curl).
    Translates -> Publishes to NATS.
    """
    try:
        body = await request.json()
        content = body.get("content", "")
        sender = body.get("author", "unknown_sovereign")

        print(f"[SPINE] Hearing voice from {sender}: '{content}'")

        # 1. Cognition (Translate)
        directive = translator.translate(content)
        
        # 2. Action (Publish)
        # We enrich the payload with the sender
        directive["payload"]["requested_by"] = sender
        
        ack = await nerve.publish_directive(
            subject=directive["subject"],
            data=directive["payload"]
        )
        
        return {
            "status": "dispatched", 
            "directive": directive["subject"], 
            "seq": ack.seq
        }

    except Exception as e:
        print(f"[SPINE] Cognitive Dissonance: {e}")
        return {"status": "error", "message": str(e)}

if __name__ == "__main__":
    # Local Dev execution
    port = int(os.getenv("PORT", 8000))
    uvicorn.run(app, host="0.0.0.0", port=port)