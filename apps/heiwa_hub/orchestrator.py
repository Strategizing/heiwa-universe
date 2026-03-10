import libtmux
import os
import time

class SwarmOrchestrator:
    def __init__(self, session_name="heiwa_swarm"):
        self.session_name = session_name
        self.server = libtmux.Server()
        self.session = None

    def boot_swarm(self):
        """Initializes the tmux session and kills old ones (self-healing)."""
        # Kill existing session if it exists (prevents zombie processes on restart)
        try:
            self.server.kill_session(self.session_name)
        except:
            pass

        # Create new detached session
        print(f"[ORCHESTRATOR] Initializing Tmux Session: {self.session_name}")
        self.session = self.server.new_session(
            session_name=self.session_name,
            kill_session=True,
            attach=False
        )
        return True

    def spawn_agent(self, agent_name, command):
        """Creates a new window for an agent and starts its process."""
        if not self.session:
            print("[ORCHESTRATOR] Error: No active session.")
            return False

        print(f"[ORCHESTRATOR] Spawning Agent: {agent_name}")
        
        # Check if window exists, else create
        window = self.session.find_where({"window_name": agent_name})
        if not window:
            window = self.session.new_window(attach=False, window_name=agent_name)
        
        # Send the command to the pane
        pane = window.attached_pane
        pane.send_keys("export PYTHONPATH=$PYTHONPATH:$(pwd)") # Fix import paths
        pane.send_keys(command)
        
        print(f"[ORCHESTRATOR] {agent_name} is running.")
        return True

    def get_agent_logs(self, agent_name, lines=20):
        """Peeks into the agent's pane to capture stdout."""
        window = self.session.find_where({"window_name": agent_name})
        if window:
            pane = window.attached_pane
            return pane.capture_pane(start=-lines)
        return ["Agent not found."]