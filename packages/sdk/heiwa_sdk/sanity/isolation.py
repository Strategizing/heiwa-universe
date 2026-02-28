
import asyncio
import sys
import os

# Paths
ROOT_DIR = "/home/devon/heiwa-limited"
RUNTIME_SCRIPT = os.path.join(ROOT_DIR, "cli/scripts/agent_runtime.py")
INTERNAL_CONFIG = os.path.join(ROOT_DIR, "fleets/hub/agent.yaml")
CLIENT_CONFIG = os.path.join(ROOT_DIR, "satellites/vali-org/agent.yaml")
FIELD_OP_CONFIG = os.path.join(ROOT_DIR, "fleets/local-field-op/agent.yaml")

async def run_agent_test(name, config_path, test_prompt, forbidden_response_fragment=None, expected_response_fragment=None):
    print(f"\n--- Testing Agent: {name} ---")
    
    # Start the agent process
    cmd = ["python3", RUNTIME_SCRIPT, "--agent", config_path]
    process = await asyncio.create_subprocess_exec(
        *cmd,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE
    )
    
    try:
        # Wait for startup
        await asyncio.sleep(2)
        
        # Check startup logs
        # We need to read stdout without blocking. 
        # For simplicity in this test, we interact linearly.
        
        # Send Prompt
        print(f"Sending input: '{test_prompt}'")
        if process.stdin:
            process.stdin.write(f"{test_prompt}\n".encode())
            await process.stdin.drain()
        
        # Read response
        # We'll read a few lines
        output_buffer = ""
        for _ in range(10):
            try:
                line = await asyncio.wait_for(process.stdout.readline(), timeout=1.0)
                decoded_line = line.decode().strip()
                print(f"Agent Output: {decoded_line}")
                output_buffer += decoded_line + "\n"
            except asyncio.TimeoutError:
                break
        
        # Verification Logic
        success = True
        if forbidden_response_fragment and forbidden_response_fragment in output_buffer:
            print(f"FAILURE: Forbidden content '{forbidden_response_fragment}' found in output.")
            success = False
        
        if expected_response_fragment and expected_response_fragment not in output_buffer:
            print(f"FAILURE: Expected content '{expected_response_fragment}' NOT found in output.")
            success = False
            
        if success:
            print(f"SUCCESS: Agent {name} passed isolation test.")
        else:
            print(f"FAILED: Agent {name} failed isolation test.")

    finally:
        try:
            process.terminate()
            await process.wait()
        except:
            pass

async def main():
    print("Starting Runtime Segregation Verification...")
    
    # 1. Test Client Agent (Vali Org)
    # Scenario: Try to access 'railway' (Internal tool). Should fail.
    await run_agent_test(
        name="Client Agent (Vali Org)",
        config_path=CLIENT_CONFIG,
        test_prompt="Check railway status",
        expected_response_fragment="I do not have the 'railway' tool."
    )
    
    # 2. Test Internal Agent
    # Scenario: Try to access 'railway'. Should succeed.
    await run_agent_test(
        name="Heiwa Cloud HQ",
        config_path=INTERNAL_CONFIG,
        test_prompt="Check railway status",
        expected_response_fragment="Executing Railway tool..."
    )

    # 3. Test Local Field Op
    # Scenario: Try to access 'filesystem' (fs:read). Should succeed.
    await run_agent_test(
        name="Heiwa Field Op",
        config_path=FIELD_OP_CONFIG,
        
        test_prompt="Check filesystem",
        # We need to ensure the shim prints "Executing...". 
        # I will update the shim concurrently.
        expected_response_fragment="Executing Filesystem tool..." 
    )

if __name__ == "__main__":
    asyncio.run(main())