import unittest
import libtmux
import time
import os

from heiwa_hub.orchestrator import SwarmOrchestrator

class TestTmuxOrchestration(unittest.TestCase):
    def setUp(self):
        # Use a simplified session name for testing
        self.session_name = "test_swarm_verify"
        self.orch = SwarmOrchestrator(session_name=self.session_name)
    
    def tearDown(self):
        # Clean up
        try:
            self.orch.server.kill_session(self.session_name)
        except:
            pass

    def test_spawn_and_verify(self):
        print("\n[TEST] Booting Swarm...")
        self.orch.boot_swarm()
        
        # Spawn a dummy agent that echoes output
        agent_name = "EchoAgent"
        cmd = "echo 'Swarm Verification Successful'; sleep 5"
        
        print(f"[TEST] Spawning {agent_name}...")
        self.orch.spawn_agent(agent_name, cmd)
        
        # Wait for execution
        time.sleep(2)
        
        # Verify logs
        print("[TEST] Reading logs...")
        logs = self.orch.get_agent_logs(agent_name)
        full_log = "\n".join(logs)
        print(f"[LOG CAPTURE]\n{full_log}\n[END LOG]")
        
        self.assertIn("Swarm Verification Successful", full_log)

if __name__ == '__main__':
    unittest.main()