import sys
import os
import json

from heiwa_sdk.claw_adapter import ClawAdapter

def test_claw_adapter():
    print("--- Testing ClawAdapter ---")
    adapter = ClawAdapter()
    
    # Test 1: Simple echo task with --local
    print("Testing Test 1: Local Echo...")
    prompt = "echo 'Heiwa Intelligence Active'"
    result = adapter.run(prompt, agent_id="main", use_local=True)
    
    print(f"Result: {json.dumps(result, indent=2)}")
    
    # Check if 'reply' is in result or if it at least didn't error out hard
    if result.get("status") == "error":
        print(f"❌ Test 1 Failed: {result.get('error')}")
        return False
    
    print("✅ Test 1 Passed (or at least executed without fatal error)")
    return True

if __name__ == "__main__":
    if test_claw_adapter():
        print("--- ALL ADAPTER TESTS PASSED ---")
        sys.exit(0)
    else:
        print("--- ADAPTER TESTS FAILED ---")
        sys.exit(1)