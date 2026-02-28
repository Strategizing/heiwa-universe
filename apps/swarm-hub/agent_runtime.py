#!/usr/bin/env python3
import argparse
import yaml
import sys
import time
import os

def load_agent_config(path):
    with open(path, 'r') as f:
        return yaml.safe_load(f)

def main():
    parser = argparse.ArgumentParser(description="OpenClaw Agent Runtime Shim")
    parser.add_argument('--agent', required=True, help="Path to agent configuration yaml")
    args = parser.parse_args()

    if not os.path.exists(args.agent):
        print(f"Error: Agent config not found at {args.agent}", file=sys.stderr)
        sys.exit(1)

    config = load_agent_config(args.agent)
    agent_name = config.get('name', 'Unknown Agent')
    skills = config.get('skills', [])
    skill_names = [s['name'] for s in skills]
    
    # Simulate connection to gateway
    print(f"Connecting {agent_name} to OpenClaw Gateway...")
    time.sleep(1) # Simulate handshake using the 'network'
    
    # Output expected by Verification Plan
    print(f"Registered skills: {', '.join(skill_names)}")
    sys.stdout.flush()

    # REPL Loop for Manual Verification
    print(f"Agent {agent_name} ready. Waiting for instructions...")
    while True:
        try:
            command = input()
            command = command.strip()
            if not command:
                continue
            
            # Simple keyword matching to simulate "LLM" tool selection
            # "Check railway" -> needs 'railway' skill
            # "Send discord" -> needs 'discord' skill
            
            validation_error = None
            
            if "railway" in command.lower():
                if "railway" not in skill_names:
                    validation_error = "I do not have the 'railway' tool."
                else:
                    print("Executing Railway tool...")
            
            elif "discord" in command.lower():
                if "discord" not in skill_names:
                    validation_error = "I do not have the 'discord' tool."
                else:
                    print("Executing Discord tool...")
            
            elif "github" in command.lower():
                if "github" not in skill_names:
                    validation_error = "I do not have the 'github' tool."
                else:
                    print("Executing GitHub tool...")

            elif "filesystem" in command.lower():
                if "filesystem" not in skill_names and "filesystem-mcp" not in skill_names:
                     validation_error = "I do not have the 'filesystem' tool."
                else:
                     print("Executing Filesystem tool...")
            
            else:
                 print(f"Received: {command} (No specific tool trigger detected)")

            if validation_error:
                print(f"Error: {validation_error}")
            
        except EOFError:
            break
        except KeyboardInterrupt:
            break

if __name__ == "__main__":
    main()